//! Unified backend runner.
//!
//! Consumes a `Box<dyn AgentBackend>` and produces an `AgentManifest`,
//! replacing the dual code paths in the old `AgentRunner`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::memory::MemoryStore;
use crate::observer::AcpObserver;
use crate::repo_map::RepoMap;
use crate::swarm::board::{parse_discoveries, SharedBoard};
use crate::validation_gate::ValidationPipeline;
use crate::write_gate::WriteGatePipeline;

use super::super::models::{AgentManifest, AgentStatus, WorkUnit};
use super::{AgentBackend, CompletionRequest, UnifiedStreamEvent};

/// Run a work unit through any `AgentBackend`, producing an `AgentManifest`.
///
/// When `validation` is provided, runs the validation pipeline after each agent
/// turn. On failure the error is fed back as a corrective prompt and the agent
/// retries up to `work_unit.max_retries` additional times.
///
/// When `repo_map` is provided, a ranked context outline is prepended to the
/// agent's base prompt so that even cheap-tier models have optimal scope.
#[tracing::instrument(
    skip(backend, write_gate, memory, observer, validation, board, repo_map),
    fields(agent_id = %work_unit.id, tier = ?work_unit.tier)
)]
pub async fn run_backend(
    backend: &dyn AgentBackend,
    work_unit: &WorkUnit,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    workspace_root: &Path,
    memory: Option<&MemoryStore>,
    read_namespaces: &[String],
    observer: &dyn AcpObserver,
    validation: Option<&ValidationPipeline>,
    board: Option<&SharedBoard>,
    repo_map: Option<&RepoMap>,
) -> Result<AgentManifest> {
    let agent_id = format!("agent-{}", work_unit.id);

    // 1. Register scope with write gate (once — persists across retries)
    {
        let mut gate = write_gate.lock().await;
        gate.register_agent_scope(&agent_id, &work_unit.scope);
    }

    // 2. Build base prompt (memory + scope + task + optional repo context)
    let mut base_prompt = build_prompt(work_unit, memory, read_namespaces, repo_map).await;

    // Prepend any relevant discoveries from other agents
    if let Some(b) = board {
        let discoveries = b.format_for_prompt(&work_unit.scope.owned_paths).await;
        if !discoveries.is_empty() {
            base_prompt = format!("{}\n\n{}", discoveries, base_prompt);
        }
    }

    // 3. Retry loop
    let max_attempts = (work_unit.max_retries as usize) + 1;
    // Union of all files written across all attempts (deduped)
    let mut all_modified: std::collections::HashSet<std::path::PathBuf> = Default::default();
    // Corrective suffix appended to the base prompt on retries
    let mut corrective: Option<String> = None;

    for attempt in 0..max_attempts {
        let prompt = match &corrective {
            None => base_prompt.clone(),
            Some(fix) => format!("{}\n\n{}", base_prompt, fix),
        };

        let request = CompletionRequest {
            prompt,
            system_prompt: None,
            workspace_root: workspace_root.to_path_buf(),
            allowed_tools: vec![
                "Read".into(),
                "Glob".into(),
                "Grep".into(),
                "Write".into(),
                "Edit".into(),
                "MultiEdit".into(),
            ],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
        };

        // Stream completion
        let stream_result = backend.stream_completion(request).await;
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                return Ok(AgentManifest {
                    work_unit_id: work_unit.id.clone(),
                    status: AgentStatus::Failed(e.to_string()),
                    modified_files: vec![],
                    branch: None,
                    summary: Some(format!("Backend error: {}", e)),
                    cost_usd: 0.0,
                });
            }
        };

        // Consume stream
        let mut attempt_modified: Vec<std::path::PathBuf> = Vec::new();
        let mut full_text = String::new();
        let mut had_error = false;
        let mut error_msg = String::new();
        let mut in_thinking = false;

        while let Some(event_result) = stream.next().await {
            let event = match event_result {
                Ok(ev) => ev,
                Err(e) => {
                    had_error = true;
                    error_msg = e.to_string();
                    break;
                }
            };

            match event {
                UnifiedStreamEvent::TextDelta(text) => {
                    if in_thinking {
                        observer.on_stream_chunk("\n</think>\n");
                        in_thinking = false;
                    }
                    full_text.push_str(&text);
                    observer.on_stream_chunk(&text);
                }
                UnifiedStreamEvent::ThinkingDelta(text) => {
                    if !in_thinking {
                        observer.on_stream_chunk("<think>\n");
                        in_thinking = true;
                    }
                    observer.on_stream_chunk(&text);
                }
                UnifiedStreamEvent::ToolCallStart { name, .. } => {
                    observer.on_tool_call_started(&name);
                    observer.on_streaming_status(&format!("Using {}...", name));
                }
                UnifiedStreamEvent::ToolCallDelta { .. } => {}
                UnifiedStreamEvent::ToolCallEnd { .. } => {}
                UnifiedStreamEvent::FileBlock { path, content } => {
                    match propose_write(&agent_id, &path, &content, workspace_root, &write_gate, observer)
                        .await
                    {
                        Ok(true) => {
                            attempt_modified.push(workspace_root.join(&path));
                        }
                        Ok(false) => {}
                        Err(e) => {
                            tracing::error!(
                                "Failed to create proposal for {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
                UnifiedStreamEvent::Usage(_) => {}
                UnifiedStreamEvent::Error(msg) => {
                    had_error = true;
                    error_msg = msg;
                }
                UnifiedStreamEvent::Done(_) => {
                    break;
                }
            }
        }

        if in_thinking {
            observer.on_stream_chunk("\n</think>\n");
        }

        if had_error {
            // Propagate hard errors immediately
            return Ok(AgentManifest {
                work_unit_id: work_unit.id.clone(),
                status: AgentStatus::Failed(error_msg.clone()),
                modified_files: all_modified.into_iter().collect(),
                branch: None,
                summary: Some(error_msg),
                cost_usd: 0.0,
            });
        }

        if !full_text.is_empty() {
            observer.on_message_complete("assistant", &full_text);

            // Parse and post discoveries to the shared board
            if let Some(b) = board {
                for entry in parse_discoveries(&work_unit.id, &full_text) {
                    b.post(entry).await;
                }
            }
        }

        all_modified.extend(attempt_modified.iter().cloned());

        // 4. Inline validation
        if let Some(vp) = validation {
            let files: Vec<std::path::PathBuf> = all_modified.iter().cloned().collect();
            if !files.is_empty() {
                let next_attempt = attempt + 1;
                let can_retry = next_attempt < max_attempts;

                let failure = vp
                    .run_reporting(&files, workspace_root, false, |gate, passed| {
                        observer.on_validation_result(gate, passed, None);
                    })
                    .await;

                if let Some((gate_name, result)) = failure {
                    let message = result.message().unwrap_or("validation failed").to_string();
                    observer.on_validation_result(gate_name, false, Some(&message));

                    if can_retry {
                        observer.on_validation_retry(next_attempt as u8, work_unit.max_retries);
                        // Build corrective prompt using the first failed file as context
                        let failed_file = files.first().map(|p| p.as_path())
                            .unwrap_or(std::path::Path::new("unknown"));
                        corrective = Some(crate::validation_gate::corrective_prompt(
                            gate_name,
                            failed_file,
                            &message,
                        ));
                        continue; // retry
                    }
                    // Exhausted retries — soft failure: agent output exists but is flagged
                    tracing::warn!(
                        "Agent {} exhausted retries ({} attempts), marking SoftFailure",
                        work_unit.id,
                        max_attempts
                    );
                    return Ok(AgentManifest {
                        work_unit_id: work_unit.id.clone(),
                        status: AgentStatus::Failed(format!(
                            "validation failed after {} retries: {}",
                            max_attempts, message
                        )),
                        modified_files: all_modified.into_iter().collect(),
                        branch: None,
                        summary: Some(format!("Validation failed ({}): {}", gate_name, message)),
                        cost_usd: 0.0,
                    });
                }
            }
        }

        // Validation passed (or no validator) — done
        return Ok(AgentManifest {
            work_unit_id: work_unit.id.clone(),
            status: AgentStatus::Completed,
            modified_files: all_modified.into_iter().collect(),
            branch: None,
            summary: Some(format!("Modified {} files", attempt_modified.len())),
            cost_usd: 0.0,
        });
    }

    // Should be unreachable (loop always returns), but provide a fallback
    Ok(AgentManifest {
        work_unit_id: work_unit.id.clone(),
        status: AgentStatus::Completed,
        modified_files: all_modified.into_iter().collect(),
        branch: None,
        summary: Some("completed".into()),
        cost_usd: 0.0,
    })
}

/// Build the base prompt (repo context + memory context + scope clause + task description).
async fn build_prompt(
    work_unit: &WorkUnit,
    memory: Option<&MemoryStore>,
    read_namespaces: &[String],
    repo_map: Option<&RepoMap>,
) -> String {
    let mut parts = Vec::new();

    // Repo map context (prepended first for maximum LLM attention)
    if let Some(rm) = repo_map {
        let ctx = rm.rank_for_agent(&work_unit.scope.owned_paths, 8_000);
        if !ctx.repo_outline.is_empty() {
            parts.push(ctx.repo_outline);
        }
    }

    if let Some(mem) = memory {
        let ctx = mem
            .search_context(read_namespaces, &work_unit.description, 5)
            .await;
        if !ctx.is_empty() {
            parts.push(ctx);
        }
    }

    let scope_clause = work_unit.scope.to_prompt_clause();
    if !scope_clause.is_empty() {
        parts.push(format!("[File scope]:\n{}", scope_clause));
    }

    let task_text = if work_unit.coordinator_instructions.is_empty() {
        &work_unit.description
    } else {
        &work_unit.coordinator_instructions
    };
    parts.push(task_text.clone());

    parts.join("\n\n")
}

/// Create a write proposal through the Write Gate.
///
/// Returns `Ok(true)` if a proposal was created, `Ok(false)` if skipped
/// (scope rejected, duplicate, unchanged content, empty diff).
async fn propose_write(
    agent_id: &str,
    rel_path: &Path,
    proposed_content: &str,
    workspace_root: &Path,
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &dyn AcpObserver,
) -> Result<bool> {
    let abs_path = workspace_root.join(rel_path);

    // 1. Scope check + duplicate check + allocate ID
    let (id, is_deferred) = {
        let mut gate = write_gate.lock().await;
        let path_str = rel_path.to_string_lossy();
        if !gate.is_scope_allowed(agent_id, &path_str) {
            tracing::warn!("Scope rejected for {}", rel_path.display());
            return Ok(false);
        }
        if gate.proposal_for_path(&abs_path).is_some() {
            return Ok(false);
        }
        if gate
            .pending_proposals()
            .iter()
            .any(|p| p.file_path == abs_path)
        {
            return Ok(false);
        }
        (gate.next_id(), gate.is_deferred())
    };

    // 2. Read original + build proposal (outside lock)
    let original = if abs_path.exists() {
        tokio::fs::read_to_string(&abs_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };

    if original == proposed_content {
        return Ok(false);
    }

    let proposal = WriteGatePipeline::build_proposal(
        id,
        agent_id,
        &abs_path,
        &original,
        proposed_content,
    );

    if proposal.structural_hunks.is_empty() {
        return Ok(false);
    }

    // 3. Insert proposal
    let auto_accept_result = {
        let mut gate = write_gate.lock().await;
        gate.insert_proposal(proposal)
    };

    // 4. Notify observer if deferred
    if is_deferred {
        let old = if original.is_empty() {
            None
        } else {
            Some(original.as_str())
        };
        observer.on_proposal_deferred(&abs_path, old, proposed_content);
    }

    // 5. Auto-accept: write to disk
    if let Some((path, content)) = auto_accept_result {
        tokio::fs::write(&path, &content)
            .await
            .map_err(|e| anyhow::anyhow!("writing auto-accepted file: {}", e))?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::AcpObserver;
    use crate::swarm::backend::mock::MockBackend;
    use crate::swarm::backend::{StopReason, UnifiedStreamEvent};
    use crate::types::FileScope;
    use crate::write_gate::{WriteGatePipeline, WriteMode};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A recording observer for tests.
    struct TestObserver {
        chunks: Mutex<Vec<String>>,
        message_complete: AtomicBool,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                chunks: Mutex::new(Vec::new()),
                message_complete: AtomicBool::new(false),
            }
        }

        async fn chunks(&self) -> Vec<String> {
            self.chunks.lock().await.clone()
        }
    }

    impl AcpObserver for TestObserver {
        fn on_stream_chunk(&self, text: &str) {
            // Use try_lock for sync context
            if let Ok(mut chunks) = self.chunks.try_lock() {
                chunks.push(text.to_string());
            }
        }
        fn on_tool_call_started(&self, _name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {
            self.message_complete.store(true, Ordering::Relaxed);
        }
        fn on_proposal_deferred(
            &self,
            _path: &Path,
            _old_content: Option<&str>,
            _new_content: &str,
        ) {
        }
    }

    fn test_work_unit() -> WorkUnit {
        WorkUnit {
            id: "test-unit".into(),
            description: "test task".into(),
            scope: FileScope {
                owned_paths: vec!["src/".into()],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            backend: Default::default(),
            model: None,
            tier: crate::types::ModelTier::Cheap,
            privacy: crate::types::PrivacyLevel::Public,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
        }
    }

    struct NoopWriteGateObserver;
    impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
        fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
        fn on_proposal_updated(&self, _proposal_id: u64) {}
        fn on_proposal_finalized(&self, _path: &str) {}
    }

    // Test 17: Trait runner success path
    #[tokio::test]
    async fn test_run_backend_success() {
        let events = vec![
            UnifiedStreamEvent::TextDelta("hello ".into()),
            UnifiedStreamEvent::TextDelta("world".into()),
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ];
        let backend = MockBackend::new("test", events);
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWriteGateObserver),
        )));
        let observer = TestObserver::new();
        let unit = test_work_unit();

        let manifest = run_backend(
            &backend,
            &unit,
            write_gate,
            Path::new("/tmp/workspace"),
            None,
            &["default".to_string()],
            &observer,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(matches!(manifest.status, AgentStatus::Completed));
        assert!(observer.message_complete.load(Ordering::Relaxed));

        let chunks = observer.chunks().await;
        assert_eq!(chunks, vec!["hello ", "world"]);
    }

    // Test 18: Trait runner failure path
    #[tokio::test]
    async fn test_run_backend_failure() {
        let events = vec![
            UnifiedStreamEvent::Error("model unavailable".into()),
            UnifiedStreamEvent::Done(StopReason::Error),
        ];
        let backend = MockBackend::new("test", events);
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWriteGateObserver),
        )));
        let observer = TestObserver::new();
        let unit = test_work_unit();

        let manifest = run_backend(
            &backend,
            &unit,
            write_gate,
            Path::new("/tmp/workspace"),
            None,
            &["default".to_string()],
            &observer,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(matches!(manifest.status, AgentStatus::Failed(_)));
        if let AgentStatus::Failed(msg) = &manifest.status {
            assert!(msg.contains("model unavailable"));
        }
    }

    // Test 19: FileBlock triggers write gate proposal
    #[tokio::test]
    async fn test_run_backend_file_block_creates_proposal() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();

        // Create the src/ directory and a file to propose changes to
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/main.rs"), "fn main() {}").unwrap();

        let events = vec![
            UnifiedStreamEvent::FileBlock {
                path: PathBuf::from("src/main.rs"),
                content: "fn main() { println!(\"hello\"); }".into(),
            },
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ];
        let backend = MockBackend::new("test", events);
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::Deferred,
            Box::new(NoopWriteGateObserver),
        )));
        let observer = TestObserver::new();
        let unit = test_work_unit();

        let manifest = run_backend(
            &backend,
            &unit,
            write_gate.clone(),
            workspace,
            None,
            &["default".to_string()],
            &observer,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(matches!(manifest.status, AgentStatus::Completed));
        assert_eq!(manifest.modified_files.len(), 1);

        // Verify proposal exists in write gate
        let gate = write_gate.lock().await;
        let proposals = gate.pending_proposals();
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].file_path.ends_with("src/main.rs"));
    }

    // Test 20: Scope enforcement — out-of-scope FileBlock rejected
    #[tokio::test]
    async fn test_run_backend_scope_enforcement() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();

        std::fs::create_dir_all(workspace.join("tests")).unwrap();
        std::fs::write(workspace.join("tests/foo.rs"), "// test").unwrap();

        let events = vec![
            UnifiedStreamEvent::FileBlock {
                path: PathBuf::from("tests/foo.rs"),
                content: "// modified test".into(),
            },
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ];
        let backend = MockBackend::new("test", events);
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWriteGateObserver),
        )));
        let observer = TestObserver::new();
        // WorkUnit scope is restricted to src/ — tests/ is out of scope
        let unit = test_work_unit();

        let manifest = run_backend(
            &backend,
            &unit,
            write_gate,
            workspace,
            None,
            &["default".to_string()],
            &observer,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Should complete but with no modified files (scope rejected)
        assert!(matches!(manifest.status, AgentStatus::Completed));
        assert_eq!(manifest.modified_files.len(), 0);
    }

    // Test 22: Stream contract — runner always terminates even with just Done
    #[tokio::test]
    async fn test_run_backend_terminates_on_done() {
        let events = vec![UnifiedStreamEvent::Done(StopReason::EndTurn)];
        let backend = MockBackend::new("test", events);
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWriteGateObserver),
        )));
        let observer = TestObserver::new();
        let unit = test_work_unit();

        let manifest = run_backend(
            &backend,
            &unit,
            write_gate,
            Path::new("/tmp"),
            None,
            &["default".to_string()],
            &observer,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(matches!(manifest.status, AgentStatus::Completed));
    }
}
