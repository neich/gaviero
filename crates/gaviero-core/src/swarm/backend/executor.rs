use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::observer::AcpObserver;
use crate::write_gate::WriteGatePipeline;

use super::runner::propose_write;
use super::{AgentBackend, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent};

#[derive(Debug, Default)]
pub struct CompletionOutcome {
    pub text: String,
    pub modified_files: Vec<PathBuf>,
    pub usage: Option<TokenUsage>,
}

pub async fn complete_to_text(
    backend: &dyn AgentBackend,
    request: CompletionRequest,
    observer: Option<&dyn AcpObserver>,
) -> Result<CompletionOutcome> {
    validate_request(backend, &request)?;
    // M0 instrumentation: log prompt size at dispatch so we know what the
    // backend saw vs. what the planner will emit in later milestones.
    tracing::info!(
        target: "turn_metrics",
        backend = backend.name(),
        prompt_chars = request.prompt.len(),
        history_entries = request.conversation_history.len(),
        file_refs = request.file_refs.len(),
        file_attachments = request.file_attachments.len(),
        "backend_dispatch"
    );
    let backend_name = backend.name().to_string();
    let mut stream = backend.stream_completion(request).await?;
    let mut outcome = CompletionOutcome::default();
    let mut in_thinking = false;
    let mut error: Option<String> = None;
    let mut read_count: usize = 0;

    while let Some(event_result) = stream.next().await {
        let event = event_result?;
        match event {
            UnifiedStreamEvent::TextDelta(text) => {
                if in_thinking {
                    if let Some(obs) = observer {
                        obs.on_stream_chunk("\n</think>\n");
                    }
                    in_thinking = false;
                }
                if let Some(obs) = observer {
                    obs.on_stream_chunk(&text);
                }
                outcome.text.push_str(&text);
            }
            UnifiedStreamEvent::ThinkingDelta(text) => {
                if !in_thinking {
                    if let Some(obs) = observer {
                        obs.on_stream_chunk("<think>\n");
                    }
                    in_thinking = true;
                }
                if let Some(obs) = observer {
                    obs.on_stream_chunk(&text);
                }
            }
            UnifiedStreamEvent::ToolCallStart { name, .. } => {
                if name == "Read" {
                    read_count += 1;
                }
                if let Some(obs) = observer {
                    obs.on_tool_call_started(&name);
                    obs.on_streaming_status(&format!("Using {}...", name));
                }
            }
            UnifiedStreamEvent::ToolCallDelta { .. } => {}
            UnifiedStreamEvent::ToolCallEnd { .. } => {}
            UnifiedStreamEvent::FileBlock { .. } => {}
            UnifiedStreamEvent::Usage(usage) => {
                // M0 instrumentation: log provider-reported token usage.
                tracing::info!(
                    target: "turn_metrics",
                    backend = %backend_name,
                    input_tokens = usage.input_tokens,
                    output_tokens = usage.output_tokens,
                    duration_ms = ?usage.duration_ms,
                    "token_usage"
                );
                outcome.usage = Some(usage);
            }
            UnifiedStreamEvent::Error(msg) => {
                error = Some(msg);
            }
            UnifiedStreamEvent::Done(StopReason::EndTurn)
            | UnifiedStreamEvent::Done(StopReason::ToolUse)
            | UnifiedStreamEvent::Done(StopReason::Timeout)
            | UnifiedStreamEvent::Done(StopReason::Error) => break,
        }
    }

    if in_thinking {
        if let Some(obs) = observer {
            obs.on_stream_chunk("\n</think>\n");
        }
    }

    // M0 instrumentation: emit per-turn Read tool count.
    tracing::info!(
        target: "turn_metrics",
        backend = %backend_name,
        read_count,
        "turn_read_count"
    );

    if let Some(obs) = observer {
        obs.on_message_complete("assistant", &outcome.text);
    }

    if let Some(msg) = error {
        anyhow::bail!(msg);
    }

    Ok(outcome)
}

pub async fn complete_to_write_gate(
    backend: &dyn AgentBackend,
    request: CompletionRequest,
    observer: &dyn AcpObserver,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    agent_id: &str,
    conv_id: Option<&str>,
) -> Result<CompletionOutcome> {
    validate_request(backend, &request)?;
    // M0 instrumentation: log prompt size at dispatch.
    tracing::info!(
        target: "turn_metrics",
        backend = backend.name(),
        agent_id,
        prompt_chars = request.prompt.len(),
        history_entries = request.conversation_history.len(),
        file_refs = request.file_refs.len(),
        file_attachments = request.file_attachments.len(),
        "backend_dispatch"
    );
    let backend_name = backend.name().to_string();
    let in_band_file_blocks = backend.capabilities().supports_file_blocks;
    let workspace_root = request.workspace_root.clone();
    let mut stream = backend.stream_completion(request).await?;
    let mut modified_files = HashSet::new();
    let mut outcome = CompletionOutcome::default();
    let mut in_thinking = false;
    let mut error: Option<String> = None;
    let mut read_count: usize = 0;

    while let Some(event_result) = stream.next().await {
        let event = event_result?;
        match event {
            UnifiedStreamEvent::TextDelta(text) => {
                if in_thinking {
                    observer.on_stream_chunk("\n</think>\n");
                    in_thinking = false;
                }
                observer.on_stream_chunk(&text);
                outcome.text.push_str(&text);
            }
            UnifiedStreamEvent::ThinkingDelta(text) => {
                if !in_thinking {
                    observer.on_stream_chunk("<think>\n");
                    in_thinking = true;
                }
                observer.on_stream_chunk(&text);
            }
            UnifiedStreamEvent::ToolCallStart { name, .. } => {
                if name == "Read" {
                    read_count += 1;
                }
                observer.on_tool_call_started(&name);
                observer.on_streaming_status(&format!("Using {}...", name));
            }
            UnifiedStreamEvent::ToolCallDelta { .. } => {}
            UnifiedStreamEvent::ToolCallEnd { .. } => {}
            UnifiedStreamEvent::FileBlock { path, content } => {
                // Capability invariant: backends that advertise
                // `supports_file_blocks=false` must not emit FileBlock events.
                // This guards against a future regression silently re-enabling
                // the in-band parser path that produced the "garbled chat →
                // proposal" incidents on the Claude backend.
                debug_assert!(
                    in_band_file_blocks,
                    "backend '{}' advertises supports_file_blocks=false but emitted a FileBlock for {}",
                    backend_name,
                    path.display()
                );
                if !in_band_file_blocks {
                    tracing::error!(
                        "Dropping FileBlock from backend '{}' for {} — backend declared supports_file_blocks=false. \
                         This is a backend bug; tool calls are the only edit channel.",
                        backend_name,
                        path.display()
                    );
                    continue;
                }
                if propose_write(
                    agent_id,
                    conv_id.or(Some(agent_id)),
                    &path,
                    &content,
                    &workspace_root,
                    &write_gate,
                    observer,
                )
                .await?
                {
                    modified_files.insert(workspace_root.join(&path));
                }
            }
            UnifiedStreamEvent::Usage(usage) => {
                // M0 instrumentation: log provider-reported token usage.
                tracing::info!(
                    target: "turn_metrics",
                    backend = %backend_name,
                    agent_id,
                    input_tokens = usage.input_tokens,
                    output_tokens = usage.output_tokens,
                    duration_ms = ?usage.duration_ms,
                    "token_usage"
                );
                outcome.usage = Some(usage);
            }
            UnifiedStreamEvent::Error(msg) => {
                error = Some(msg);
            }
            UnifiedStreamEvent::Done(StopReason::EndTurn)
            | UnifiedStreamEvent::Done(StopReason::ToolUse)
            | UnifiedStreamEvent::Done(StopReason::Timeout)
            | UnifiedStreamEvent::Done(StopReason::Error) => break,
        }
    }

    if in_thinking {
        observer.on_stream_chunk("\n</think>\n");
    }

    // M0 instrumentation: emit per-turn Read tool count.
    tracing::info!(
        target: "turn_metrics",
        backend = %backend_name,
        agent_id,
        read_count,
        "turn_read_count"
    );

    observer.on_message_complete("assistant", &outcome.text);
    outcome.modified_files = modified_files.into_iter().collect();

    if let Some(msg) = error {
        anyhow::bail!(msg);
    }

    Ok(outcome)
}

fn validate_request(backend: &dyn AgentBackend, request: &CompletionRequest) -> Result<()> {
    let caps = backend.capabilities();

    if !caps.supports_system_prompt
        && request
            .system_prompt
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    {
        anyhow::bail!(
            "backend '{}' does not support system prompts",
            backend.name()
        );
    }

    if !caps.vision && !request.file_attachments.is_empty() {
        anyhow::bail!(
            "backend '{}' does not support file attachments",
            backend.name()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{AcpObserver, WriteGateObserver};
    use crate::swarm::backend::Capabilities;
    use crate::swarm::backend::mock::MockBackend;
    use crate::types::WriteProposal;
    use crate::write_gate::WriteMode;

    struct NoopObserver;

    impl AcpObserver for NoopObserver {
        fn on_stream_chunk(&self, _text: &str) {}
        fn on_tool_call_started(&self, _tool_name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {}
        fn on_proposal_deferred(
            &self,
            _path: &std::path::Path,
            _old_content: Option<&str>,
            _new_content: &str,
        ) {
        }
    }

    struct NoopWriteGateObserver;
    impl WriteGateObserver for NoopWriteGateObserver {
        fn on_proposal_created(&self, _proposal: &WriteProposal) {}
        fn on_proposal_updated(&self, _proposal_id: u64) {}
        fn on_proposal_finalized(&self, _path: &str) {}
    }

    #[tokio::test]
    async fn test_complete_to_text_collects_stream() {
        let backend = MockBackend::new(
            "mock",
            vec![
                UnifiedStreamEvent::TextDelta("hello".into()),
                UnifiedStreamEvent::TextDelta(" world".into()),
                UnifiedStreamEvent::Done(StopReason::EndTurn),
            ],
        );
        let request = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        };

        let outcome = complete_to_text(&backend, request, Some(&NoopObserver))
            .await
            .unwrap();
        assert_eq!(outcome.text, "hello world");
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    #[should_panic(expected = "supports_file_blocks=false")]
    async fn fileblock_from_capability_disabled_backend_trips_debug_assert() {
        // A backend that declares `supports_file_blocks=false` MUST NOT emit
        // FileBlock events. If it does, the executor trips a debug_assert
        // (and in release silently drops + logs). This test pins that the
        // guard is in place — without it, the in-band parser path that bit
        // the Claude backend could be silently re-enabled by a future regression.
        let backend = MockBackend::new(
            "no-fileblocks",
            vec![
                UnifiedStreamEvent::FileBlock {
                    path: PathBuf::from("foo.rs"),
                    content: "fn x() {}".into(),
                },
                UnifiedStreamEvent::Done(StopReason::EndTurn),
            ],
        )
        .with_capabilities(Capabilities {
            supports_file_blocks: false,
            ..Capabilities::default()
        });

        let request = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        };

        let gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::RejectAll,
            Box::new(NoopWriteGateObserver),
        )));
        let _ = complete_to_write_gate(&backend, request, &NoopObserver, gate, "agent-x", None).await;
    }

    #[tokio::test]
    async fn fileblock_handled_when_capability_enabled() {
        // Sanity counter-test: with `supports_file_blocks=true` the executor
        // routes the FileBlock through propose_write without panicking.
        let backend = MockBackend::new(
            "supports-fileblocks",
            vec![
                UnifiedStreamEvent::FileBlock {
                    path: PathBuf::from("foo.rs"),
                    content: "fn x() {}".into(),
                },
                UnifiedStreamEvent::Done(StopReason::EndTurn),
            ],
        )
        .with_capabilities(Capabilities {
            supports_file_blocks: true,
            ..Capabilities::default()
        });

        let workspace = tempfile::tempdir().unwrap();
        let request = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: workspace.path().to_path_buf(),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        };

        // RejectAll mode: gate discards the proposal (no disk write), but
        // propose_write still returns Ok(true) once the proposal exists, so
        // the FileBlock arm of the executor records the modified path. The
        // assertion is "we got here without panicking" — this is the
        // counter-test to `..._trips_debug_assert` above.
        let gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::RejectAll,
            Box::new(NoopWriteGateObserver),
        )));
        let outcome = complete_to_write_gate(&backend, request, &NoopObserver, gate, "agent-x", None)
            .await
            .unwrap();
        assert_eq!(outcome.modified_files.len(), 1);
    }

    #[tokio::test]
    async fn test_complete_to_text_surfaces_errors() {
        let backend = MockBackend::new(
            "mock",
            vec![
                UnifiedStreamEvent::Error("boom".into()),
                UnifiedStreamEvent::Done(StopReason::Error),
            ],
        );
        let request = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        };

        let err = complete_to_text(&backend, request, Some(&NoopObserver))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }
}
