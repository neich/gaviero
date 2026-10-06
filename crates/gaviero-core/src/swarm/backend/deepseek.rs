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
    AgentBackend, Capabilities, CompletionRequest, RetrievalToolset, StopReason, TokenUsage,
    UnifiedStreamEvent,
};

/// DeepSeek's thinking-mode effort when gaviero names no level.
///
/// The API's own default (see <https://api-docs.deepseek.com/guides/thinking_mode>),
/// pinned explicitly so every request carries a level instead of inheriting
/// whatever the model or account default happens to be.
pub const DEFAULT_DEEPSEEK_REASONING_EFFORT: &str = "high";

/// Map gaviero's provider-neutral `effort` onto DeepSeek's `reasoning_effort`
/// vocabulary.
///
/// DeepSeek's chat API accepts `low | high | max`, and maps a *requested* effort
/// with the table from <https://api-docs.deepseek.com/guides/thinking_mode>
/// (minimal→low, low→low, medium→high, high→high, xhigh→high, max→max,
/// ultra→max). The dsh DeepSeek adapter advertises the same thinking levels as
/// its ACP `reasoning_effort` select (`low | high | max`, plus `off`), so both
/// DeepSeek paths share this one mapping.
///
/// `None` means "gaviero names no level" — `off`/`auto`/unknown are deliberately
/// **not** forwarded as DeepSeek's `off`, which would disable thinking. Callers
/// substitute [`DEFAULT_DEEPSEEK_REASONING_EFFORT`].
pub fn reasoning_effort_for_deepseek(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => Some("low"),
        "medium" | "high" | "xhigh" => Some("high"),
        "max" | "ultra" => Some("max"),
        _ => None,
    }
}

/// The `reasoning_effort` to send on the DeepSeek chat API.
///
/// Thinking mode is always on for gaviero, so the mapping never yields `off`:
/// an explicit level maps onto `low|high|max`, and anything else
/// (`off`/`auto`/unset/unknown) resolves to `high`. Sending a level that the
/// model cannot do would be a hard API error, which is why unknown values fall
/// back rather than being forwarded verbatim.
pub fn deepseek_reasoning_effort(effort: Option<&str>) -> &'static str {
    match effort.and_then(reasoning_effort_for_deepseek) {
        Some(level) => level,
        None => DEFAULT_DEEPSEEK_REASONING_EFFORT,
    }
}

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
            // DeepSeek's in-process tool agent exposes Read/Grep/Glob/Bash/Write,
            // not the gaviero MCP retrieval tools → no pull stanza.
            retrieval: RetrievalToolset::default(),
        }
    }
}

/// Bridges harness observer callbacks into [`UnifiedStreamEvent`]s on a channel.
struct StreamBridge {
    tx: mpsc::Sender<Result<UnifiedStreamEvent>>,
}

impl StreamBridge {
    /// Observer callbacks run on the tokio runtime thread inside `run_turn`;
    /// `blocking_send` would panic there, so schedule an async send instead.
    fn emit(&self, event: Result<UnifiedStreamEvent>) {
        let tx = self.tx.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = tx.send(event).await;
                });
            }
            Err(_) => {
                let _ = tx.try_send(event);
            }
        }
    }
}

impl AcpObserver for StreamBridge {
    fn on_stream_chunk(&self, text: &str) {
        self.emit(Ok(UnifiedStreamEvent::TextDelta(text.to_string())));
    }

    fn on_tool_call_started(&self, summary: &str) {
        let name = summary
            .split_whitespace()
            .next()
            .unwrap_or("tool")
            .to_string();
        self.emit(Ok(UnifiedStreamEvent::ToolCallStart {
            id: String::new(),
            name,
            args: Value::Null,
        }));
    }

    fn on_streaming_status(&self, _status: &str) {}

    fn on_message_complete(&self, _role: &str, _content: &str) {}

    fn on_proposal_deferred(&self, _path: &Path, _old_content: Option<&str>, _new_content: &str) {}

    fn on_turn_token_usage(&self, usage: &crate::acp::protocol::TokenUsage) {
        self.emit(Ok(UnifiedStreamEvent::Usage(TokenUsage {
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
        let exposed = request.exposed_tools.clone();
        let system = request.system_prompt.unwrap_or_else(|| {
            super::shared::default_editor_system_prompt(
                &Self::capabilities_for_swarm().with_exposed_tools(exposed.as_deref()),
            )
        });
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
            tool_policy: request.tool_policy,
            effort: request.effort,
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

    /// The exact table from the DeepSeek thinking-mode guide, shared by the
    /// dsh ACP path and the in-process chat client.
    #[test]
    fn effort_vocabulary_maps_onto_deepseek_levels() {
        for (requested, expected) in [
            ("minimal", Some("low")),
            ("low", Some("low")),
            ("medium", Some("high")),
            ("high", Some("high")),
            ("xhigh", Some("high")),
            ("max", Some("max")),
            ("ultra", Some("max")),
        ] {
            assert_eq!(
                reasoning_effort_for_deepseek(requested),
                expected,
                "requested {requested}"
            );
        }
        // Case/whitespace tolerant, like the other providers' effort maps.
        assert_eq!(reasoning_effort_for_deepseek(" ULTRA "), Some("max"));
        assert_eq!(reasoning_effort_for_deepseek("Medium"), Some("high"));
    }

    #[test]
    fn off_and_auto_never_map_to_thinking_off() {
        // Thinking mode is always on, so `off`/`auto`/unknown must not become
        // DeepSeek's `off` reasoning value.
        for requested in ["off", "auto", "", "   ", "OFF", "AUTO", "unknown"] {
            assert_eq!(reasoning_effort_for_deepseek(requested), None, "{requested:?}");
        }
    }

    #[test]
    fn unset_effort_resolves_to_the_thinking_default() {
        assert_eq!(deepseek_reasoning_effort(Some("max")), "max");
        assert_eq!(deepseek_reasoning_effort(Some("minimal")), "low");
        for unset in [None, Some("off"), Some("auto"), Some(""), Some("turbo")] {
            assert_eq!(
                deepseek_reasoning_effort(unset),
                DEFAULT_DEEPSEEK_REASONING_EFFORT,
                "{unset:?} must keep thinking on at the default level"
            );
        }
        assert_eq!(DEFAULT_DEEPSEEK_REASONING_EFFORT, "high");
    }
}
