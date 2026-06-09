//! The multi-round in-process agent loop (DeepSeek plan Unit 6).
//!
//! Each round: send the message array + tool schemas → stream the reply
//! (text/reasoning to the observer, tool calls collected) → if the model
//! requested tools, execute them in-process, append the assistant tool-call
//! message and one `tool` result message per call, and loop; otherwise the
//! round's text is the final answer.
//!
//! Two hard caps bound a runaway model: `max_rounds` and an optional
//! `cost_ceiling_usd` (accumulated from per-round [`crate::swarm::backend::TokenUsage::cost_usd`]).

use futures::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::observer::AcpObserver;

use super::tools::{ToolCtx, ToolRegistry};
use super::{ApiClient, ApiEvent, ApiRequest, ToolCall};

/// Runaway bounds for one turn.
pub(crate) struct LoopLimits {
    pub max_rounds: u32,
    pub cost_ceiling_usd: Option<f64>,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_rounds: 40,
            cost_ceiling_usd: None,
        }
    }
}

/// Result of a turn. `visible` is the accumulated assistant-visible text (for
/// `on_message_complete`); `error` is `Some` when the turn failed (the partial
/// `visible` is still returned, matching `ObservedStreamSession`).
pub(crate) struct LoopOutcome {
    pub visible: String,
    pub error: Option<String>,
    pub total_cost_usd: f64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_agent_loop(
    client: &dyn ApiClient,
    tools: &ToolRegistry,
    ctx: &ToolCtx,
    observer: &dyn AcpObserver,
    model: &str,
    mut messages: Vec<Value>,
    limits: &LoopLimits,
    cancel: &CancellationToken,
) -> LoopOutcome {
    let schemas = tools.schemas();
    let mut visible = String::new();
    let mut in_thinking = false;
    let mut total_cost = 0.0_f64;

    for _round in 0..limits.max_rounds {
        if cancel.is_cancelled() {
            return LoopOutcome {
                visible,
                error: Some("turn cancelled".to_string()),
                total_cost_usd: total_cost,
            };
        }

        let request = ApiRequest {
            model: model.to_string(),
            messages: messages.clone(),
            tools: schemas.clone(),
            max_tokens: None,
        };
        let mut stream = match client.complete(request).await {
            Ok(s) => s,
            Err(e) => {
                return LoopOutcome {
                    visible,
                    error: Some(format!("{e:#}")),
                    total_cost_usd: total_cost,
                };
            }
        };

        let mut round_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut round_err: Option<String> = None;

        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => {
                    return LoopOutcome {
                        visible,
                        error: Some("turn cancelled".to_string()),
                        total_cost_usd: total_cost,
                    };
                }
                e = stream.next() => {
                    match e {
                        None => break,
                        Some(ev) => ev,
                    }
                }
            };

            match event {
                Ok(ApiEvent::Text(t)) => {
                    if in_thinking {
                        observer.on_stream_chunk("\n</think>\n");
                        in_thinking = false;
                    }
                    observer.on_stream_chunk(&t);
                    round_text.push_str(&t);
                    visible.push_str(&t);
                }
                Ok(ApiEvent::Reasoning(t)) => {
                    if !in_thinking {
                        observer.on_stream_chunk("<think>\n");
                        in_thinking = true;
                    }
                    observer.on_stream_chunk(&t);
                }
                Ok(ApiEvent::ToolCall(call)) => tool_calls.push(call),
                Ok(ApiEvent::Usage(usage)) => {
                    if let Some(c) = usage.cost_usd {
                        total_cost += c;
                    }
                    observer.on_turn_token_usage(&crate::acp::protocol::TokenUsage {
                        input_tokens: usage.input_tokens,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                        output_tokens: usage.output_tokens,
                    });
                }
                Ok(ApiEvent::Done(_)) => break,
                Ok(ApiEvent::Error(m)) => round_err = Some(m),
                Err(e) => {
                    round_err = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        if in_thinking {
            observer.on_stream_chunk("\n</think>\n");
            in_thinking = false;
        }
        if let Some(e) = round_err {
            return LoopOutcome {
                visible,
                error: Some(e),
                total_cost_usd: total_cost,
            };
        }

        if tool_calls.is_empty() {
            return LoopOutcome {
                visible,
                error: None,
                total_cost_usd: total_cost,
            };
        }

        messages.push(assistant_tool_call_msg(&round_text, &tool_calls));
        for call in &tool_calls {
            if cancel.is_cancelled() {
                return LoopOutcome {
                    visible,
                    error: Some("turn cancelled".to_string()),
                    total_cost_usd: total_cost,
                };
            }
            let summary =
                crate::acp::client::format_tool_summary(&call.name, &call.args, &ctx.workspace_root);
            observer.on_tool_call_started(&summary);
            observer
                .on_streaming_status(&format!("Using {}...", call.name));

            let content = match tools.get(&call.name) {
                Some(tool) => tool.run(call.args.clone(), ctx).await.content,
                None => format!("Error: unknown tool '{}'", call.name),
            };
            messages.push(tool_result_msg(&call.id, &content));
        }

        if let Some(ceiling) = limits.cost_ceiling_usd
            && total_cost >= ceiling
        {
            visible.push_str(&format!(
                "\n\n[stopped: cost ceiling ${ceiling:.4} reached (spent ${total_cost:.4})]"
            ));
            return LoopOutcome {
                visible,
                error: None,
                total_cost_usd: total_cost,
            };
        }
    }

    visible.push_str(&format!(
        "\n\n[stopped: reached the {}-round tool limit without a final answer]",
        limits.max_rounds
    ));
    LoopOutcome {
        visible,
        error: None,
        total_cost_usd: total_cost,
    }
}

/// Build the assistant message that carries `tool_calls`. OpenAI requires
/// `function.arguments` to be a JSON *string*.
fn assistant_tool_call_msg(text: &str, calls: &[ToolCall]) -> Value {
    let tool_calls: Vec<Value> = calls
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "type": "function",
                "function": { "name": c.name, "arguments": c.args.to_string() }
            })
        })
        .collect();
    json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
        "tool_calls": tool_calls,
    })
}

fn tool_result_msg(call_id: &str, content: &str) -> Value {
    json!({ "role": "tool", "tool_call_id": call_id, "content": content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::backend::{StopReason, TokenUsage};
    use crate::types::FileScope;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Mutex;
    use anyhow::Result;
    use futures::Stream;

    use super::super::tools::{Tool, ToolOutcome};

    /// Client that replays a scripted list of event batches, one per round.
    struct ScriptedClient {
        rounds: Mutex<VecDeque<Vec<ApiEvent>>>,
    }

    #[async_trait::async_trait]
    impl ApiClient for ScriptedClient {
        async fn complete(
            &self,
            _request: ApiRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<ApiEvent>> + Send>>> {
            let batch = self
                .rounds
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| vec![ApiEvent::Done(StopReason::EndTurn)]);
            Ok(Box::pin(futures::stream::iter(batch.into_iter().map(Ok))))
        }
    }

    struct EchoTool;
    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn schema(&self) -> Value {
            json!({ "type": "function", "function": { "name": "echo", "parameters": { "type": "object", "properties": {} } } })
        }
        async fn run(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
            ToolOutcome::ok(format!("echoed:{args}"))
        }
    }

    struct NoopObserver;
    impl AcpObserver for NoopObserver {
        fn on_stream_chunk(&self, _t: &str) {}
        fn on_tool_call_started(&self, _t: &str) {}
        fn on_streaming_status(&self, _t: &str) {}
        fn on_message_complete(&self, _r: &str, _c: &str) {}
        fn on_proposal_deferred(&self, _p: &Path, _o: Option<&str>, _n: &str) {}
    }

    fn ctx() -> ToolCtx {
        ToolCtx {
            workspace_root: std::env::temp_dir(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: None,
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: false,
            observer: None,
        }
    }

    fn initial_messages() -> Vec<Value> {
        vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "go" }),
        ]
    }

    fn tool_call(name: &str) -> ApiEvent {
        ApiEvent::ToolCall(ToolCall {
            id: "call_1".into(),
            name: name.into(),
            args: json!({ "x": 1 }),
        })
    }

    #[tokio::test]
    async fn executes_tool_then_returns_final_answer() {
        let client = ScriptedClient {
            rounds: Mutex::new(VecDeque::from(vec![
                vec![tool_call("echo"), ApiEvent::Done(StopReason::ToolUse)],
                vec![
                    ApiEvent::Text("done: 42".into()),
                    ApiEvent::Usage(TokenUsage {
                        input_tokens: 5,
                        output_tokens: 2,
                        cost_usd: Some(0.001),
                        duration_ms: None,
                    }),
                    ApiEvent::Done(StopReason::EndTurn),
                ],
            ])),
        };
        let tools = ToolRegistry::new(vec![Box::new(EchoTool)]);
        let cancel = CancellationToken::new();
        let outcome = run_agent_loop(
            &client,
            &tools,
            &ctx(),
            &NoopObserver,
            "deepseek-v4-pro",
            initial_messages(),
            &LoopLimits::default(),
            &cancel,
        )
        .await;
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        assert!(outcome.visible.contains("done: 42"));
        assert!((outcome.total_cost_usd - 0.001).abs() < 1e-9);
    }

    #[tokio::test]
    async fn unknown_tool_feeds_error_then_continues() {
        let client = ScriptedClient {
            rounds: Mutex::new(VecDeque::from(vec![
                vec![tool_call("nope"), ApiEvent::Done(StopReason::ToolUse)],
                vec![
                    ApiEvent::Text("recovered".into()),
                    ApiEvent::Done(StopReason::EndTurn),
                ],
            ])),
        };
        let tools = ToolRegistry::new(vec![Box::new(EchoTool)]);
        let cancel = CancellationToken::new();
        let outcome = run_agent_loop(
            &client,
            &tools,
            &ctx(),
            &NoopObserver,
            "m",
            initial_messages(),
            &LoopLimits::default(),
            &cancel,
        )
        .await;
        assert!(outcome.error.is_none());
        assert!(outcome.visible.contains("recovered"));
    }

    #[tokio::test]
    async fn max_rounds_caps_runaway() {
        let mut rounds = VecDeque::new();
        for _ in 0..10 {
            rounds.push_back(vec![tool_call("echo"), ApiEvent::Done(StopReason::ToolUse)]);
        }
        let client = ScriptedClient {
            rounds: Mutex::new(rounds),
        };
        let tools = ToolRegistry::new(vec![Box::new(EchoTool)]);
        let cancel = CancellationToken::new();
        let limits = LoopLimits {
            max_rounds: 3,
            cost_ceiling_usd: None,
        };
        let outcome = run_agent_loop(
            &client,
            &tools,
            &ctx(),
            &NoopObserver,
            "m",
            initial_messages(),
            &limits,
            &cancel,
        )
        .await;
        assert!(outcome.error.is_none());
        assert!(
            outcome.visible.contains("3-round tool limit"),
            "got: {}",
            outcome.visible
        );
    }

    #[tokio::test]
    async fn bash_denied_feeds_error_to_model() {
        use std::sync::Arc;

        use crate::agent_session::tool_agent::policy::ScriptingObserver;

        let client = ScriptedClient {
            rounds: Mutex::new(VecDeque::from(vec![
                vec![
                    ApiEvent::ToolCall(ToolCall {
                        id: "c1".into(),
                        name: "Bash".into(),
                        args: json!({ "command": "npm install" }),
                    }),
                    ApiEvent::Done(StopReason::ToolUse),
                ],
                vec![
                    ApiEvent::Text("ok".into()),
                    ApiEvent::Done(StopReason::EndTurn),
                ],
            ])),
        };
        let tools = ToolRegistry::full_chat();
        let observer = Arc::new(ScriptingObserver {
            allow: false,
            prompted: Mutex::new(vec![]),
        });
        let mut ctx = ctx();
        ctx.observer = Some(observer.clone());
        let cancel = CancellationToken::new();
        let outcome = run_agent_loop(
            &client,
            &tools,
            &ctx,
            observer.as_ref(),
            "m",
            initial_messages(),
            &LoopLimits::default(),
            &cancel,
        )
        .await;
        assert!(outcome.error.is_none());
        assert_eq!(observer.prompted.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pre_cancelled_returns_error() {
        let client = ScriptedClient {
            rounds: Mutex::new(VecDeque::new()),
        };
        let tools = ToolRegistry::new(vec![Box::new(EchoTool)]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = run_agent_loop(
            &client,
            &tools,
            &ctx(),
            &NoopObserver,
            "m",
            initial_messages(),
            &LoopLimits::default(),
            &cancel,
        )
        .await;
        assert_eq!(outcome.error.as_deref(), Some("turn cancelled"));
    }
}
