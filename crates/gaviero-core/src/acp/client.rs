//! AcpPipeline — sends prompts to Claude Code and routes file changes
//! through the Write Gate.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::observer::AcpObserver;
use crate::write_gate::WriteGatePipeline;

use super::protocol::{StreamEvent, find_next_file_block, parse_file_blocks};
use super::session::{AcpSession, AgentOptions};

/// Maximum characters to include per message when sending conversation history.
const HISTORY_TRUNCATION_CHARS: usize = 2000;

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a coding assistant working inside the gaviero editor.

Use the available tools (Read, Glob, Grep, Write, Edit) to understand the codebase and make changes.
When modifying files, use Edit for surgical changes and Write for creating new files or full rewrites."#;

/// The ACP pipeline manages communication with a Claude Code subprocess.
///
/// Each `send_prompt()` call spawns a fresh subprocess. File changes
/// proposed by the agent are routed through the Write Gate for review.
pub struct AcpPipeline {
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Box<dyn AcpObserver>,
    model: String,
    workspace_root: PathBuf,
    agent_id: String,
    options: AgentOptions,
}

impl AcpPipeline {
    pub fn new(
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        observer: Box<dyn AcpObserver>,
        model: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
        agent_id: impl Into<String>,
        options: AgentOptions,
    ) -> Self {
        Self {
            write_gate,
            observer,
            model: model.into(),
            workspace_root: workspace_root.into(),
            agent_id: agent_id.into(),
            options,
        }
    }

    /// Send a user prompt to Claude Code and process the streaming response.
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
        let allowed_tools = &["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"];

        // Build enriched prompt with conversation history + file contents
        let mut parts = Vec::new();

        // Include conversation history for multi-turn context
        if !conversation_history.is_empty() {
            parts.push("Previous conversation:\n".to_string());
            for (role, content) in conversation_history {
                // Truncate long messages to avoid prompt bloat
                let truncated: String = content.chars().take(HISTORY_TRUNCATION_CHARS).collect();
                let ellipsis = if content.chars().count() > HISTORY_TRUNCATION_CHARS { "..." } else { "" };
                parts.push(format!("[{}]: {}{}\n", role, truncated, ellipsis));
            }
            parts.push("---\n".to_string());
        }

        // Include referenced file contents
        if !file_refs.is_empty() {
            parts.push("Referenced files:\n".to_string());
            for (path, content) in file_refs {
                parts.push(format!("--- {} ---\n{}\n--- end {} ---\n", path, content, path));
            }
        }

        // The actual user prompt
        parts.push(prompt.to_string());

        let enriched_prompt = parts.join("\n");

        let attach_refs: Vec<&std::path::Path> = file_attachments
            .iter()
            .map(|p| p.as_path())
            .collect();

        let mut session = AcpSession::spawn(
            &self.model,
            &self.workspace_root,
            &enriched_prompt,
            DEFAULT_SYSTEM_PROMPT,
            allowed_tools,
            &self.options,
            &attach_refs,
        )?;

        // Accumulate the full response text; detect <file> blocks incrementally
        let mut full_text = String::new();
        let mut file_scan_pos: usize = 0; // how far we've scanned for complete blocks

        // Track files that Write/Edit tools will modify — snapshot BEFORE CLI executes them.
        // Key: absolute path, Value: original content at the time of snapshot.
        let mut file_snapshots: HashMap<PathBuf, String> = HashMap::new();

        // Process streaming events
        loop {
            match session.next_event().await {
                Ok(Some(event)) => match event {
                    StreamEvent::ContentDelta(text) => {
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
                        self.observer.on_tool_call_started(&tool_name);
                    }
                    StreamEvent::AssistantMessage { text, tool_uses } => {
                        // Use the complete message text if we didn't get deltas
                        if full_text.is_empty() && !text.is_empty() {
                            full_text = text;
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
                            self.observer
                                .on_message_complete("system", &format!("Error: {}", result_text));
                        } else {
                            if full_text.is_empty() && !result_text.is_empty() {
                                full_text = result_text.clone();
                            }
                            self.observer.on_message_complete("assistant", &full_text);
                        }
                        break;
                    }
                    StreamEvent::SystemInit { .. } => {}
                    StreamEvent::Unknown(_) => {}
                },
                Ok(None) => {
                    if !full_text.is_empty() {
                        self.observer.on_message_complete("assistant", &full_text);
                    } else {
                        // Include stderr/unparsed stdout + exit status for diagnostics.
                        // Wait for process to exit first so stderr drain task can finish.
                        let exit_status = session.wait().await.ok();
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
                            format!("Claude CLI error{}:\n{}", exit_info, stderr)
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

        // Catch any <file> blocks that completed after the last ContentDelta (fallback)
        let remaining = parse_file_blocks(&full_text[file_scan_pos..]);
        for (rel_path, content) in remaining {
            if let Err(e) = self.propose_write(&rel_path, &content).await {
                tracing::error!("Failed to create proposal for {}: {}", rel_path.display(), e);
            }
        }

        // Wait for subprocess to finish (tools have now been executed)
        let _ = session.wait().await;

        // Create proposals from snapshotted files — compare original vs current disk content.
        // The CLI has already written the files, so we diff snapshot vs disk.
        if !file_snapshots.is_empty() {
            tracing::info!(
                "Processing {} file snapshots for tool-based proposals",
                file_snapshots.len()
            );
            for (abs_path, original) in &file_snapshots {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_system_prompt_not_empty() {
        assert!(!DEFAULT_SYSTEM_PROMPT.is_empty());
        assert!(DEFAULT_SYSTEM_PROMPT.contains("gaviero"));
    }
}
