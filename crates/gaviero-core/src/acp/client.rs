//! AcpPipeline — sends prompts to the configured provider and routes file changes
//! through the Write Gate.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::observer::AcpObserver;
use crate::swarm::backend::AgentBackend as _;
use crate::swarm::backend::{executor, shared, CompletionRequest};
use crate::write_gate::WriteGatePipeline;

use super::protocol::{StreamEvent, find_next_file_block, parse_file_blocks};
use super::session::{AcpSession, AgentOptions};

/// How long to wait for the next stream event before sending a keepalive status.
/// Claude tool calls (Read, Grep on large repos) can take a while, so this should
/// be generous enough to avoid false positives but short enough to reassure the user.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time to wait for the subprocess to exit after the stream ends.
/// After this, the process is killed to avoid blocking the pipeline indefinitely.
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// The ACP pipeline manages chat execution and routes file changes
/// through the Write Gate.
///
/// Each `send_prompt()` call spawns a fresh subprocess. File changes
/// proposed by the agent are routed through the Write Gate for review.
pub struct AcpPipeline {
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Box<dyn AcpObserver>,
    model: String,
    ollama_base_url: Option<String>,
    workspace_root: PathBuf,
    agent_id: String,
    options: AgentOptions,
}

impl AcpPipeline {
    pub fn new(
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        observer: Box<dyn AcpObserver>,
        model: impl Into<String>,
        ollama_base_url: Option<String>,
        workspace_root: impl Into<PathBuf>,
        agent_id: impl Into<String>,
        options: AgentOptions,
    ) -> Self {
        Self {
            write_gate,
            observer,
            model: model.into(),
            ollama_base_url,
            workspace_root: workspace_root.into(),
            agent_id: agent_id.into(),
            options,
        }
    }

    /// Send a user prompt and process the streaming response.
    ///
    /// `@path/to/file` references in the prompt are resolved: the file
    /// contents are read and prepended as context. After the subprocess
    /// completes, any `<file>` blocks in the response are parsed and
    /// routed through the Write Gate as proposals.
    pub async fn send_prompt(
        &self,
        prompt: &str,
        file_refs: &[(String, String)],
        conversation_history: &[(String, String)],
        file_attachments: &[PathBuf],
    ) -> Result<()> {
        // M0 instrumentation: one "turn" event per send_prompt. Captures
        // prompt size, provider selection, and continuity-mode hint (resume
        // id presence) so baselines can diff first-turn vs follow-up.
        let provider_hint = if shared::is_ollama_model(&self.model) {
            "ollama"
        } else if shared::is_codex_model(&self.model) {
            "codex"
        } else {
            "claude"
        };
        let resume_hint = self
            .options
            .resume_session_id
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
            provider = provider_hint,
            model = %self.model,
            prompt_chars = prompt.len(),
            replay_chars = history_chars,
            file_refs_count = file_refs.len(),
            file_attachments_count = file_attachments.len(),
            resume_hint,
            "turn_dispatch"
        );

        // Codex and Ollama both go through the trait-based executor path.
        // Only Claude Code uses the specialized ACP session (below) for its
        // bidirectional permission handshake.
        if shared::is_ollama_model(&self.model) || shared::is_codex_model(&self.model) {
            let backend =
                shared::create_backend_for_model(&self.model, self.ollama_base_url.as_deref())?;
            let caps = backend.capabilities();
            let request = CompletionRequest {
                prompt: prompt.to_string(),
                system_prompt: Some(shared::default_editor_system_prompt(&caps)),
                workspace_root: self.workspace_root.clone(),
                allowed_tools: if caps.tool_use {
                    vec![
                        "Read".into(),
                        "Glob".into(),
                        "Grep".into(),
                        "Write".into(),
                        "Edit".into(),
                        "MultiEdit".into(),
                    ]
                } else {
                    vec![]
                },
                file_attachments: file_attachments.to_vec(),
                conversation_history: conversation_history.to_vec(),
                file_refs: file_refs.to_vec(),
                effort: Some(self.options.effort.clone()),
                max_tokens: Some(self.options.max_tokens),
                auto_approve: self.options.auto_approve,
            };
            executor::complete_to_write_gate(
                &*backend,
                request,
                self.observer.as_ref(),
                self.write_gate.clone(),
                &self.agent_id,
            )
            .await?;
            return Ok(());
        }

        self.send_prompt_via_claude(prompt, file_refs, conversation_history, file_attachments)
            .await
    }

    async fn send_prompt_via_claude(
        &self,
        prompt: &str,
        file_refs: &[(String, String)],
        conversation_history: &[(String, String)],
        file_attachments: &[PathBuf],
    ) -> Result<()> {
        let claude_model = self
            .model
            .strip_prefix("claude-code:")
            .or_else(|| self.model.strip_prefix("claude:"))
            .unwrap_or(&self.model);
        let available_tools = &["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"];
        let approved_tools: &[&str] = if self.options.auto_approve {
            available_tools
        } else {
            &["Read", "Glob", "Grep"]
        };

        let enriched_prompt = shared::build_enriched_prompt(prompt, conversation_history, file_refs);
        let system_prompt = shared::default_editor_system_prompt(
            &crate::swarm::backend::claude_code::ClaudeCodeBackend::new(claude_model).capabilities(),
        );

        let attach_refs: Vec<&std::path::Path> = file_attachments
            .iter()
            .map(|p| p.as_path())
            .collect();

        let mut session = AcpSession::spawn(
            claude_model,
            &self.workspace_root,
            &enriched_prompt,
            &system_prompt,
            available_tools,
            approved_tools,
            &self.options,
            &attach_refs,
        )?;

        // Accumulate the full response text; detect <file> blocks incrementally
        let mut full_text = String::new();
        let mut file_scan_pos: usize = 0; // how far we've scanned for complete blocks
        let mut in_thinking = false;

        // Track files that Write/Edit tools will modify — snapshot BEFORE CLI executes them.
        // Key: absolute path, Value: original content at the time of snapshot.
        let mut file_snapshots: HashMap<PathBuf, String> = HashMap::new();

        // M0 instrumentation: count Read tool invocations per turn so baselines
        // can measure graph pre-attach effectiveness (M7 target).
        let mut read_count: usize = 0;

        // Process streaming events with idle timeout.
        // During tool execution, Claude CLI emits no events — the idle timeout
        // detects this and sends keepalive status updates so the UI stays responsive.
        let mut idle_count: u32 = 0;
        loop {
            let next = tokio::time::timeout(
                STREAM_IDLE_TIMEOUT,
                session.next_event(),
            )
            .await;

            match next {
                // Timeout — no event received within STREAM_IDLE_TIMEOUT
                Err(_elapsed) => {
                    idle_count += 1;
                    let elapsed_secs = idle_count * STREAM_IDLE_TIMEOUT.as_secs() as u32;
                    // Check if the subprocess is still alive
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
                    // Process still alive — send keepalive status
                    self.observer.on_streaming_status(
                        &format!("Working... ({}s elapsed, tools running)", elapsed_secs),
                    );
                    tracing::debug!(
                        "Stream idle for {}s, subprocess still alive — sending keepalive",
                        elapsed_secs
                    );
                    continue;
                }
                // Got a result from next_event()
                Ok(result) => {
                    idle_count = 0; // reset on any activity
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

                        // Detect complete <file> blocks as they arrive (fallback path)
                        while let Some((rel_path, content, end)) =
                            find_next_file_block(&full_text, file_scan_pos)
                        {
                            tracing::info!(
                                "Detected <file> block: path={}, content_len={}",
                                rel_path.display(), content.len()
                            );
                            file_scan_pos = end;
                            if let Err(e) = self.propose_write(&rel_path, &content).await {
                                tracing::error!(
                                    "Failed to create proposal for {}: {}",
                                    rel_path.display(), e
                                );
                            }
                        }
                    }
                    StreamEvent::ToolUseStart { tool_name, .. } => {
                        // M0 instrumentation: count Read invocations per turn.
                        if tool_name == "Read" {
                            read_count += 1;
                        }
                        // Update streaming status for the spinner label.
                        // The enriched tool call (with details) will be sent
                        // from AssistantMessage when the full input is known.
                        self.observer.on_streaming_status(&format!("Using {}...", tool_name));
                    }
                    StreamEvent::ToolInputDelta(_) => {
                        // Tool input JSON fragments — ignored here.
                        // The AcpPipeline extracts tool details from AssistantMessage instead.
                    }
                    StreamEvent::AssistantMessage { text, tool_uses } => {
                        // Use the complete message text if we didn't get deltas
                        if full_text.is_empty() && !text.is_empty() {
                            full_text = text;
                        }

                        // Enrich tool calls with details from the input JSON.
                        // This replaces the bare names sent by ToolUseStart.
                        for tu in &tool_uses {
                            let summary = format_tool_summary(&tu.name, &tu.input, &self.workspace_root);
                            self.observer.on_tool_call_started(&summary);
                        }

                        // Snapshot files BEFORE the CLI executes Write/Edit tools.
                        // The AssistantMessage fires when the model finishes a turn,
                        // before the CLI executes the tool calls within it.
                        for tu in &tool_uses {
                            if matches!(tu.name.as_str(), "Write" | "Edit" | "MultiEdit") {
                                if let Some(fp) = tu.input.get("file_path").and_then(|v| v.as_str()) {
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
                                            tu.name, abs_path.display(), content.len()
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
                    StreamEvent::PermissionRequest { tool_name, description, request_id } => {
                        // Pause the pipeline: ask the observer (TUI) for a decision.
                        // The session is not borrowed here so awaiting is safe.
                        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                        self.observer.on_permission_request(&tool_name, &description, tx);
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
                        // M0 instrumentation: record whether Claude honored the
                        // resume id we passed. `resume_accepted` = session id
                        // we got back matches the id we asked for.
                        let asked = self.options.resume_session_id.as_deref();
                        let resume_accepted = match asked {
                            Some(asked_id) if !asked_id.is_empty() && !session_id.is_empty() => {
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
                        // Hand the fresh session id to the observer so the TUI
                        // can persist it on the Conversation and pass it back
                        // via AgentOptions::resume_session_id on the next turn.
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
                        // Include stderr/unparsed stdout + exit status for diagnostics.
                        // Wait for process to exit first so stderr drain task can finish.
                        let exit_status = match tokio::time::timeout(
                            PROCESS_WAIT_TIMEOUT,
                            session.wait(),
                        ).await {
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
                            let base = format!("Claude CLI error{}:\n{}", exit_info, stderr);
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

        // Close any open thinking block
        if in_thinking {
            self.observer.on_stream_chunk("\n</think>\n");
        }

        // Catch any <file> blocks that completed after the last ContentDelta (fallback)
        let remaining = parse_file_blocks(&full_text[file_scan_pos..]);
        for (rel_path, content) in remaining {
            if let Err(e) = self.propose_write(&rel_path, &content).await {
                tracing::error!("Failed to create proposal for {}: {}", rel_path.display(), e);
            }
        }

        // Wait for subprocess to finish (tools have now been executed).
        // Use a timeout to avoid blocking indefinitely if the process hangs.
        self.observer.on_streaming_status("Finalizing...");
        match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, session.wait()).await {
            Ok(Ok(status)) => {
                tracing::info!("Claude subprocess exited: {}", status);
            }
            Ok(Err(e)) => {
                tracing::warn!("Error waiting for claude subprocess: {}", e);
            }
            Err(_) => {
                tracing::warn!(
                    "Claude subprocess did not exit within {}s, killing",
                    PROCESS_WAIT_TIMEOUT.as_secs()
                );
                session.kill();
            }
        }

        // M0 instrumentation: emit per-turn Read tool count at end of stream.
        tracing::info!(
            target: "turn_metrics",
            provider = "claude",
            read_count,
            "turn_read_count"
        );

        // Create proposals from snapshotted files — compare original vs current disk content.
        // The CLI has already written the files, so we diff snapshot vs disk.
        if !file_snapshots.is_empty() {
            let total = file_snapshots.len();
            tracing::info!(
                "Processing {} file snapshots for tool-based proposals",
                total
            );
            self.observer.on_streaming_status(
                &format!("Processing {} file change{}...", total, if total == 1 { "" } else { "s" }),
            );
            for (i, (abs_path, original)) in file_snapshots.iter().enumerate() {
                let current = tokio::fs::read_to_string(&abs_path)
                    .await
                    .unwrap_or_default();
                if current == *original {
                    tracing::info!("Snapshot unchanged: {}", abs_path.display());
                    continue;
                }
                tracing::info!(
                    "File changed by tool: {} (orig={} bytes, new={} bytes)",
                    abs_path.display(), original.len(), current.len()
                );

                if total > 1 {
                    self.observer.on_streaming_status(
                        &format!("Processing file {}/{}...", i + 1, total),
                    );
                }

                // Revert the file to its original content on disk.
                // The proposal stores both versions; review mode will re-apply if accepted.
                if let Err(e) = tokio::fs::write(&abs_path, &original).await {
                    tracing::error!("Failed to revert {}: {}", abs_path.display(), e);
                    continue;
                }

                // Now create a proposal through the write gate (in Deferred mode)
                let rel_path = abs_path.strip_prefix(&self.workspace_root)
                    .unwrap_or(abs_path.as_path());
                if let Err(e) = self.propose_write(rel_path, &current).await {
                    tracing::error!(
                        "Failed to create proposal for {}: {}",
                        abs_path.display(), e
                    );
                }
            }
        }

        Ok(())
    }

    /// Create a write proposal through the Write Gate.
    ///
    /// Follows the propose_write pattern from SPEC.md 5.3:
    /// brief lock for scope check → expensive work outside lock → brief lock for insertion.
    ///
    /// NOTE: There is a deliberate window between the scope check (step 1) and
    /// insertion (step 5) where another task could finalize a proposal for the
    /// same file. This is accepted to avoid holding the Mutex across I/O. In
    /// practice the risk is low because only one agent session is active at a
    /// time, and duplicate proposals for the same file are harmless (the user
    /// reviews each one independently).
    async fn propose_write(&self, rel_path: &Path, proposed_content: &str) -> Result<()> {
        let abs_path = self.workspace_root.join(rel_path);

        // 1. Scope check + duplicate check + allocate ID (single lock)
        let (id, is_deferred) = {
            let mut gate = self.write_gate.lock().await;
            let path_str = rel_path.to_string_lossy();
            if !gate.is_scope_allowed(&self.agent_id, &path_str) {
                tracing::warn!("Scope rejected for {}", rel_path.display());
                return Ok(());
            }
            if gate.proposal_for_path(&abs_path).is_some() {
                tracing::debug!("Skipping duplicate proposal for {}", rel_path.display());
                return Ok(());
            }
            // Also check deferred proposals for duplicates
            if gate.pending_proposals().iter().any(|p| p.file_path == abs_path) {
                tracing::debug!("Skipping duplicate deferred proposal for {}", rel_path.display());
                return Ok(());
            }
            (gate.next_id(), gate.is_deferred())
        };

        // 2. Read original content + build proposal (outside lock)
        let original = if abs_path.exists() {
            tokio::fs::read_to_string(&abs_path)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };

        if original == proposed_content {
            tracing::info!("propose_write: {} — content unchanged, skipping", rel_path.display());
            return Ok(());
        }

        let proposal = WriteGatePipeline::build_proposal(
            id,
            &self.agent_id,
            &abs_path,
            &original,
            proposed_content,
        );

        if proposal.structural_hunks.is_empty() {
            tracing::info!(
                "propose_write: {} — no structural hunks after diff, skipping",
                rel_path.display()
            );
            return Ok(());
        }

        tracing::info!(
            "propose_write: {} — {} hunks, is_deferred={}, inserting",
            rel_path.display(), proposal.structural_hunks.len(), is_deferred
        );

        // 3. Insert proposal (single lock)
        let auto_accept_result = {
            let mut gate = self.write_gate.lock().await;
            gate.insert_proposal(proposal)
        };

        // 4. If deferred, notify observer for compact summary display
        if is_deferred {
            let old = if original.is_empty() { None } else { Some(original.as_str()) };
            self.observer.on_proposal_deferred(&abs_path, old, proposed_content);
        }

        // 5. If AutoAccept mode, write to disk outside lock
        if let Some((path, content)) = auto_accept_result {
            tokio::fs::write(&path, &content)
                .await
                .context("writing auto-accepted file")?;
        }

        Ok(())
    }
}

/// Return true if the error text indicates an OAuth / authentication failure.
fn is_auth_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    (lower.contains("oauth") || lower.contains("authentication") || lower.contains("unauthorized"))
        && (lower.contains("expired") || lower.contains("invalid") || lower.contains("failed"))
        || lower.contains("oauth token expired")
        || lower.contains("not logged in")
        || lower.contains("please log in")
        || lower.contains("401")
}

/// Format a one-line summary for a tool call, extracting key info from the input JSON.
fn format_tool_summary(tool_name: &str, input: &serde_json::Value, workspace_root: &Path) -> String {
    let get_str = |key: &str| input.get(key).and_then(|v| v.as_str());

    // Try to make paths relative for display
    let rel_path = |p: &str| -> String {
        let path = Path::new(p);
        path.strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    };

    match tool_name {
        "Read" => {
            if let Some(fp) = get_str("file_path") {
                format!("Read {}", rel_path(fp))
            } else {
                "Read".into()
            }
        }
        "Write" => {
            if let Some(fp) = get_str("file_path") {
                format!("Write {}", rel_path(fp))
            } else {
                "Write".into()
            }
        }
        "Edit" | "MultiEdit" => {
            if let Some(fp) = get_str("file_path") {
                format!("{} {}", tool_name, rel_path(fp))
            } else {
                tool_name.into()
            }
        }
        "Grep" => {
            let pattern = get_str("pattern").unwrap_or("?");
            if let Some(path) = get_str("path") {
                format!("Grep '{}' in {}", pattern, rel_path(path))
            } else {
                format!("Grep '{}'", pattern)
            }
        }
        "Glob" => {
            if let Some(pattern) = get_str("pattern") {
                format!("Glob {}", pattern)
            } else {
                "Glob".into()
            }
        }
        "Bash" => {
            if let Some(cmd) = get_str("command") {
                let short: String = cmd.chars().take(60).collect();
                if cmd.len() > 60 { format!("Bash: {}...", short) } else { format!("Bash: {}", short) }
            } else {
                "Bash".into()
            }
        }
        _ => tool_name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_system_prompt_not_empty() {
        let prompt = shared::default_editor_system_prompt(
            &crate::swarm::backend::claude_code::ClaudeCodeBackend::new("sonnet").capabilities(),
        );
        assert!(!prompt.is_empty());
        assert!(prompt.contains("gaviero"));
    }
}
