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
use std::path::Path;
use tokio_util::sync::CancellationToken;

use crate::observer::AcpObserver;
use crate::workspace::{Workspace, settings};

use super::tools::{ToolCtx, ToolRegistry};
use super::{ApiClient, ApiEvent, ApiRequest, ToolCall};

/// Rounds allowed in one turn before the loop stops and hands off.
///
/// 40 covers a focused change; broad refactors that legitimately need many
/// reads can raise it per workspace with `agent.toolAgent.maxRounds`. Every
/// round is one billed API call, so this is a cost bound as much as a
/// runaway bound — and hitting it costs one *more* call, the hand-off round.
pub const DEFAULT_MAX_ROUNDS: u32 = 40;

/// Instruction appended for the post-cap hand-off round only. Never replayed
/// into the next turn as a user message: it is part of this request's local
/// `messages` vector, which is dropped when the turn returns.
const HANDOFF_INSTRUCTION: &str = "You have hit this turn's tool-call budget, so tools are
now disabled and you cannot continue working. Do not apologise and do not
restate the plan. Report only what the next turn needs to resume without
re-deriving it: (1) what you already established or changed, naming the exact
files and symbols; (2) what you verified and how; (3) the concrete next steps
that remain. Assume the reader has none of your tool output.";

/// Runaway bounds for one turn.
pub(crate) struct LoopLimits {
    pub max_rounds: u32,
    pub cost_ceiling_usd: Option<f64>,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_rounds: DEFAULT_MAX_ROUNDS,
            cost_ceiling_usd: None,
        }
    }
}

impl LoopLimits {
    /// Resolve the round cap from the workspace cascade.
    ///
    /// Key: `agent.toolAgent.maxRounds` (default [`DEFAULT_MAX_ROUNDS`]). An
    /// unparseable or zero value falls back to the default rather than
    /// disabling the bound — an unbounded tool loop is never the intent, and
    /// the plan that introduced this cap makes it mandatory.
    pub fn from_workspace(workspace: &Workspace, root: Option<&Path>) -> Self {
        let max_rounds = workspace
            .resolve_setting(settings::AGENT_TOOL_AGENT_MAX_ROUNDS, root)
            .as_u64()
            .filter(|n| *n > 0)
            .map(|n| n.min(u32::MAX as u64) as u32)
            .unwrap_or(DEFAULT_MAX_ROUNDS);
        Self {
            max_rounds,
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
        let mut round_reasoning = String::new();
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
                    round_reasoning.push_str(&t);
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

        messages.push(assistant_tool_call_msg(
            &round_text,
            &tool_calls,
            &round_reasoning,
        ));
        for call in &tool_calls {
            if cancel.is_cancelled() {
                return LoopOutcome {
                    visible,
                    error: Some("turn cancelled".to_string()),
                    total_cost_usd: total_cost,
                };
            }
            let summary = crate::acp::client::format_tool_summary(
                &call.name,
                &call.args,
                &ctx.workspace_root,
            );
            observer.on_tool_call_started(&summary);
            observer.on_streaming_status(&format!("Using {}...", call.name));

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

    // The cap is a budget, not a failure. Spend one final tools-disabled round
    // so the model hands off in prose instead of the transcript ending on a
    // bare marker: that hand-off is what lets a "continue" resume rather than
    // re-explore. `messages` currently ends on `tool` results (every round
    // appends its results before the cap check), so appending a `user`
    // instruction is valid — an unanswered `assistant` tool_calls message
    // would be rejected by the API.
    let marker = format!(
        "\n\n[stopped: reached the {}-round tool limit; asking for a hand-off]\n\n",
        limits.max_rounds
    );
    observer.on_stream_chunk(&marker);
    visible.push_str(&marker);

    if let Some(handoff) = handoff_round(client, observer, model, &messages, cancel).await {
        visible.push_str(&handoff);
    }

    LoopOutcome {
        visible,
        error: None,
        total_cost_usd: total_cost,
    }
}

/// Final tools-disabled round after the round cap trips.
///
/// Returns the model's hand-off text, or `None` when the call failed, was
/// cancelled, or produced nothing. Text is streamed to `observer` and also
/// returned so the caller can fold it into the turn's visible output — that
/// is what the TUI stores as the assistant transcript and replays next turn.
async fn handoff_round(
    client: &dyn ApiClient,
    observer: &dyn AcpObserver,
    model: &str,
    messages: &[Value],
    cancel: &CancellationToken,
) -> Option<String> {
    if cancel.is_cancelled() {
        return None;
    }
    let mut request_messages = messages.to_vec();
    request_messages.push(json!({ "role": "user", "content": HANDOFF_INSTRUCTION }));
    let request = ApiRequest {
        model: model.to_string(),
        messages: request_messages,
        // No schemas: the model cannot keep working, only report.
        tools: Vec::new(),
        max_tokens: None,
    };
    let mut stream = match client.complete(request).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("tool-agent hand-off round failed: {e:#}");
            return None;
        }
    };

    let mut text = String::new();
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => return None,
            e = stream.next() => match e {
                None => break,
                Some(ev) => ev,
            },
        };
        match event {
            Ok(ApiEvent::Text(t)) => {
                observer.on_stream_chunk(&t);
                text.push_str(&t);
            }
            // A tools-disabled request should not yield calls; ignore any that
            // arrive rather than inventing tool results for them.
            Ok(ApiEvent::ToolCall(_) | ApiEvent::Reasoning(_) | ApiEvent::Usage(_)) => {}
            Ok(ApiEvent::Done(_) | ApiEvent::Error(_)) => break,
            Err(_) => break,
        }
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("\n\n{trimmed}"))
}

/// Build the assistant message that carries `tool_calls`. OpenAI requires
/// `function.arguments` to be a JSON *string*.
fn assistant_tool_call_msg(text: &str, calls: &[ToolCall], reasoning: &str) -> Value {
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
    let mut msg = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
        "tool_calls": tool_calls,
    });
    if !reasoning.is_empty() {
        msg["reasoning_content"] = json!(reasoning);
    }
    msg
}

fn tool_result_msg(call_id: &str, content: &str) -> Value {
    json!({ "role": "tool", "tool_call_id": call_id, "content": content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::backend::{StopReason, TokenUsage};
    use crate::types::FileScope;
    use anyhow::Result;
    use futures::Stream;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Mutex;

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

    /// Client that records each `ApiRequest` then replays scripted events.
    struct RecordingClient {
        rounds: Mutex<VecDeque<Vec<ApiEvent>>>,
        seen: Mutex<Vec<Vec<Value>>>,
    }

    #[async_trait::async_trait]
    impl ApiClient for RecordingClient {
        async fn complete(
            &self,
            request: ApiRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<ApiEvent>> + Send>>> {
            self.seen.lock().unwrap().push(request.messages);
            let batch = self
                .rounds
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| vec![ApiEvent::Done(StopReason::EndTurn)]);
            Ok(Box::pin(futures::stream::iter(batch.into_iter().map(Ok))))
        }
    }

    #[tokio::test]
    async fn assistant_tool_call_replay_includes_reasoning_content() {
        let client = RecordingClient {
            rounds: Mutex::new(VecDeque::from(vec![
                vec![
                    ApiEvent::Reasoning("let me think".into()),
                    tool_call("echo"),
                    ApiEvent::Done(StopReason::ToolUse),
                ],
                vec![
                    ApiEvent::Text("ok".into()),
                    ApiEvent::Done(StopReason::EndTurn),
                ],
            ])),
            seen: Mutex::new(Vec::new()),
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
        let seen = client.seen.lock().unwrap();
        assert!(
            seen.len() >= 2,
            "expected a second request after the tool round"
        );
        let assistant = seen[1]
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .expect("assistant tool-call message on second request");
        assert_eq!(assistant["reasoning_content"], "let me think");
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
        // The hand-off round was requested but the scripted client had no
        // batch left, so it returned no text: the turn must still be a clean
        // success with the marker intact, not an error.
        assert!(
            outcome.visible.trim_end().ends_with("hand-off]"),
            "got: {}",
            outcome.visible
        );
    }

    /// Hitting the cap must produce a *usable* hand-off, not a bare marker:
    /// that text is what the next turn replays, so without it a "continue"
    /// re-derives everything the capped turn had already established.
    #[tokio::test]
    async fn cap_trip_runs_a_handoff_round_and_keeps_its_text() {
        let mut rounds = VecDeque::new();
        for _ in 0..3 {
            rounds.push_back(vec![tool_call("echo"), ApiEvent::Done(StopReason::ToolUse)]);
        }
        rounds.push_back(vec![
            ApiEvent::Text("read src/a.rs; edited B::c; next: cargo test".into()),
            ApiEvent::Done(StopReason::EndTurn),
        ]);
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
        assert!(outcome.visible.contains("3-round tool limit"));
        assert!(
            outcome.visible.contains("next: cargo test"),
            "hand-off text must reach the turn output: {}",
            outcome.visible
        );
    }

    /// A cancelled turn must not spend an extra API call on the hand-off.
    #[tokio::test]
    async fn cancelled_turn_skips_the_handoff_round() {
        let client = ScriptedClient {
            rounds: Mutex::new(VecDeque::new()),
        };
        let tools = ToolRegistry::new(vec![Box::new(EchoTool)]);
        let cancel = CancellationToken::new();
        let limits = LoopLimits {
            max_rounds: 0,
            cost_ceiling_usd: None,
        };
        cancel.cancel();
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
        assert!(outcome.visible.contains("0-round tool limit"));
        assert!(!outcome.visible.contains("next: cargo test"));
    }

    #[test]
    fn loop_limits_resolve_from_the_workspace_cascade() {
        let dir = tempfile::tempdir().unwrap();
        let single =
            |dir: &std::path::Path| crate::workspace::Workspace::single_folder(dir.to_path_buf());
        assert_eq!(
            LoopLimits::from_workspace(&single(dir.path()), Some(dir.path())).max_rounds,
            DEFAULT_MAX_ROUNDS
        );

        let gaviero = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gaviero).unwrap();
        std::fs::write(
            gaviero.join("settings.json"),
            r#"{ "agent": { "toolAgent": { "maxRounds": 120 } } }"#,
        )
        .unwrap();
        assert_eq!(
            LoopLimits::from_workspace(&single(dir.path()), Some(dir.path())).max_rounds,
            120
        );

        // Zero must not unbind the loop — an unbounded tool loop is never the
        // intent, so it falls back to the default cap.
        std::fs::write(
            gaviero.join("settings.json"),
            r#"{ "agent": { "toolAgent": { "maxRounds": 0 } } }"#,
        )
        .unwrap();
        assert_eq!(
            LoopLimits::from_workspace(&single(dir.path()), Some(dir.path())).max_rounds,
            DEFAULT_MAX_ROUNDS
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
