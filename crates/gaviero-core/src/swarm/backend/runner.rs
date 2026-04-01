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
use crate::write_gate::WriteGatePipeline;

use super::super::models::{AgentManifest, AgentStatus, WorkUnit};
use super::{AgentBackend, CompletionRequest, UnifiedStreamEvent};

/// Run a work unit through any `AgentBackend`, producing an `AgentManifest`.
///
/// This consolidates the logic from the old `AgentRunner::run_with_model()`
/// and `AgentRunner::run_ollama()` into a single code path.
pub async fn run_backend(
    backend: &dyn AgentBackend,
    work_unit: &WorkUnit,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    workspace_root: &Path,
    memory: Option<&MemoryStore>,
    read_namespaces: &[String],
    observer: &dyn AcpObserver,
) -> Result<AgentManifest> {
    let agent_id = format!("agent-{}", work_unit.id);

    // 1. Register scope with write gate
    {
        let mut gate = write_gate.lock().await;
        gate.register_agent_scope(&agent_id, &work_unit.scope);
    }

    // 2. Build prompt (memory + scope + task)
    let mut prompt_parts = Vec::new();

    if let Some(mem) = memory {
        let ctx = mem
            .search_context(read_namespaces, &work_unit.description, 5)
            .await;
        if !ctx.is_empty() {
            prompt_parts.push(ctx);
        }
    }

    let scope_clause = work_unit.scope.to_prompt_clause();
    if !scope_clause.is_empty() {
        prompt_parts.push(format!("[File scope]:\n{}", scope_clause));
    }

    let task_text = if work_unit.coordinator_instructions.is_empty() {
        &work_unit.description
    } else {
        &work_unit.coordinator_instructions
    };
    prompt_parts.push(task_text.clone());

    let full_prompt = prompt_parts.join("\n\n");

    let request = CompletionRequest {
        prompt: full_prompt,
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

    // 3. Stream completion
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
            });
        }
    };

    // 4. Consume stream
    let mut modified_files = Vec::new();
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
            UnifiedStreamEvent::ToolCallDelta { .. } => {
                // Tool input fragments — ignored
            }
            UnifiedStreamEvent::ToolCallEnd { .. } => {
                // Tool call complete
            }
            UnifiedStreamEvent::FileBlock { path, content } => {
                match propose_write(&agent_id, &path, &content, workspace_root, &write_gate, observer)
                    .await
                {
                    Ok(true) => {
                        modified_files.push(workspace_root.join(&path));
                    }
                    Ok(false) => {
                        // Proposal was skipped (scope rejected, duplicate, or unchanged)
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to create proposal for {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
            UnifiedStreamEvent::Usage(_) => {
                // Token usage — could be logged/tracked in the future
            }
            UnifiedStreamEvent::Error(msg) => {
                had_error = true;
                error_msg = msg;
            }
            UnifiedStreamEvent::Done(_) => {
                break;
            }
        }
    }

    // Close any open thinking block
    if in_thinking {
        observer.on_stream_chunk("\n</think>\n");
    }

    // 5. Build manifest
    let (status, summary) = if had_error {
        (
            AgentStatus::Failed(error_msg.clone()),
            Some(error_msg),
        )
    } else {
        if !full_text.is_empty() {
            observer.on_message_complete("assistant", &full_text);
        }
        (
            AgentStatus::Completed,
            Some(format!("Modified {} files", modified_files.len())),
        )
    };

    Ok(AgentManifest {
        work_unit_id: work_unit.id.clone(),
        status,
        modified_files,
        branch: None,
        summary,
    })
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
            tier: crate::types::ModelTier::Execution,
            privacy: crate::types::PrivacyLevel::Public,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
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
        )
        .await
        .unwrap();

        assert!(matches!(manifest.status, AgentStatus::Completed));
    }
}
