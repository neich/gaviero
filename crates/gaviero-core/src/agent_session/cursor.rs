//! Cursor CLI chat session.
//!
//! Drives the Cursor `agent -p` subprocess for the chat path and surfaces
//! events through the [`AcpObserver`]. Mirrors `ClaudeSession`'s
//! snapshot+revert workflow: every file the CLI's native tool calls touch
//! is snapshotted at the `editToolCall.started` / `writeToolCall.started`
//! event, reverted after the stream ends, and surfaced as a Write Gate
//! proposal carrying the agent's intended content.
//!
//! **Why snapshot+revert (not "propose-only" mode):** the Cursor CLI's
//! `-p`/`--print` headless mode writes to disk by default. `-f`/`--force`
//! is about auto-approving Bash commands, not about deferring write
//! proposals. Probe-confirmed against `agent 2026.05.16-…`.
//!
//! **Continuity:** `NativeResume`. The `system.init` event carries a
//! `session_id` (Cursor's chat / thread id) that we surface via
//! [`AcpObserver::on_cursor_session_started`]. The TUI persists it on the
//! conversation's [`crate::context_planner::SessionLedger`] as a
//! [`ContinuityHandle::CursorThreadId`]; on the next turn the chat path
//! reads the handle back and passes the id to `agent --resume <id>` so
//! Cursor retains model context server-side.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::Stream;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::client::{propose_delete, propose_write};
use crate::context_planner::{ContinuityHandle, ContinuityMode, ProviderProfile};
use crate::observer::AcpObserver;
use crate::swarm::backend::cursor::{
    CURSOR_ARGV_LIMIT, CursorEvent, cursor_argv, is_auth_error, parse_cursor_event,
    tool_display_name, write_tool_args,
};
use crate::swarm::backend::shared;
use crate::swarm::backend::{Capabilities, UnifiedStreamEvent};
use crate::write_gate::WriteGatePipeline;

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};

/// Idle wait between NDJSON lines before we send a keepalive status — long
/// `bashToolCall` runs can stall the stream for many seconds; matches
/// `acp::client::STREAM_IDLE_TIMEOUT`.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound the post-stream wait so a wedged subprocess can't hang the chat
/// loop forever. Matches `acp::client::PROCESS_WAIT_TIMEOUT`.
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cursor CLI chat session — phase 1 of the V9 provider parity work.
pub struct CursorSession {
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Box<dyn AcpObserver>,
    /// Cursor model name (`cursor:` prefix already stripped).
    cursor_model: String,
    workspace_root: PathBuf,
    /// Sibling workspace folders (workspace-mode multi-folder). The Cursor
    /// CLI exposes no documented multi-root flag, so the chat path embeds
    /// these in the user message as a `<workspace_folders>` hint — same
    /// fallback `codex_app_server` uses.
    additional_roots: Vec<PathBuf>,
    agent_id: String,
    conv_id: Option<String>,
    profile: ProviderProfile,
    /// Resume handle, seeded at construction from
    /// [`crate::acp::session::AgentOptions::resume_session_id`]. On each
    /// turn it's read back to add `--resume <id>` to the argv so Cursor
    /// resumes the prior chat. The `system.init` event arrives with the
    /// authoritative id (which may differ from the requested one if the
    /// prior session expired); we update the handle in place and surface
    /// it through [`AcpObserver::on_cursor_session_started`] so the TUI
    /// can persist it on the ledger.
    handle: Option<ContinuityHandle>,
    cancel_token: CancellationToken,
}

impl CursorSession {
    /// Construct a new `CursorSession`. Called exclusively by
    /// `registry::create_session` for Cursor providers
    /// (`ContinuityMode::NativeResume && profile.provider == "cursor"`).
    pub(super) fn new(args: SessionConstruction) -> Self {
        let cursor_model = args
            .model
            .strip_prefix("cursor:")
            .unwrap_or(&args.model)
            .to_string();

        // The TUI feeds the prior chat id back via the (deprecated)
        // `resume_session_id` field on `AgentOptions`. Allow stays until
        // M10 retires the field across all provider sessions.
        #[allow(deprecated)]
        let handle = args
            .options
            .resume_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|id| ContinuityHandle::CursorThreadId(id.to_string()));

        Self {
            write_gate: args.write_gate,
            observer: args.observer,
            cursor_model,
            workspace_root: args.workspace_root,
            additional_roots: args.additional_roots,
            agent_id: args.agent_id,
            conv_id: args.conv_id,
            profile: args.profile,
            handle,
            cancel_token: args.cancel_token,
        }
    }

    async fn run_cursor_turn(&mut self, turn: &Turn) -> Result<()> {
        // ── Reconstruct the enriched prompt ─────────────────────────────
        let user_message =
            embed_workspace_folders(&turn.user_message, &self.workspace_root, &self.additional_roots);

        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("<user_message>\n{}\n</user_message>", user_message));
        if let Some(block) = shared::render_graph_block(&turn.graph_selections) {
            parts.push(block);
        }
        if let Some(block) = shared::render_memory_block(&turn.memory_selections) {
            parts.push(block);
        }
        if let Some(block) = shared::render_skill_block(&turn.skill_selections) {
            parts.push(block);
        }
        let enriched_prompt = parts.join("\n\n");

        // Replay history: phase 1 is StatelessReplay so the planner gives
        // us a payload to embed in the prompt. NativeResume promotion will
        // strip this in favour of `--resume <id>` continuity.
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

        // File refs: keep only ones with content. Cursor's `Read` tool
        // handles binary attachments natively when paths are mentioned in
        // prose, but the chat path passes inline text content the same way
        // Codex does to avoid burning extra Read tool calls.
        let file_refs: Vec<(String, String)> = turn
            .file_refs
            .iter()
            .filter_map(|f| {
                f.content
                    .as_ref()
                    .map(|c| (f.path.to_string_lossy().into_owned(), c.clone()))
            })
            .collect();

        // System prompt: same vocabulary as the swarm CursorBackend so the
        // cached prefix stays consistent across paths.
        let backend_caps = Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
            extended_thinking: true,
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            supports_file_blocks: false,
        };
        let system_prompt = shared::default_editor_system_prompt(&backend_caps);
        let user_prompt =
            shared::build_enriched_prompt(&enriched_prompt, &conversation_history, &file_refs);
        let combined_prompt = format!("{system_prompt}\n\n{user_prompt}");

        if combined_prompt.len() >= CURSOR_ARGV_LIMIT {
            anyhow::bail!(
                "cursor prompt is {} bytes which exceeds the {}-byte argv limit. \
                 The `agent` CLI has no stdin or `--prompt-file` fallback. \
                 Most of a first-turn prompt is bootstrap context: run `/lite` \
                 to drop <repo_outline> + memory + impact (keeps topology) for \
                 the next turn, then send. You can also trim the user message / \
                 attachments, or switch to a provider with stdin support \
                 (claude, codex, ollama).",
                combined_prompt.len(),
                CURSOR_ARGV_LIMIT,
            );
        }

        // ── M0 instrumentation ──────────────────────────────────────────
        let history_chars: usize = conversation_history
            .iter()
            .map(|(r, c)| r.len() + c.len())
            .sum();
        tracing::info!(
            target: "turn_metrics",
            kind = "chat",
            provider = "cursor",
            model = %self.cursor_model,
            prompt_chars = combined_prompt.len(),
            replay_chars = history_chars,
            file_refs_count = file_refs.len(),
            "turn_dispatch"
        );

        // ── Spawn subprocess ────────────────────────────────────────────
        //
        // Resume id: extracted from the typed handle the TUI persisted on
        // the prior turn (`ContinuityHandle::CursorThreadId`). The first
        // turn of a new conversation passes `None`; the `system.init`
        // event below installs the freshly minted id so subsequent turns
        // round-trip server-side context via `--resume <id>`.
        let resume_id: Option<String> = match &self.handle {
            Some(ContinuityHandle::CursorThreadId(id)) if !id.is_empty() => Some(id.clone()),
            _ => None,
        };

        let mut cmd = Command::new("agent");
        for arg in cursor_argv(
            &self.cursor_model,
            &self.workspace_root,
            resume_id.as_deref(),
        ) {
            cmd.arg(arg);
        }
        cmd.arg(&combined_prompt)
            .current_dir(&self.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "spawning cursor `agent` subprocess: {e}\n\
                     The `agent` CLI binary was not found on PATH. \
                     Install it from https://cursor.com/cli (curl https://cursor.com/install -fsS | bash)."
                )
            } else {
                anyhow::anyhow!("spawning cursor `agent` subprocess: {e}")
            }
        })?;

        let stdout = child
            .stdout
            .take()
            .context("cursor `agent` stdout unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("cursor `agent` stderr unavailable")?;

        // Drain stderr in the background so the OS pipe buffer can't fill
        // and block the subprocess mid-tool-call.
        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        // ── Streaming loop ──────────────────────────────────────────────
        let mut lines = BufReader::new(stdout).lines();
        let mut full_text = String::new();
        let mut last_segment_text = String::new();
        let mut in_thinking = false;
        let mut snapshots: HashMap<PathBuf, Option<String>> = HashMap::new();
        let mut pending_writes: HashMap<String, (PathBuf, String)> = HashMap::new();
        let mut error_msg: Option<String> = None;
        let mut idle_count: u32 = 0;
        let mut cancelled = false;
        let mut session_id_captured: Option<String> = None;

        loop {
            let next = tokio::select! {
                biased;
                _ = self.cancel_token.cancelled() => {
                    cancelled = true;
                    break;
                }
                line = tokio::time::timeout(STREAM_IDLE_TIMEOUT, lines.next_line()) => line,
            };

            match next {
                Err(_elapsed) => {
                    idle_count += 1;
                    let elapsed_secs = idle_count * STREAM_IDLE_TIMEOUT.as_secs() as u32;
                    if let Ok(Some(_)) = child.try_wait() {
                        tracing::warn!(
                            "cursor subprocess exited during idle wait at {}s",
                            elapsed_secs
                        );
                        break;
                    }
                    self.observer.on_streaming_status(&format!(
                        "Working... ({}s elapsed, tools running)",
                        elapsed_secs
                    ));
                    continue;
                }
                Ok(Ok(Some(line))) => {
                    idle_count = 0;
                    let Some(event) = parse_cursor_event(&line) else {
                        continue;
                    };
                    if self
                        .dispatch_event(
                            event,
                            &mut full_text,
                            &mut last_segment_text,
                            &mut in_thinking,
                            &mut snapshots,
                            &mut pending_writes,
                            &mut error_msg,
                            &mut session_id_captured,
                        )
                        .await
                    {
                        break;
                    }
                }
                Ok(Ok(None)) => {
                    // EOF without a `result` event — surface stderr if any.
                    break;
                }
                Ok(Err(e)) => {
                    error_msg = Some(format!("cursor stream read error: {e}"));
                    break;
                }
            }
        }

        if in_thinking {
            self.observer.on_stream_chunk("\n</think>\n");
        }

        // ── Cleanup / cancel handling ──────────────────────────────────
        if cancelled {
            tracing::info!("cursor session cancelled by host — killing subprocess");
            let _ = child.start_kill();
            self.observer
                .on_streaming_status("Cancelling — reverting in-flight edits...");
            let _ = tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await;
        } else {
            // Wait for the subprocess to exit so we can surface its stderr
            // before returning. Bound to avoid wedging the chat loop.
            self.observer.on_streaming_status("Finalizing...");
            match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
                Ok(Ok(status)) => tracing::info!("cursor subprocess exited: {}", status),
                Ok(Err(e)) => tracing::warn!("waiting for cursor subprocess: {}", e),
                Err(_) => {
                    tracing::warn!(
                        "cursor subprocess did not exit within {}s, killing",
                        PROCESS_WAIT_TIMEOUT.as_secs()
                    );
                    let _ = child.start_kill();
                }
            }
        }

        let stderr_text = stderr_handle.await.unwrap_or_default();

        // ── Surface a final assistant / system message ─────────────────
        if full_text.is_empty() && !last_segment_text.is_empty() {
            // No deltas were captured but the consolidated `assistant`
            // event(s) carried text — fall back to that so the user sees
            // the model's reply even if `--stream-partial-output` got
            // disabled by a future CLI flag change.
            full_text = last_segment_text.clone();
        }

        // ── Snapshot revert + Write Gate proposals ──────────────────────
        //
        // Walk every file we snapshotted at tool-call-start. For each:
        //   1. Re-read disk; this is what the CLI's tool ultimately wrote.
        //   2. Compare against snapshot. If unchanged, skip.
        //   3. Drift-check: read disk again to make sure nothing else
        //      raced in. If the second read differs, bail with a system
        //      message rather than clobbering a concurrent writer.
        //   4. Revert (write snapshot bytes back, or unlink for new files).
        //   5. Propose the new content (or deletion) through the Write Gate.
        //
        // Mirrors the post-stream loop in `agent_session::claude::run_claude_turn`.
        if !snapshots.is_empty() {
            let total = snapshots.len();
            self.observer.on_streaming_status(&format!(
                "Processing {} file change{}...",
                total,
                if total == 1 { "" } else { "s" }
            ));
            let entries: Vec<(PathBuf, Option<String>)> = snapshots.into_iter().collect();
            for (i, (abs_path, original)) in entries.iter().enumerate() {
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
                if total > 1 {
                    self.observer.on_streaming_status(&format!(
                        "Processing file {}/{}...",
                        i + 1,
                        total
                    ));
                }
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
                        "⚠ Disk drifted on {} between tool completion and revert — \
                         skipping revert and proposal to avoid clobbering a concurrent write.",
                        abs_path.display()
                    );
                    tracing::warn!("{}", msg);
                    self.observer.on_message_complete("system", &msg);
                    continue;
                }
                // Revert.
                let revert_result = match original {
                    Some(orig) => tokio::fs::write(abs_path, orig).await,
                    None => match tokio::fs::remove_file(abs_path).await {
                        Ok(()) => Ok(()),
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
                let proposal_result = match (current.as_deref(), original.as_deref()) {
                    (Some(new_content), _) => {
                        propose_write(
                            &self.write_gate,
                            self.observer.as_ref(),
                            &self.workspace_root,
                            &self.agent_id,
                            self.conv_id.as_deref(),
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
                            self.conv_id.as_deref(),
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

        // Capture session id on the session struct so AgentSession::continuity_handle
        // can return it for the M4 conversation ledger.
        if let Some(id) = session_id_captured {
            self.handle = Some(ContinuityHandle::CursorThreadId(id));
        }

        // Terminal message: either the agent's reply, an explicit error,
        // or a cancellation note.
        if cancelled {
            self.observer
                .on_message_complete("system", "Cancelled by user.");
        } else if let Some(msg) = error_msg.clone() {
            let hint = if is_auth_error(&stderr_text) || is_auth_error(&msg) {
                "\n\nTo re-authenticate, run `agent login` in a terminal, then retry."
            } else {
                ""
            };
            let body = if msg.is_empty() {
                format!("cursor `agent` reported an error.{hint}")
            } else {
                format!("Error: {msg}{hint}")
            };
            self.observer.on_message_complete("system", &body);
        } else if !full_text.is_empty() {
            self.observer.on_message_complete("assistant", &full_text);
        } else if !stderr_text.trim().is_empty() {
            let hint = if is_auth_error(&stderr_text) {
                "\n\nTo re-authenticate, run `agent login` in a terminal, then retry."
            } else {
                ""
            };
            self.observer.on_message_complete(
                "system",
                &format!("cursor `agent` produced no output.\n{}{hint}", stderr_text.trim()),
            );
        } else {
            self.observer.on_message_complete(
                "system",
                "cursor `agent` produced no output. \
                 Check that the agent is logged in (`agent whoami`).",
            );
        }

        if error_msg.is_some() && !cancelled {
            anyhow::bail!("cursor turn failed");
        }
        Ok(())
    }

    /// Dispatch a single parsed [`CursorEvent`] against the streaming
    /// state. Returns `true` when the caller should break out of the
    /// streaming loop (terminal `result` event).
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_event(
        &mut self,
        event: CursorEvent,
        full_text: &mut String,
        last_segment_text: &mut String,
        in_thinking: &mut bool,
        snapshots: &mut HashMap<PathBuf, Option<String>>,
        pending_writes: &mut HashMap<String, (PathBuf, String)>,
        error_msg: &mut Option<String>,
        session_id_captured: &mut Option<String>,
    ) -> bool {
        match event {
            CursorEvent::SystemInit { session_id, .. } => {
                if !session_id.is_empty() {
                    self.observer.on_cursor_session_started(&session_id);
                    *session_id_captured = Some(session_id);
                }
                false
            }
            CursorEvent::UserEcho => false,
            CursorEvent::AssistantDelta(text) => {
                if *in_thinking {
                    self.observer.on_stream_chunk("\n</think>\n");
                    *in_thinking = false;
                }
                self.observer.on_stream_chunk(&text);
                full_text.push_str(&text);
                false
            }
            CursorEvent::AssistantSegment(text) => {
                // Don't print — the deltas already covered this segment.
                // We retain the text as a fallback for the (unlikely) case
                // where the CLI emitted no deltas for a turn.
                *last_segment_text = text;
                false
            }
            CursorEvent::ThinkingDelta(text) => {
                if !*in_thinking {
                    self.observer.on_stream_chunk("<think>\n");
                    *in_thinking = true;
                }
                self.observer.on_stream_chunk(&text);
                false
            }
            CursorEvent::ThinkingCompleted => {
                if *in_thinking {
                    self.observer.on_stream_chunk("\n</think>\n");
                    *in_thinking = false;
                }
                false
            }
            CursorEvent::ToolCallStarted {
                id,
                name,
                args_json,
            } => {
                let tool_label = tool_display_name(&name);
                self.observer.on_tool_call_started(&tool_label);
                self.observer
                    .on_streaming_status(&format!("Using {}...", tool_label));

                // Snapshot the target file BEFORE the tool finishes. The
                // CLI runs the tool concurrently with NDJSON emission, so
                // by the time we see `completed` the file may already
                // contain the new content. Reading on `started` gives us
                // the pre-change baseline most of the time; we still
                // drift-check post-stream before reverting.
                if let Some((rel_or_abs, proposed)) = write_tool_args(&name, &args_json) {
                    let abs_path = if rel_or_abs.is_absolute() {
                        rel_or_abs.clone()
                    } else {
                        self.workspace_root.join(&rel_or_abs)
                    };
                    if !snapshots.contains_key(&abs_path) {
                        let snapshot = match tokio::fs::read_to_string(&abs_path).await {
                            Ok(s) => Some(s),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
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
                            "Snapshot before cursor tool {}: {} ({})",
                            tool_label,
                            abs_path.display(),
                            match &snapshot {
                                Some(s) => format!("{} bytes", s.len()),
                                None => "did not exist".to_string(),
                            }
                        );
                        snapshots.insert(abs_path.clone(), snapshot);
                    }
                    pending_writes.insert(id, (abs_path, proposed));
                }
                false
            }
            CursorEvent::ToolCallCompleted { id, .. } => {
                // We only track pending writes for snapshot bookkeeping;
                // the actual content comparison happens against on-disk
                // state after the stream ends so racy mid-stream reads
                // don't matter. Drop the pending entry so the map stays
                // bounded.
                pending_writes.remove(&id);
                false
            }
            CursorEvent::ResultSuccess { text, usage, .. } => {
                if let Some(u) = usage.as_ref() {
                    tracing::info!(
                        target: "turn_metrics",
                        provider = "cursor",
                        input_tokens = u.input_tokens,
                        output_tokens = u.output_tokens,
                        "turn_token_usage"
                    );
                    self.observer
                        .on_turn_token_usage(&crate::acp::protocol::TokenUsage {
                            input_tokens: u.input_tokens,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            output_tokens: u.output_tokens,
                        });
                }
                if full_text.is_empty() && !text.is_empty() {
                    *full_text = text;
                }
                true
            }
            CursorEvent::ResultError { message, usage } => {
                if let Some(u) = usage.as_ref() {
                    self.observer
                        .on_turn_token_usage(&crate::acp::protocol::TokenUsage {
                            input_tokens: u.input_tokens,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            output_tokens: u.output_tokens,
                        });
                }
                *error_msg = Some(message);
                true
            }
        }
    }
}

/// Append a `<workspace_folders>` hint to the user message listing
/// sibling-folder roots. Cursor's CLI has no documented multi-root flag,
/// so this is the same fallback the Codex app-server path uses for
/// workspace-mode awareness.
fn embed_workspace_folders(
    user_message: &str,
    workspace_root: &Path,
    additional_roots: &[PathBuf],
) -> String {
    if additional_roots.is_empty() {
        return user_message.to_string();
    }
    let mut hint = String::from("\n\n<workspace_folders>\n");
    hint.push_str(&format!("primary: {}\n", workspace_root.display()));
    for r in additional_roots {
        if r.as_os_str().is_empty() || r == workspace_root {
            continue;
        }
        hint.push_str(&format!("sibling: {}\n", r.display()));
    }
    hint.push_str("</workspace_folders>\n");
    hint.push_str(
        "Read freely from any folder above. File edits land in the primary cwd by default.",
    );
    format!("{user_message}{hint}")
}

#[async_trait::async_trait]
impl AgentSession for CursorSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        self.run_cursor_turn(&turn).await?;
        // Phase 1 returns an empty stream — events flow through the
        // observer. A later milestone migrates the TUI to consume the
        // stream directly, matching the Claude/Codex path.
        Ok(Box::pin(futures::stream::empty()))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.profile.continuity_mode
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        self.handle.as_ref()
    }

    async fn close(self: Box<Self>) {
        // Subprocess is spawned per send_turn and torn down before that
        // future resolves — nothing to release here in phase 1.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_workspace_folders_returns_original_when_no_siblings() {
        let msg = "do the thing";
        assert_eq!(
            embed_workspace_folders(msg, Path::new("/work/proj"), &[]),
            msg
        );
    }

    #[test]
    fn embed_workspace_folders_lists_siblings_and_skips_workspace_root() {
        let msg = "do the thing";
        let workspace = PathBuf::from("/work/proj");
        let siblings = vec![
            PathBuf::from("/work/lib-a"),
            PathBuf::from("/work/proj"), // skip — same as workspace root
            PathBuf::from("/work/lib-b"),
        ];
        let out = embed_workspace_folders(msg, &workspace, &siblings);
        assert!(out.contains("do the thing"));
        assert!(out.contains("<workspace_folders>"));
        assert!(out.contains("primary: /work/proj"));
        assert!(out.contains("sibling: /work/lib-a"));
        assert!(out.contains("sibling: /work/lib-b"));
        // The dedup rule: workspace root must not show up as a sibling
        // even when the caller mistakenly passes it.
        let sibling_workspace = format!("sibling: /work/proj");
        assert!(
            !out.contains(&sibling_workspace),
            "workspace root must not be listed as a sibling"
        );
        assert!(out.contains("</workspace_folders>"));
    }
}
