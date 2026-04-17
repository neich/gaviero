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

use crate::acp::client::{
    PROCESS_WAIT_TIMEOUT, STREAM_IDLE_TIMEOUT, format_tool_summary, is_auth_error, propose_write,
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

/// M6 `AgentSession` implementation for Claude Code (`claude-code:` / `claude:`
/// model prefix). Owns the subprocess lifecycle for one turn and drives resume
/// via `ContinuityHandle::ClaudeSessionId` rather than the deprecated
/// `AgentOptions::resume_session_id` field.
pub struct ClaudeSession {
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Box<dyn AcpObserver>,
    /// Claude model name, stripped of `claude-code:` / `claude:` prefix.
    claude_model: String,
    workspace_root: PathBuf,
    agent_id: String,
    effort: String,
    max_tokens: u32,
    auto_approve: bool,
    profile: ProviderProfile,
    /// Resume handle, initialized from `options.resume_session_id` at
    /// construction. M6: set once and used as input to `AcpSession::spawn`
    /// to pass `--resume <id>`. Updated after `send_turn` via the observer
    /// callback (`on_claude_session_started`) rather than in-struct — the
    /// TUI controller remains the authoritative updater in M6.
    handle: Option<ContinuityHandle>,
}

impl ClaudeSession {
    /// Construct a new `ClaudeSession` from the unified `SessionConstruction`
    /// args. Called exclusively by `registry::create_session` for Claude
    /// providers (`ContinuityMode::NativeResume`).
    pub(super) fn new(args: SessionConstruction) -> Self {
        // Strip provider prefix — `claude-code:sonnet` → `sonnet`,
        // `claude:haiku` → `haiku`, bare `sonnet` → `sonnet`.
        let claude_model = args
            .model
            .strip_prefix("claude-code:")
            .or_else(|| args.model.strip_prefix("claude:"))
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

        Self {
            write_gate: args.write_gate,
            observer: args.observer,
            claude_model,
            workspace_root: args.workspace_root,
            agent_id: args.agent_id,
            effort,
            max_tokens,
            auto_approve,
            profile: args.profile,
            handle,
        }
    }

    /// Reconstruct the enriched prompt and all legacy inputs from the
    /// structured `Turn`, then run the Claude subprocess loop. This is the
    /// core of the Claude streaming path, extracted from
    /// `AcpPipeline::send_prompt_via_claude` (M6 parity reference kept there
    /// as `#[allow(dead_code)]` until M10).
    async fn run_claude_turn(&self, turn: &Turn, resume_session_id: Option<String>) -> Result<()> {
        // ── Reconstruct legacy inputs from Turn ──────────────────────────

        // Enriched prompt: graph block → memory block → user message (byte-identical
        // to `LegacyAgentSession::send_turn` and the pre-M5 chat path).
        let mut parts: Vec<String> = Vec::new();
        if let Some(block) = shared::render_graph_block(&turn.graph_selections) {
            parts.push(block);
        }
        if let Some(block) = shared::render_memory_block(&turn.memory_selections) {
            parts.push(block);
        }
        parts.push(turn.user_message.clone());
        let enriched_prompt = parts.join("\n\n");

        // Split FileAttachment into (path, content) text refs vs bare-path
        // image/document attachments (routed via `--file`).
        let mut file_refs: Vec<(String, String)> = Vec::new();
        let mut file_attachments: Vec<PathBuf> = Vec::new();
        for f in &turn.file_refs {
            match &f.content {
                Some(text) => file_refs.push((f.path.to_string_lossy().into_owned(), text.clone())),
                None => file_attachments.push(f.path.clone()),
            }
        }

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
                resume_session_id,
            }
        };

        // ── Tool permission configuration ─────────────────────────────────
        let available_tools = &["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"];
        let approved_tools: &[&str] = if self.auto_approve {
            available_tools
        } else {
            &["Read", "Glob", "Grep"]
        };

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

        let attach_refs: Vec<&Path> = file_attachments.iter().map(|p| p.as_path()).collect();

        let mut session = AcpSession::spawn(
            &self.claude_model,
            &self.workspace_root,
            &final_prompt,
            &system_prompt,
            available_tools,
            approved_tools,
            &options,
            &attach_refs,
        )?;

        // ── Streaming loop (identical to send_prompt_via_claude) ──────────
        let mut full_text = String::new();
        let mut file_scan_pos: usize = 0;
        let mut in_thinking = false;
        let mut file_snapshots: HashMap<PathBuf, String> = HashMap::new();
        let mut read_count: usize = 0;
        let mut idle_count: u32 = 0;

        loop {
            let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, session.next_event()).await;

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

                                while let Some((rel_path, content, end)) =
                                    crate::acp::protocol::find_next_file_block(
                                        &full_text,
                                        file_scan_pos,
                                    )
                                {
                                    tracing::info!(
                                        "Detected <file> block: path={}, content_len={}",
                                        rel_path.display(),
                                        content.len()
                                    );
                                    file_scan_pos = end;
                                    if let Err(e) = propose_write(
                                        &self.write_gate,
                                        self.observer.as_ref(),
                                        &self.workspace_root,
                                        &self.agent_id,
                                        &rel_path,
                                        &content,
                                    )
                                    .await
                                    {
                                        tracing::error!(
                                            "Failed to create proposal for {}: {}",
                                            rel_path.display(),
                                            e
                                        );
                                    }
                                }
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
                                                let content = tokio::fs::read_to_string(&abs_path)
                                                    .await
                                                    .unwrap_or_default();
                                                tracing::info!(
                                                    "Snapshot before tool {}: {} ({} bytes)",
                                                    tu.name,
                                                    abs_path.display(),
                                                    content.len()
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

        // Catch any <file> blocks that completed after the last ContentDelta.
        let remaining = crate::acp::protocol::parse_file_blocks(&full_text[file_scan_pos..]);
        for (rel_path, content) in remaining {
            if let Err(e) = propose_write(
                &self.write_gate,
                self.observer.as_ref(),
                &self.workspace_root,
                &self.agent_id,
                &rel_path,
                &content,
            )
            .await
            {
                tracing::error!(
                    "Failed to create proposal for {}: {}",
                    rel_path.display(),
                    e
                );
            }
        }

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
            let snapshots: Vec<(PathBuf, String)> = file_snapshots.into_iter().collect();
            for (i, (abs_path, original)) in snapshots.iter().enumerate() {
                let current = tokio::fs::read_to_string(abs_path)
                    .await
                    .unwrap_or_default();
                if current == *original {
                    tracing::info!("Snapshot unchanged: {}", abs_path.display());
                    continue;
                }
                tracing::info!(
                    "File changed by tool: {} (orig={} bytes, new={} bytes)",
                    abs_path.display(),
                    original.len(),
                    current.len()
                );
                if total > 1 {
                    self.observer.on_streaming_status(&format!(
                        "Processing file {}/{}...",
                        i + 1,
                        total
                    ));
                }
                if let Err(e) = tokio::fs::write(abs_path, original).await {
                    tracing::error!("Failed to revert {}: {}", abs_path.display(), e);
                    continue;
                }
                let rel_path = abs_path
                    .strip_prefix(&self.workspace_root)
                    .unwrap_or(abs_path.as_path());
                if let Err(e) = propose_write(
                    &self.write_gate,
                    self.observer.as_ref(),
                    &self.workspace_root,
                    &self.agent_id,
                    rel_path,
                    &current,
                )
                .await
                {
                    tracing::error!(
                        "Failed to create proposal for {}: {}",
                        abs_path.display(),
                        e
                    );
                }
            }
        }

        Ok(())
    }
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
