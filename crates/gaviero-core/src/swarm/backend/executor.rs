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
                if propose_write(
                    agent_id,
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
    use crate::observer::AcpObserver;
    use crate::swarm::backend::mock::MockBackend;

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
