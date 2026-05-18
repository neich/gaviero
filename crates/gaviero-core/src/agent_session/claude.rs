//! Claude session (V9 §11 M6).
//!
//! `ClaudeSession` drives the Claude subprocess lifecycle directly,
//! extracted from `acp/client.rs::AcpPipeline::send_prompt_via_claude`.
//! Resume is driven via `ContinuityHandle::ClaudeSessionId` stored on
//! the struct, making this the first provider to own its continuity state
//! entirely within the `AgentSession` boundary.
//!
//! **Lifecycle (V9 §11 M6):** `ClaudeSession` is created once per turn by
//! `registry::create_session` and torn down when `send_turn` resolves.
//! `AcpSession` (subprocess handle) is created inside `send_turn` and torn
//! down before it returns — subprocess lifecycle is per-turn, not per-session.
//! A future milestone (M9+) may persist the handle across turns for warm-start
//! semantics; nothing in M6 blocks that.
//!
//! **Stream contract (M6):** `send_turn` returns an empty stream. All events
//! flow through the `AcpObserver` injected at construction. The stream return
//! type satisfies the `AgentSession` trait; a later milestone migrates the
//! TUI to consume the stream directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::client::{
    PROCESS_WAIT_TIMEOUT, STREAM_IDLE_TIMEOUT, format_tool_summary, is_auth_error, propose_delete,
    propose_write,
};
use crate::acp::protocol::StreamEvent;
use crate::acp::session::{AcpSession, AgentOptions};
use crate::context_planner::{ContinuityHandle, ContinuityMode, ProviderProfile};
use crate::observer::AcpObserver;
use crate::swarm::backend::AgentBackend as _;
use crate::swarm::backend::UnifiedStreamEvent;
use crate::swarm::backend::shared;
use crate::write_gate::WriteGatePipeline;

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};

// ── ClaudeSession ─────────────────────────────────────────────────────────────

/// M6 `AgentSession` implementation for Claude Code (`claude:` model prefix).
/// Owns the subprocess lifecycle for one turn and drives resume
/// via `ContinuityHandle::ClaudeSessionId` rather than the deprecated
/// `AgentOptions::resume_session_id` field.
pub struct ClaudeSession {
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Box<dyn AcpObserver>,
    /// Claude model name, stripped of `claude:` prefix.
    claude_model: String,
    workspace_root: PathBuf,
    /// Sibling workspace folders (workspace-mode multi-folder). Forwarded to
    /// the Claude CLI as repeated `--add-dir` flags so the model can
    /// read/write across folders, not just the primary cwd. Empty in
    /// single-folder mode and per-agent swarm worktrees.
    additional_roots: Vec<PathBuf>,
    agent_id: String,
    effort: String,
    max_tokens: u32,
    auto_approve: bool,
    /// Snapshotted from `AgentOptions::available_tools` at construction
    /// so per-turn spawn doesn't need to re-resolve workspace settings.
    available_tools: Option<Vec<String>>,
    approved_tools: Option<Vec<String>>,
    profile: ProviderProfile,
    /// Resume handle, initialized from `options.resume_session_id` at
    /// construction. M6: set once and used as input to `AcpSession::spawn`
    /// to pass `--resume <id>`. Updated after `send_turn` via the observer
    /// callback (`on_claude_session_started`) rather than in-struct — the
    /// TUI controller remains the authoritative updater in M6.
    handle: Option<ContinuityHandle>,
    /// Host-owned cancellation signal. When fired (e.g. by the TUI's
    /// `cancel_agent`), the streaming loop breaks out, the subprocess is
    /// killed, and the snapshot revert path runs so no half-applied tool
    /// edits remain on disk.
    cancel_token: CancellationToken,
}

impl ClaudeSession {
    /// Construct a new `ClaudeSession` from the unified `SessionConstruction`
    /// args. Called exclusively by `registry::create_session` for Claude
    /// providers (`ContinuityMode::NativeResume`).
    pub(super) fn new(args: SessionConstruction) -> Self {
        // Strip provider prefix — `claude:haiku` → `haiku`,
        // bare `sonnet` → `sonnet`.
        let claude_model = args
            .model
            .strip_prefix("claude:")
            .unwrap_or(&args.model)
            .to_string();

        // M6: resume handle sourced from the (deprecated) AgentOptions field.
        // A later milestone wires this from the ledger's `ContinuityHandle`
        // directly; for now the construction path mirrors `LegacyAgentSession`.
        #[allow(deprecated)]
        let handle = args
            .options
            .resume_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|id| ContinuityHandle::ClaudeSessionId(id.to_string()));

        let effort = args.options.effort.clone();
        let max_tokens = args.options.max_tokens;
        let auto_approve = args.options.auto_approve;
        let available_tools = args.options.available_tools.clone();
        let approved_tools = args.options.approved_tools.clone();

        Self {
            write_gate: args.write_gate,
            observer: args.observer,
            claude_model,
            workspace_root: args.workspace_root,
            additional_roots: args.additional_roots,
            agent_id: args.agent_id,
            effort,
            max_tokens,
            auto_approve,
            available_tools,
            approved_tools,
            profile: args.profile,
            handle,
            cancel_token: args.cancel_token,
        }
    }

    /// Reconstruct the enriched prompt and all legacy inputs from the
    /// structured `Turn`, then run the Claude subprocess loop. This is the
    /// core of the Claude streaming path.
    async fn run_claude_turn(&self, turn: &Turn, resume_session_id: Option<String>) -> Result<()> {
        // ── Reconstruct legacy inputs from Turn ──────────────────────────

        // Enriched prompt: user message FIRST (wrapped in <user_message>),
        // then graph block, then memory block. Placing the user's request at
        // the top (rather than the legacy graph → memory → user-msg order)
        // keeps it inside Claude's default 2000-line Read window when this
        // blob is later spilled to `.gaviero/tmp/prompt-*.md` on
        // bootstrap-heavy first turns. The XML tag gives the agent an
        // unambiguous boundary between the user's request and the injected
        // context that follows.
        // Split FileAttachment into (path, content) text refs vs bare-path
        // image/document attachments. Bare paths used to ride on Claude's
        // `--file` flag, but that flag is for downloading remote file
        // resources (format `file_id:relative_path`); using it for local
        // paths errors out with "Session token required for file downloads".
        // Instead, we mention them inside the user message so Claude calls
        // its `Read` tool (which handles PNG/JPG/GIF/WEBP/SVG natively) and
        // we widen `--add-dir` to include each parent so the tool is
        // allowed to access them.
        let mut file_refs: Vec<(String, String)> = Vec::new();
        let mut file_attachments: Vec<PathBuf> = Vec::new();
        for f in &turn.file_refs {
            match &f.content {
                Some(text) => file_refs.push((f.path.to_string_lossy().into_owned(), text.clone())),
                None => file_attachments.push(f.path.clone()),
            }
        }

        let user_message_with_attachments =
            embed_attachment_refs(&turn.user_message, &file_attachments);

        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "<user_message>\n{}\n</user_message>",
            user_message_with_attachments
        ));
        if let Some(block) = shared::render_graph_block(&turn.graph_selections) {
            parts.push(block);
        }
        if let Some(block) = shared::render_memory_block(&turn.memory_selections) {
            parts.push(block);
        }
        let enriched_prompt = parts.join("\n\n");

        // Conversation history from replay payload (StatelessReplay only;
        // NativeResume sends empty history because Claude keeps it server-side).
        // In M6 Claude always uses NativeResume, so this is always empty.
        // Kept here so the code is correct if ContinuityMode changes later.
        let conversation_history: Vec<(String, String)> = turn
            .replay_history
            .as_ref()
            .map(|p| {
                p.entries
                    .iter()
                    .map(|(r, c)| (super::role_to_string(*r), c.clone()))
                    .collect()
            })
            .unwrap_or_default();

        // ── M0 instrumentation ────────────────────────────────────────────
        let resume_hint = resume_session_id
            .as_deref()
            .map(|id| !id.is_empty())
            .unwrap_or(false);
        let history_chars: usize = conversation_history
            .iter()
            .map(|(r, c)| r.len() + c.len())
            .sum();
        tracing::info!(
            target: "turn_metrics",
            kind = "chat",
            provider = "claude",
            model = %self.claude_model,
            prompt_chars = enriched_prompt.len(),
            replay_chars = history_chars,
            file_refs_count = file_refs.len(),
            file_attachments_count = file_attachments.len(),
            resume_hint,
            "turn_dispatch"
        );

        // ── Build AgentOptions for AcpSession ────────────────────────────
        let options = {
            #[allow(deprecated)]
            AgentOptions {
                effort: self.effort.clone(),
                max_tokens: self.max_tokens,
                auto_approve: self.auto_approve,
                available_tools: self.available_tools.clone(),
                approved_tools: self.approved_tools.clone(),
                resume_session_id,
                ..AgentOptions::default()
            }
        };

        // ── Tool permission configuration ─────────────────────────────────
        let (available_owned, approved_owned) = options.resolved_tools();
        let available_tools: Vec<&str> = available_owned.iter().map(String::as_str).collect();
        let approved_tools: Vec<&str> = approved_owned.iter().map(String::as_str).collect();

        // System prompt (same as legacy path — from ClaudeCodeBackend capabilities).
        let system_prompt = shared::default_editor_system_prompt(
            &crate::swarm::backend::claude_code::ClaudeCodeBackend::new(&self.claude_model)
                .capabilities(),
        );

        // Build `build_enriched_prompt`-equivalent for file refs + history.
        // `AcpSession::spawn` takes the enriched prompt directly; we pass the
        // same value that `send_prompt_via_claude` produced via
        // `shared::build_enriched_prompt`.
        let final_prompt =
            shared::build_enriched_prompt(&enriched_prompt, &conversation_history, &file_refs);

        // Widen `--add-dir` with each unique parent so Claude's Read tool
        // can reach attachments that live outside the workspace (e.g.
        // clipboard images saved under `$XDG_CACHE_HOME/gaviero/attachments`
        // or screenshots in `/tmp`). The workspace root is already added
        // implicitly by `AcpSession::spawn`, so we skip it here.
        let extra_attachment_roots: Vec<PathBuf> = collect_attachment_parents(
            &file_attachments,
            &self.workspace_root,
            &self.additional_roots,
        );
        let attach_refs: Vec<&Path> = file_attachments.iter().map(|p| p.as_path()).collect();
        let additional_root_refs: Vec<&Path> = self
            .additional_roots
            .iter()
            .chain(extra_attachment_roots.iter())
            .map(|p| p.as_path())
            .collect();

        let mut session = AcpSession::spawn(
            &self.claude_model,
            &self.workspace_root,
            &final_prompt,
            &system_prompt,
            &available_tools,
            &approved_tools,
            &options,
            &attach_refs,
            &additional_root_refs,
        )?;

        // ── Streaming loop ────────────────────────────────────────────────
        //
        // Claude file edits flow through native Write/Edit/MultiEdit tool
        // calls (snapshot+revert below). The in-band <file ...> parser is
        // intentionally NOT invoked on this path — instructing the model
        // about it produced false-positive proposals when prose quoted the
        // marker. The text stream is now treated as opaque assistant prose.
        let mut full_text = String::new();
        let mut in_thinking = false;
        // `None` means the file did not exist at snapshot time → revert via
        // unlink. `Some(s)` means the file existed with content `s` → revert
        // by overwriting with `s`. Distinguishing these two cases is what
        // prevents a rejected new-file proposal from leaving an empty stub
        // on disk.
        let mut file_snapshots: HashMap<PathBuf, Option<String>> = HashMap::new();
        let mut read_count: usize = 0;
        let mut idle_count: u32 = 0;
        let mut cancelled = false;

        loop {
            // Cancellation is checked first (`biased`) so a token fired while
            // a tool call is mid-flight wins immediately over any newly
            // arrived stream event. The subprocess is killed synchronously
            // below — `kill_on_drop` is only the safety net for surprise
            // task aborts.
            let next = tokio::select! {
                biased;
                _ = self.cancel_token.cancelled() => {
                    cancelled = true;
                    break;
                }
                n = tokio::time::timeout(STREAM_IDLE_TIMEOUT, session.next_event()) => n,
            };

            match next {
                Err(_elapsed) => {
                    idle_count += 1;
                    let elapsed_secs = idle_count * STREAM_IDLE_TIMEOUT.as_secs() as u32;
                    if session.try_wait_exited() {
                        tracing::warn!("Claude subprocess exited during idle wait");
                        let stderr = session.stderr_output().await;
                        let msg = if stderr.is_empty() {
                            "Claude process exited unexpectedly during tool execution.\n\
                             Check ~/.cache/gaviero/gaviero.log for details."
                                .to_string()
                        } else {
                            format!("Claude CLI error during tool execution:\n{}", stderr)
                        };
                        self.observer.on_message_complete("system", &msg);
                        break;
                    }
                    self.observer.on_streaming_status(&format!(
                        "Working... ({}s elapsed, tools running)",
                        elapsed_secs
                    ));
                    tracing::debug!(
                        "Stream idle for {}s, subprocess still alive — sending keepalive",
                        elapsed_secs
                    );
                    continue;
                }
                Ok(result) => {
                    idle_count = 0;
                    match result {
                        Ok(Some(event)) => match event {
                            StreamEvent::ThinkingDelta(text) => {
                                if !in_thinking {
                                    self.observer.on_stream_chunk("<think>\n");
                                    in_thinking = true;
                                }
                                self.observer.on_stream_chunk(&text);
                            }
                            StreamEvent::ContentDelta(text) => {
                                if in_thinking {
                                    self.observer.on_stream_chunk("\n</think>\n");
                                    in_thinking = false;
                                }
                                full_text.push_str(&text);
                                self.observer.on_stream_chunk(&text);
                            }
                            StreamEvent::ToolUseStart { tool_name, .. } => {
                                if tool_name == "Read" {
                                    read_count += 1;
                                }
                                self.observer
                                    .on_streaming_status(&format!("Using {}...", tool_name));
                            }
                            StreamEvent::ToolInputDelta(_) => {}
                            StreamEvent::AssistantMessage { text, tool_uses } => {
                                if full_text.is_empty() && !text.is_empty() {
                                    full_text = text;
                                }
                                for tu in &tool_uses {
                                    let summary = format_tool_summary(
                                        &tu.name,
                                        &tu.input,
                                        &self.workspace_root,
                                    );
                                    self.observer.on_tool_call_started(&summary);
                                }
                                for tu in &tool_uses {
                                    if matches!(tu.name.as_str(), "Write" | "Edit" | "MultiEdit") {
                                        if let Some(fp) =
                                            tu.input.get("file_path").and_then(|v| v.as_str())
                                        {
                                            let abs_path = if Path::new(fp).is_absolute() {
                                                PathBuf::from(fp)
                                            } else {
                                                self.workspace_root.join(fp)
                                            };
                                            if !file_snapshots.contains_key(&abs_path) {
                                                let content = match tokio::fs::read_to_string(
                                                    &abs_path,
                                                )
                                                .await
                                                {
                                                    Ok(s) => Some(s),
                                                    Err(e)
                                                        if e.kind()
                                                            == std::io::ErrorKind::NotFound =>
                                                    {
                                                        None
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "Snapshot read of {} failed ({}); treating as did-not-exist",
                                                            abs_path.display(),
                                                            e
                                                        );
                                                        None
                                                    }
                                                };
                                                tracing::info!(
                                                    "Snapshot before tool {}: {} ({})",
                                                    tu.name,
                                                    abs_path.display(),
                                                    match &content {
                                                        Some(s) => format!("{} bytes", s.len()),
                                                        None => "did not exist".to_string(),
                                                    }
                                                );
                                                file_snapshots.insert(abs_path, content);
                                            }
                                        }
                                    }
                                }
                            }
                            StreamEvent::ResultEvent {
                                is_error,
                                result_text,
                                ..
                            } => {
                                if is_error {
                                    let msg = if is_auth_error(&result_text) {
                                        format!(
                                            "Error: {}\n\nTo re-authenticate, run `claude login` in a terminal, then retry.",
                                            result_text
                                        )
                                    } else {
                                        format!("Error: {}", result_text)
                                    };
                                    self.observer.on_message_complete("system", &msg);
                                } else {
                                    if full_text.is_empty() && !result_text.is_empty() {
                                        full_text = result_text.clone();
                                    }
                                    self.observer.on_message_complete("assistant", &full_text);
                                }
                                break;
                            }
                            StreamEvent::PermissionRequest {
                                tool_name,
                                description,
                                request_id,
                            } => {
                                let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                                self.observer
                                    .on_permission_request(&tool_name, &description, tx);
                                let allow = rx.await.unwrap_or(false);
                                tracing::info!(
                                    "Permission request for '{}': {}",
                                    tool_name,
                                    if allow { "allowed" } else { "denied" }
                                );
                                session.respond_permission(allow, &request_id);
                                idle_count = 0;
                            }
                            StreamEvent::SystemInit { session_id, .. } => {
                                #[allow(deprecated)]
                                let asked = options.resume_session_id.as_deref();
                                let resume_accepted = match asked {
                                    Some(asked_id)
                                        if !asked_id.is_empty() && !session_id.is_empty() =>
                                    {
                                        asked_id == session_id
                                    }
                                    _ => false,
                                };
                                tracing::info!(
                                    target: "turn_metrics",
                                    provider = "claude",
                                    session_id = %session_id,
                                    resume_accepted,
                                    "session_init"
                                );
                                if !session_id.is_empty() {
                                    self.observer.on_claude_session_started(&session_id);
                                }
                            }
                            StreamEvent::Unknown(_) => {}
                        },
                        Ok(None) => {
                            if !full_text.is_empty() {
                                self.observer.on_message_complete("assistant", &full_text);
                            } else {
                                let exit_status = match tokio::time::timeout(
                                    PROCESS_WAIT_TIMEOUT,
                                    session.wait(),
                                )
                                .await
                                {
                                    Ok(status) => status.ok(),
                                    Err(_) => {
                                        tracing::warn!("Process wait timed out on EOF, killing");
                                        session.kill();
                                        None
                                    }
                                };
                                tokio::task::yield_now().await;
                                let stderr = session.stderr_output().await;
                                let exit_info = match exit_status {
                                    Some(s) => format!(" (exit code: {})", s),
                                    None => String::new(),
                                };
                                let msg = if stderr.is_empty() {
                                    format!(
                                        "Claude process exited without output{}\n\
                                         Check ~/.cache/gaviero/gaviero.log for details.",
                                        exit_info
                                    )
                                } else {
                                    let base =
                                        format!("Claude CLI error{}:\n{}", exit_info, stderr);
                                    if is_auth_error(&stderr) {
                                        format!(
                                            "{}\n\nTo re-authenticate, run `claude login` in a terminal, then retry.",
                                            base
                                        )
                                    } else {
                                        base
                                    }
                                };
                                self.observer.on_message_complete("system", &msg);
                            }
                            break;
                        }
                        Err(e) => {
                            self.observer
                                .on_message_complete("system", &format!("Stream error: {}", e));
                            break;
                        }
                    }
                }
            }
        }

        if in_thinking {
            self.observer.on_stream_chunk("\n</think>\n");
        }

        if cancelled {
            // Transactional cancel: kill the subprocess immediately so no
            // further tool calls fire, then fall through to the snapshot
            // revert path so any tool edits already on disk are rolled back
            // (and surfaced as proposals — never auto-applied).
            tracing::info!("Claude session cancelled by host — killing subprocess");
            session.kill();
            self.observer
                .on_streaming_status("Cancelling — reverting in-flight edits...");
            // Reap the subprocess so we don't leave a zombie. SIGKILL on the
            // child means this should be near-instant; bound it anyway.
            let _ = tokio::time::timeout(PROCESS_WAIT_TIMEOUT, session.wait()).await;
        } else {
            // Wait for subprocess to finish.
            self.observer.on_streaming_status("Finalizing...");
            match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, session.wait()).await {
                Ok(Ok(status)) => tracing::info!("Claude subprocess exited: {}", status),
                Ok(Err(e)) => tracing::warn!("Error waiting for claude subprocess: {}", e),
                Err(_) => {
                    tracing::warn!(
                        "Claude subprocess did not exit within {}s, killing",
                        PROCESS_WAIT_TIMEOUT.as_secs()
                    );
                    session.kill();
                }
            }
        }

        tracing::info!(
            target: "turn_metrics",
            provider = "claude",
            read_count,
            "turn_read_count"
        );

        // Create proposals from snapshotted files.
        if !file_snapshots.is_empty() {
            let total = file_snapshots.len();
            tracing::info!(
                "Processing {} file snapshots for tool-based proposals",
                total
            );
            self.observer.on_streaming_status(&format!(
                "Processing {} file change{}...",
                total,
                if total == 1 { "" } else { "s" }
            ));
            let snapshots: Vec<(PathBuf, Option<String>)> = file_snapshots.into_iter().collect();
            for (i, (abs_path, original)) in snapshots.iter().enumerate() {
                // Read current on-disk state. ENOENT here means the tool
                // ultimately did not produce a file at this path (e.g. a
                // failed Write). Treat as "no change" against a missing
                // baseline.
                let current = match tokio::fs::read_to_string(abs_path).await {
                    Ok(s) => Some(s),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        tracing::warn!(
                            "Post-tool read of {} failed ({}); skipping",
                            abs_path.display(),
                            e
                        );
                        continue;
                    }
                };
                if current.as_deref() == original.as_deref() {
                    tracing::info!("Snapshot unchanged: {}", abs_path.display());
                    continue;
                }
                tracing::info!(
                    "File changed by tool: {} (orig={}, new={})",
                    abs_path.display(),
                    match original {
                        Some(s) => format!("{} bytes", s.len()),
                        None => "did not exist".to_string(),
                    },
                    match &current {
                        Some(s) => format!("{} bytes", s.len()),
                        None => "did not exist".to_string(),
                    },
                );
                if total > 1 {
                    self.observer.on_streaming_status(&format!(
                        "Processing file {}/{}...",
                        i + 1,
                        total
                    ));
                }
                // Drift check: between the post-tool read above and this
                // revert, another writer (a peer agent, the user, or a
                // background task) may have changed the file. If we revert
                // blindly we'd clobber that write. Re-read and bail out if
                // the on-disk content no longer matches `current`.
                let on_disk_now = match tokio::fs::read_to_string(abs_path).await {
                    Ok(s) => Some(s),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        tracing::warn!(
                            "Drift-check read of {} failed ({}); skipping revert",
                            abs_path.display(),
                            e
                        );
                        continue;
                    }
                };
                if on_disk_now.as_deref() != current.as_deref() {
                    let msg = format!(
                        "⚠ Disk drifted on {} between tool completion and revert — skipping revert and proposal to avoid clobbering a concurrent write.",
                        abs_path.display()
                    );
                    tracing::warn!("{}", msg);
                    self.observer.on_message_complete("system", &msg);
                    continue;
                }
                // Revert to the snapshotted state. The proposal carries the
                // tool's intended change; review will re-apply it on accept.
                //
                // Critical: if the file did NOT exist at snapshot time, we
                // must UNLINK it — not write an empty string. Otherwise
                // rejecting a new-file proposal leaves a zero-byte stub on
                // disk (the bug that produced the `x` file in the audit).
                let revert_result = match original {
                    Some(orig) => tokio::fs::write(abs_path, orig).await,
                    None => match tokio::fs::remove_file(abs_path).await {
                        Ok(()) => Ok(()),
                        // Already gone (tool removed it itself, or a peer
                        // task cleaned up). Idempotent revert.
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e),
                    },
                };
                if let Err(e) = revert_result {
                    tracing::error!("Failed to revert {}: {}", abs_path.display(), e);
                    continue;
                }
                let rel_path = abs_path
                    .strip_prefix(&self.workspace_root)
                    .unwrap_or(abs_path.as_path());
                // Two paths: write/edit (`current=Some`) goes through
                // `propose_write`; delete (`current=None`, `original=Some`)
                // goes through `propose_delete`. `(None, None)` means the
                // file never existed — nothing to propose.
                let proposal_result = match (current.as_deref(), original.as_deref()) {
                    (Some(new_content), _) => {
                        propose_write(
                            &self.write_gate,
                            self.observer.as_ref(),
                            &self.workspace_root,
                            &self.agent_id,
                            rel_path,
                            new_content,
                        )
                        .await
                    }
                    (None, Some(orig)) => {
                        propose_delete(
                            &self.write_gate,
                            self.observer.as_ref(),
                            &self.workspace_root,
                            &self.agent_id,
                            rel_path,
                            orig,
                        )
                        .await
                    }
                    (None, None) => Ok(()),
                };
                if let Err(e) = proposal_result {
                    tracing::error!(
                        "Failed to create proposal for {}: {}",
                        abs_path.display(),
                        e
                    );
                }
            }
        }

        if cancelled {
            // Surface a terminal message so the chat finalises cleanly. The
            // observer side has already torn down the streaming task; this
            // is what gets rendered as the conversation's last entry.
            self.observer
                .on_message_complete("system", "Cancelled by user.");
        }

        Ok(())
    }
}

/// Append an `<attached_files>` block to the user message listing the
/// absolute paths of any bare-path file attachments, plus a one-line
/// instruction so the model invokes its `Read` tool to view them. Returns
/// the original message unchanged when `attachments` is empty.
fn embed_attachment_refs(user_message: &str, attachments: &[PathBuf]) -> String {
    if attachments.is_empty() {
        return user_message.to_string();
    }
    let paths = attachments
        .iter()
        .map(|p| format!("- {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{user_message}\n\n<attached_files>\n{paths}\n</attached_files>\nUse the Read tool to view the attached file(s) above."
    )
}

/// Return the unique parent directories of `attachments`, skipping any that
/// equal `workspace_root` (already added by `AcpSession::spawn`) or are
/// already present in `additional_roots`.
fn collect_attachment_parents(
    attachments: &[PathBuf],
    workspace_root: &Path,
    additional_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in attachments {
        let Some(parent) = path.parent() else {
            continue;
        };
        if parent.as_os_str().is_empty() || parent == workspace_root {
            continue;
        }
        if additional_roots.iter().any(|p| p == parent) {
            continue;
        }
        if out.iter().any(|p| p == parent) {
            continue;
        }
        out.push(parent.to_path_buf());
    }
    out
}

// ── AgentSession impl ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl AgentSession for ClaudeSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        // Extract resume id from the continuity handle (M6: drives resume via
        // `ContinuityHandle::ClaudeSessionId` rather than the legacy field).
        let resume_session_id = match &self.handle {
            Some(ContinuityHandle::ClaudeSessionId(id)) if !id.is_empty() => Some(id.clone()),
            _ => None,
        };

        self.run_claude_turn(&turn, resume_session_id).await?;

        // M6: return empty stream — events flow through the observer.
        // A later milestone migrates the TUI to consume the stream directly.
        Ok(Box::pin(futures::stream::empty()))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.profile.continuity_mode
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        self.handle.as_ref()
    }

    async fn close(self: Box<Self>) {
        // Subprocess is spawned per send_turn and torn down when that future
        // resolves — nothing to release here in M6.
    }
}

#[cfg(test)]
mod tests {
    use super::{collect_attachment_parents, embed_attachment_refs};
    use std::path::{Path, PathBuf};

    #[test]
    fn embed_attachment_refs_returns_original_when_no_attachments() {
        let msg = "look at this";
        assert_eq!(embed_attachment_refs(msg, &[]), msg);
    }

    #[test]
    fn embed_attachment_refs_appends_paths_and_read_tool_hint() {
        let msg = "what is in the screenshots?";
        let attachments = vec![
            PathBuf::from("/tmp/a.png"),
            PathBuf::from("/var/foo/b.jpg"),
        ];
        let out = embed_attachment_refs(msg, &attachments);
        assert!(out.contains("what is in the screenshots?"));
        assert!(out.contains("<attached_files>"));
        assert!(out.contains("- /tmp/a.png"));
        assert!(out.contains("- /var/foo/b.jpg"));
        assert!(out.contains("</attached_files>"));
        assert!(out.contains("Use the Read tool"));
    }

    #[test]
    fn collect_attachment_parents_skips_workspace_root_and_dedupes() {
        let workspace = PathBuf::from("/work/proj");
        let additional = vec![PathBuf::from("/work/lib")];
        let attachments = vec![
            PathBuf::from("/work/proj/notes.md"), // under workspace → skip
            PathBuf::from("/tmp/a.png"),
            PathBuf::from("/tmp/b.png"),          // dedup
            PathBuf::from("/work/lib/c.png"),     // already in additional → skip
            PathBuf::from("/home/me/d.png"),
        ];
        let parents = collect_attachment_parents(&attachments, &workspace, &additional);
        let parents: Vec<&Path> = parents.iter().map(|p| p.as_path()).collect();
        assert_eq!(
            parents,
            vec![Path::new("/tmp"), Path::new("/home/me")]
        );
    }

    #[test]
    fn collect_attachment_parents_ignores_pathless_entries() {
        let workspace = PathBuf::from("/work");
        let attachments = vec![PathBuf::from("just-a-filename")];
        let parents = collect_attachment_parents(&attachments, &workspace, &[]);
        // `Path::parent` returns Some("") for a bare filename → skipped.
        assert!(parents.is_empty());
    }
}
