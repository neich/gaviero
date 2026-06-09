//! In-process DeepSeek tool-agent backend for swarm work units (Unit 17–18).
//!
//! Unlike stream-only backends, this runs the full multi-round tool loop
//! inside `stream_completion` and emits normalized events on the unified
//! stream. File edits are Option-B direct writes scoped to the work unit.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::agent_session::tool_agent::swarm::{SwarmTurnRequest, run_turn};
use crate::observer::AcpObserver;

use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
};

/// Swarm backend that delegates to the in-process tool-agent harness.
pub struct DeepseekBackend {
    model: String,
    display_name: String,
}

impl DeepseekBackend {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            display_name: format!("deepseek:{}", model),
        }
    }

    fn capabilities_for_swarm() -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
            extended_thinking: true,
            max_context_tokens: 128_000,
            supports_system_prompt: true,
            supports_file_blocks: false,
        }
    }
}

/// Bridges harness observer callbacks into [`UnifiedStreamEvent`]s on a channel.
struct StreamBridge {
    tx: mpsc::Sender<Result<UnifiedStreamEvent>>,
}

impl AcpObserver for StreamBridge {
    fn on_stream_chunk(&self, text: &str) {
        let _ = self
            .tx
            .blocking_send(Ok(UnifiedStreamEvent::TextDelta(text.to_string())));
    }

    fn on_tool_call_started(&self, summary: &str) {
        let name = summary.split_whitespace().next().unwrap_or("tool").to_string();
        let _ = self.tx.blocking_send(Ok(UnifiedStreamEvent::ToolCallStart {
            id: String::new(),
            name,
            args: Value::Null,
        }));
    }

    fn on_streaming_status(&self, _status: &str) {}

    fn on_message_complete(&self, _role: &str, _content: &str) {}

    fn on_proposal_deferred(&self, _path: &Path, _old_content: Option<&str>, _new_content: &str) {}

    fn on_turn_token_usage(&self, usage: &crate::acp::protocol::TokenUsage) {
        let _ = self.tx.blocking_send(Ok(UnifiedStreamEvent::Usage(TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: None,
            duration_ms: None,
        })));
    }
}

#[async_trait::async_trait]
impl AgentBackend for DeepseekBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let system = request
            .system_prompt
            .unwrap_or_else(|| super::shared::default_editor_system_prompt(&Self::capabilities_for_swarm()));
        let (tx, rx) = mpsc::channel::<Result<UnifiedStreamEvent>>(256);
        let bridge = Arc::new(StreamBridge { tx: tx.clone() });
        let model = self.model.clone();
        let cancel = CancellationToken::new();

        let swarm_req = SwarmTurnRequest {
            model,
            workspace_root: request.workspace_root.clone(),
            additional_roots: request.additional_roots,
            scope: request.file_scope,
            system_prompt: system,
            user_prompt: request.prompt,
            allowed_tools: request.allowed_tools,
            auto_approve: request.auto_approve,
        };

        tokio::spawn(async move {
            let outcome = run_turn(swarm_req, bridge.as_ref(), &cancel).await;

            if !outcome.modified_paths.is_empty() {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::PathsModified(
                        outcome.modified_paths.clone(),
                    )))
                    .await;
            }

            if outcome.total_cost_usd > 0.0 {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Usage(TokenUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: Some(outcome.total_cost_usd),
                        duration_ms: None,
                    })))
                    .await;
            }

            if let Some(err) = outcome.error {
                let _ = tx.send(Ok(UnifiedStreamEvent::Error(err))).await;
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                    .await;
            } else {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)))
                    .await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn capabilities(&self) -> Capabilities {
        Self::capabilities_for_swarm()
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        // Key resolution is lazy; a missing key surfaces on the first turn.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_includes_model() {
        let backend = DeepseekBackend::new("deepseek-v4-pro");
        assert_eq!(backend.name(), "deepseek:deepseek-v4-pro");
    }

    #[test]
    fn capabilities_match_tool_agent() {
        let caps = DeepseekBackend::capabilities_for_swarm();
        assert!(caps.tool_use);
        assert!(!caps.supports_file_blocks);
    }
}
