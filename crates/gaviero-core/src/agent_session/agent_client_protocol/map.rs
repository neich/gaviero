//! `session/update` → [`UnifiedStreamEvent`].

use serde_json::Value;

use crate::acp::client::format_tool_summary;
use crate::swarm::backend::{StopReason, TokenUsage, UnifiedStreamEvent};

/// Map one `session/update` notification (or its inner `update` object)
/// into zero or more stream events.
pub fn map_session_update(
    value: &Value,
    workspace_root: &std::path::Path,
) -> Vec<UnifiedStreamEvent> {
    let update = value
        .get("params")
        .and_then(|p| p.get("update"))
        .unwrap_or_else(|| value.get("update").unwrap_or(value));
    let kind = update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => text_delta(update, false),
        "agent_thought_chunk" => text_delta(update, true),
        "tool_call" => vec![tool_start(update, workspace_root)],
        "tool_call_update" => tool_end(update),
        "usage_update" => usage(update).into_iter().collect(),
        _ => Vec::new(),
    }
}

pub fn map_stop_reason(result: &Value) -> StopReason {
    match result
        .get("stopReason")
        .or_else(|| result.get("stop_reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn")
    {
        "cancelled" | "canceled" => StopReason::Timeout,
        "max_tokens" | "max_turn_requests" => StopReason::EndTurn,
        "refusal" => StopReason::Error,
        _ => StopReason::EndTurn,
    }
}

fn content_text(update: &Value) -> String {
    let content = update.get("content").unwrap_or(update);
    if let Some(t) = content.get("text").and_then(|v| v.as_str()) {
        return t.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
    }
    content.as_str().unwrap_or("").to_string()
}

fn text_delta(update: &Value, thought: bool) -> Vec<UnifiedStreamEvent> {
    let text = content_text(update);
    if text.is_empty() {
        return Vec::new();
    }
    if thought {
        vec![UnifiedStreamEvent::ThinkingDelta(text)]
    } else {
        vec![UnifiedStreamEvent::TextDelta(text)]
    }
}

fn tool_start(update: &Value, workspace_root: &std::path::Path) -> UnifiedStreamEvent {
    let id = update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = update
        .get("title")
        .or_else(|| update.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("tool")
        .to_string();
    let args = update.get("rawInput").cloned().unwrap_or(Value::Null);
    let summary = format_tool_summary(&name, &args, workspace_root);
    UnifiedStreamEvent::ToolCallStart {
        id,
        name: summary,
        args,
    }
}

fn tool_end(update: &Value) -> Vec<UnifiedStreamEvent> {
    let id = update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
    // Only a settled call ends: agy reports `pending` before every permission
    // request, and a content-only update carries no status at all.
    if !matches!(status, "completed" | "failed") {
        return Vec::new();
    }
    vec![UnifiedStreamEvent::ToolCallEnd { id }]
}

/// Token usage carried by a `session/prompt` result, when the agent reports
/// one. agy sends `{inputTokens, outputTokens, cachedReadTokens, …}` there,
/// not in a `usage_update`.
pub fn map_prompt_usage(result: &Value) -> Option<UnifiedStreamEvent> {
    let usage = result.get("usage")?;
    if !usage.is_object() {
        return None;
    }
    let read = |camel: &str, snake: &str| {
        usage
            .get(camel)
            .or_else(|| usage.get(snake))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Some(UnifiedStreamEvent::Usage(TokenUsage {
        input_tokens: read("inputTokens", "input_tokens"),
        output_tokens: read("outputTokens", "output_tokens"),
        cost_usd: None,
        duration_ms: None,
    }))
}

fn usage(update: &Value) -> Option<UnifiedStreamEvent> {
    let used = update.get("used").or_else(|| update.get("usage"))?;
    // Live dsh-acp sends `used` as the session's total context tokens
    // (a number, with `size` = window); the TUI keeps the last report as
    // context occupancy, so it maps onto input_tokens.
    if let Some(n) = used.as_u64() {
        return Some(UnifiedStreamEvent::Usage(TokenUsage {
            input_tokens: n,
            output_tokens: 0,
            cost_usd: None,
            duration_ms: None,
        }));
    }
    let input = used
        .get("inputTokens")
        .or_else(|| used.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = used
        .get("outputTokens")
        .or_else(|| used.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Some(UnifiedStreamEvent::Usage(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cost_usd: None,
        duration_ms: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn maps_message_and_thought_chunks() {
        let root = Path::new(".");
        let msg = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" }
        });
        let thought = json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "hmm" }
        });
        assert!(matches!(
            &map_session_update(&msg, root)[..],
            [UnifiedStreamEvent::TextDelta(t)] if t == "hello"
        ));
        assert!(matches!(
            &map_session_update(&thought, root)[..],
            [UnifiedStreamEvent::ThinkingDelta(t)] if t == "hmm"
        ));
    }

    #[test]
    fn maps_tool_call_and_end() {
        let root = Path::new(".");
        let start = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "c1",
            "title": "Read",
            "rawInput": { "file_path": "src/lib.rs" }
        });
        let end = json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "status": "completed"
        });
        match &map_session_update(&start, root)[..] {
            [UnifiedStreamEvent::ToolCallStart { id, name, .. }] => {
                assert_eq!(id, "c1");
                assert!(name.contains("Read"), "{name}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            &map_session_update(&end, root)[..],
            [UnifiedStreamEvent::ToolCallEnd { id }] if id == "c1"
        ));
    }

    #[test]
    fn maps_usage_and_stop_reason() {
        let root = Path::new(".");
        let u = json!({
            "sessionUpdate": "usage_update",
            "used": { "inputTokens": 3, "outputTokens": 4 }
        });
        match &map_session_update(&u, root)[..] {
            [UnifiedStreamEvent::Usage(t)] => {
                assert_eq!(t.input_tokens, 3);
                assert_eq!(t.output_tokens, 4);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            map_stop_reason(&json!({"stopReason":"cancelled"})),
            StopReason::Timeout
        );
    }

    #[test]
    fn pending_tool_update_does_not_end_the_tool() {
        let root = Path::new(".");
        for status in ["pending", "in_progress", ""] {
            let update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "c1",
                "status": status
            });
            assert!(
                map_session_update(&update, root).is_empty(),
                "status {status:?} must not end the tool"
            );
        }
    }

    #[test]
    fn completed_and_failed_end_the_tool() {
        let root = Path::new(".");
        for status in ["completed", "failed"] {
            let update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "c1",
                "status": status
            });
            assert!(
                matches!(
                    &map_session_update(&update, root)[..],
                    [UnifiedStreamEvent::ToolCallEnd { id }] if id == "c1"
                ),
                "status {status:?} must end the tool"
            );
        }
    }

    #[test]
    fn prompt_usage_maps_input_and_output_tokens() {
        let result = json!({
            "stopReason": "end_turn",
            "usage": {
                "inputTokens": 12994, "outputTokens": 165,
                "cachedReadTokens": 3543, "thoughtTokens": 0, "totalTokens": 16702
            }
        });
        match map_prompt_usage(&result) {
            Some(UnifiedStreamEvent::Usage(t)) => {
                assert_eq!(t.input_tokens, 12994);
                assert_eq!(t.output_tokens, 165);
            }
            other => panic!("{other:?}"),
        }
        let snake = json!({"usage": {"input_tokens": 3, "output_tokens": 4}});
        match map_prompt_usage(&snake) {
            Some(UnifiedStreamEvent::Usage(t)) => {
                assert_eq!((t.input_tokens, t.output_tokens), (3, 4));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prompt_without_usage_maps_nothing() {
        assert!(map_prompt_usage(&json!({"stopReason": "end_turn"})).is_none());
        assert!(map_prompt_usage(&json!({"usage": null})).is_none());
    }

    #[test]
    fn maps_numeric_usage_as_context_tokens() {
        let u = json!({"sessionUpdate":"usage_update","used":123456,"size":1000000});
        match &map_session_update(&u, Path::new("."))[..] {
            [UnifiedStreamEvent::Usage(t)] => {
                assert_eq!(t.input_tokens, 123456);
                assert_eq!(t.output_tokens, 0);
            }
            other => panic!("{other:?}"),
        }
    }
}
