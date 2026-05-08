//! AcpPipeline — sends prompts to the configured provider and routes file changes
//! through the Write Gate.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::observer::AcpObserver;
use crate::swarm::backend::{CompletionRequest, executor, shared};
use crate::write_gate::{AutoAcceptAction, WriteGatePipeline};

use super::session::AgentOptions;

/// How long to wait for the next stream event before sending a keepalive status.
/// Claude tool calls (Read, Grep on large repos) can take a while, so this should
/// be generous enough to avoid false positives but short enough to reassure the user.
pub(crate) const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time to wait for the subprocess to exit after the stream ends.
/// After this, the process is killed to avoid blocking the pipeline indefinitely.
pub(crate) const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Sibling workspace folders (workspace-mode multi-folder). Forwarded
    /// to the codex/ollama backend via `CompletionRequest::additional_roots`
    /// so the CLI sees one `--add-dir` per folder. Empty in single-folder
    /// mode.
    additional_roots: Vec<PathBuf>,
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
        additional_roots: Vec<PathBuf>,
        agent_id: impl Into<String>,
        options: AgentOptions,
    ) -> Self {
        Self {
            write_gate,
            observer,
            model: model.into(),
            ollama_base_url,
            workspace_root: workspace_root.into(),
            additional_roots,
            agent_id: agent_id.into(),
            options,
        }
    }

    /// Send a user prompt and process the streaming response.
    ///
    /// `@path/to/file` references in the prompt are resolved: the file
    /// contents are read and prepended as context. File edits flow through
    /// the backend's native channel (Claude tool calls; in-band marker
    /// for Codex/Ollama) — no in-band parsing is performed on the Claude
    /// path, since instructing Claude about an in-band marker caused the
    /// model to quote it back in prose and create false-positive proposals.
    // M6: `resume_session_id` is deprecated for new code but still read here
    // for the Ollama/Codex instrumentation path via `LegacyAgentSession`.
    // This allow stays until M10 deletes the field.
    #[allow(deprecated)]
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
                additional_roots: self.additional_roots.clone(),
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
                extra: Vec::new(),
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

        // M6: Claude models now route through `ClaudeSession` in
        // `agent_session/claude.rs`. `AcpPipeline::send_prompt` is only
        // called from `LegacyAgentSession`, which is only instantiated for
        // Ollama/Codex providers after M6. If this branch is reached the
        // registry is misconfigured — log and bail rather than silently
        // running the legacy Claude path.
        tracing::error!(
            model = %self.model,
            "send_prompt reached Claude branch — route through ClaudeSession instead (registry bug)"
        );
        Ok(())
    }

}

/// Create a write proposal through the Write Gate.
///
/// Extracted as a `pub(crate)` free function so both `AcpPipeline` (legacy
/// Ollama/Codex path) and `ClaudeSession` (M6) can call it without
/// duplicating the logic. Follows the propose_write pattern from SPEC.md 5.3:
/// brief lock for scope check → expensive work outside lock → brief lock for
/// insertion.
///
/// NOTE: There is a deliberate window between the scope check (step 1) and
/// insertion (step 5) where another task could finalize a proposal for the
/// same file. This is accepted to avoid holding the Mutex across I/O. In
/// practice the risk is low because only one agent session is active at a
/// time, and duplicate proposals for the same file are harmless (the user
/// reviews each one independently).
pub(crate) async fn propose_write(
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &dyn AcpObserver,
    workspace_root: &Path,
    agent_id: &str,
    rel_path: &Path,
    proposed_content: &str,
) -> Result<()> {
    let abs_path = workspace_root.join(rel_path);

    // 1. Scope check + duplicate check + allocate ID (single lock)
    let (id, is_deferred) = {
        let mut gate = write_gate.lock().await;
        let path_str = rel_path.to_string_lossy();
        if !gate.is_scope_allowed(agent_id, &path_str) {
            tracing::warn!("Scope rejected for {}", rel_path.display());
            return Ok(());
        }
        if gate.proposal_for_path(&abs_path).is_some() {
            tracing::warn!(
                "Dropping later proposal for {} — an earlier proposal for this path is already pending review. \
                 If the agent is correcting itself, the user is reviewing stale content.",
                rel_path.display()
            );
            return Ok(());
        }
        // Also check deferred proposals for duplicates
        if gate
            .pending_proposals()
            .iter()
            .any(|p| p.file_path == abs_path)
        {
            tracing::warn!(
                "Dropping later deferred proposal for {} — an earlier proposal for this path is already queued. \
                 If the agent is correcting itself, the user is reviewing stale content.",
                rel_path.display()
            );
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
        tracing::info!(
            "propose_write: {} — content unchanged, skipping",
            rel_path.display()
        );
        return Ok(());
    }

    let proposal =
        WriteGatePipeline::build_proposal(id, agent_id, &abs_path, &original, proposed_content);

    if proposal.structural_hunks.is_empty() {
        tracing::info!(
            "propose_write: {} — no structural hunks after diff, skipping",
            rel_path.display()
        );
        return Ok(());
    }

    tracing::info!(
        "propose_write: {} — {} hunks, is_deferred={}, inserting",
        rel_path.display(),
        proposal.structural_hunks.len(),
        is_deferred
    );

    // 3. Insert proposal (single lock)
    let auto_accept_result = {
        let mut gate = write_gate.lock().await;
        gate.insert_proposal(proposal)
    };

    // 4. If deferred, notify observer for compact summary display
    if is_deferred {
        let old = if original.is_empty() {
            None
        } else {
            Some(original.as_str())
        };
        observer.on_proposal_deferred(&abs_path, old, proposed_content);
    }

    // 5. If AutoAccept mode, perform the disk action outside the lock.
    if let Some(action) = auto_accept_result {
        match action {
            AutoAcceptAction::Write { path, content } => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .context("creating parent directories")?;
                }
                tokio::fs::write(&path, &content)
                    .await
                    .context("writing auto-accepted file")?;
            }
            AutoAcceptAction::Delete { path } => match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("removing auto-accepted file"),
            },
        }
    }

    Ok(())
}

/// Create a deletion proposal through the Write Gate. Mirrors `propose_write`
/// but the proposal carries `is_deletion = true` and an empty proposed
/// content; on accept, the finalize path removes the file from disk.
///
/// Skips silently when scope rejects the path or an earlier proposal for the
/// same path is already pending — same dedup contract as `propose_write`.
pub(crate) async fn propose_delete(
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &dyn AcpObserver,
    workspace_root: &Path,
    agent_id: &str,
    rel_path: &Path,
    original_content: &str,
) -> Result<()> {
    let abs_path = workspace_root.join(rel_path);

    let (id, is_deferred) = {
        let mut gate = write_gate.lock().await;
        let path_str = rel_path.to_string_lossy();
        if !gate.is_scope_allowed(agent_id, &path_str) {
            tracing::warn!("Scope rejected for delete of {}", rel_path.display());
            return Ok(());
        }
        if gate.proposal_for_path(&abs_path).is_some() {
            tracing::warn!(
                "Dropping delete proposal for {} — earlier proposal already pending review",
                rel_path.display()
            );
            return Ok(());
        }
        if gate
            .pending_proposals()
            .iter()
            .any(|p| p.file_path == abs_path)
        {
            tracing::warn!(
                "Dropping delete proposal for {} — earlier proposal already queued",
                rel_path.display()
            );
            return Ok(());
        }
        (gate.next_id(), gate.is_deferred())
    };

    let proposal =
        WriteGatePipeline::build_delete_proposal(id, agent_id, &abs_path, original_content);

    tracing::info!(
        "propose_delete: {} — {} bytes, is_deferred={}, inserting",
        rel_path.display(),
        original_content.len(),
        is_deferred
    );

    let auto_accept_result = {
        let mut gate = write_gate.lock().await;
        gate.insert_proposal(proposal)
    };

    if is_deferred {
        // Surface as a deferred proposal with proposed_content=empty; the
        // batch-review render branches on `is_deletion` for the label.
        observer.on_proposal_deferred(&abs_path, Some(original_content), "");
    }

    if let Some(action) = auto_accept_result {
        match action {
            AutoAcceptAction::Delete { path } => match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("removing auto-accepted file"),
            },
            AutoAcceptAction::Write { .. } => {
                tracing::error!(
                    "build_delete_proposal returned a Write action for {} — bug",
                    rel_path.display()
                );
            }
        }
    }

    Ok(())
}

/// Return true if the error text indicates an OAuth / authentication failure.
pub(crate) fn is_auth_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    (lower.contains("oauth") || lower.contains("authentication") || lower.contains("unauthorized"))
        && (lower.contains("expired") || lower.contains("invalid") || lower.contains("failed"))
        || lower.contains("oauth token expired")
        || lower.contains("not logged in")
        || lower.contains("please log in")
        || lower.contains("401")
}

/// Format a one-line summary for a tool call, extracting key info from the input JSON.
pub(crate) fn format_tool_summary(
    tool_name: &str,
    input: &serde_json::Value,
    workspace_root: &Path,
) -> String {
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
                if cmd.len() > 60 {
                    format!("Bash: {}...", short)
                } else {
                    format!("Bash: {}", short)
                }
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
    use crate::swarm::backend::AgentBackend;

    #[test]
    fn test_default_system_prompt_not_empty() {
        let prompt = shared::default_editor_system_prompt(
            &crate::swarm::backend::claude_code::ClaudeCodeBackend::new("sonnet").capabilities(),
        );
        assert!(!prompt.is_empty());
        assert!(prompt.contains("gaviero"));
    }
}
