//! NDJSON stream event types for `claude --output-format stream-json`.
//!
//! Each line of stdout is a JSON object. We parse the ones we care about
//! and pass everything else through as `Unknown`.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

/// Information about a tool_use block extracted from an assistant message.
#[derive(Debug, Clone)]
pub struct ToolUseInfo {
    pub name: String,
    pub input: Value,
}

/// Parsed event from one NDJSON line of `claude --print --output-format stream-json`.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Session initialization (type: "system", subtype: "init").
    SystemInit {
        session_id: String,
        model: String,
    },

    /// Streaming text chunk from the assistant response.
    ContentDelta(String),

    /// Streaming thinking/reasoning chunk from extended thinking.
    ThinkingDelta(String),

    /// Agent started a tool call.
    ToolUseStart {
        tool_name: String,
        tool_use_id: String,
    },
    /// Incremental tool input JSON (type: "input_json_delta").
    ToolInputDelta(String),

    /// Complete assistant message (type: "assistant").
    AssistantMessage {
        /// Concatenated text content from the message.
        text: String,
        /// Tool calls in this message (Write, Edit, etc. with their inputs).
        tool_uses: Vec<ToolUseInfo>,
    },

    /// Final result (type: "result").
    ResultEvent {
        is_error: bool,
        result_text: String,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
    },

    /// Anything we don't specifically handle.
    Unknown(Value),
}

/// Extract a required string field from a JSON value, returning an error if missing.
fn required_str(v: &Value, key: &str) -> Result<String> {
    v.get(key)
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .with_context(|| format!("missing required field '{}'", key))
}

/// Extract an optional string field, defaulting to "" if absent.
/// Used for dispatch/discriminant fields where "unknown" is a valid fallback.
fn opt_str<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|t| t.as_str()).unwrap_or("")
}

/// Parse a single NDJSON line into a StreamEvent.
pub fn parse_stream_line(line: &str) -> Result<StreamEvent> {
    let v: Value = serde_json::from_str(line).context("parsing NDJSON line")?;

    let event_type = opt_str(&v, "type");

    match event_type {
        // System init: {"type":"system","subtype":"init","session_id":"...","model":"..."}
        "system" => {
            let subtype = opt_str(&v, "subtype");
            if subtype == "init" {
                Ok(StreamEvent::SystemInit {
                    session_id: required_str(&v, "session_id")?,
                    model: required_str(&v, "model")?,
                })
            } else {
                Ok(StreamEvent::Unknown(v))
            }
        }

        // Streaming event wrapper:
        // {"type":"stream_event","event":{"type":"content_block_delta","index":0,
        //   "delta":{"type":"text_delta","text":"..."}}}
        "stream_event" => {
            let event = v.get("event").cloned().unwrap_or(Value::Null);
            let inner_type = opt_str(&event, "type");

            match inner_type {
                "content_block_delta" => {
                    let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                    let delta_type = opt_str(&delta, "type");
                    if delta_type == "text_delta" {
                        Ok(StreamEvent::ContentDelta(
                            required_str(&delta, "text")?,
                        ))
                    } else if delta_type == "thinking_delta" {
                        Ok(StreamEvent::ThinkingDelta(
                            required_str(&delta, "thinking")?,
                        ))
                    } else if delta_type == "input_json_delta" {
                        Ok(StreamEvent::ToolInputDelta(
                            delta.get("partial_json")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        ))
                    } else {
                        Ok(StreamEvent::Unknown(v))
                    }
                }
                "content_block_start" => {
                    let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                    let block_type = opt_str(&block, "type");
                    if block_type == "tool_use" {
                        Ok(StreamEvent::ToolUseStart {
                            tool_name: required_str(&block, "name")?,
                            tool_use_id: required_str(&block, "id")?,
                        })
                    } else {
                        Ok(StreamEvent::Unknown(v))
                    }
                }
                _ => Ok(StreamEvent::Unknown(v)),
            }
        }

        // Complete assistant message:
        // {"type":"assistant","message":{"content":[{"type":"text","text":"..."},...]}}
        "assistant" => {
            let message = v.get("message").cloned().unwrap_or(Value::Null);
            let content = message.get("content").and_then(|c| c.as_array());

            let mut text = String::new();
            let mut tool_uses = Vec::new();

            if let Some(blocks) = content {
                for block in blocks {
                    let bt = opt_str(block, "type");
                    match bt {
                        "text" => {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                        "tool_use" => {
                            let name = block.get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input = block.get("input")
                                .cloned()
                                .unwrap_or(Value::Null);
                            tool_uses.push(ToolUseInfo { name, input });
                        }
                        _ => {}
                    }
                }
            }

            Ok(StreamEvent::AssistantMessage {
                text,
                tool_uses,
            })
        }

        // Final result:
        // {"type":"result","subtype":"success","result":"...","duration_ms":1234,"cost_usd":0.01}
        "result" => {
            let is_error = v.get("subtype").and_then(|s| s.as_str()) == Some("error");
            let result_text = required_str(&v, "result")?;
            let duration_ms = v.get("duration_ms").and_then(|d| d.as_u64());
            let cost_usd = v.get("cost_usd").and_then(|c| c.as_f64());

            Ok(StreamEvent::ResultEvent {
                is_error,
                result_text,
                duration_ms,
                cost_usd,
            })
        }

        _ => Ok(StreamEvent::Unknown(v)),
    }
}

/// Try to extract the next complete `<file>` block starting at `from` offset.
/// Returns `(path, content, end_position)` if a complete block is found.
pub fn find_next_file_block(text: &str, from: usize) -> Option<(PathBuf, String, usize)> {
    let search = &text[from..];

    let tag_start = search.find("<file path=\"")?;
    let tag_start = from + tag_start;
    let after_attr = tag_start + "<file path=\"".len();

    let quote_end = text[after_attr..].find('"')?;
    let path_str = &text[after_attr..after_attr + quote_end];

    let tag_close = after_attr + quote_end + 1;
    if tag_close >= text.len() || text.as_bytes()[tag_close] != b'>' {
        return None;
    }
    let mut content_start = tag_close + 1;

    // Strip leading newline
    if text[content_start..].starts_with('\n') {
        content_start += 1;
    }

    let close_pos = text[content_start..].find("</file>")?;
    let mut content_end = content_start + close_pos;

    // Strip trailing newline
    if content_end > content_start && text.as_bytes()[content_end - 1] == b'\n' {
        content_end -= 1;
    }

    let content = text[content_start..content_end].to_string();
    let block_end = content_start + close_pos + "</file>".len();

    Some((PathBuf::from(path_str), content, block_end))
}

/// Extract `<file path="...">content</file>` blocks from text.
///
/// The system prompt instructs Claude to output proposed file changes
/// in this format. We parse them and route each through the Write Gate.
pub fn parse_file_blocks(text: &str) -> Vec<(PathBuf, String)> {
    let mut results = Vec::new();
    let mut search_from = 0;

    loop {
        // Find opening tag: <file path="...">
        let Some(tag_start) = text[search_from..].find("<file path=\"") else {
            break;
        };
        let tag_start = search_from + tag_start;
        let after_attr = tag_start + "<file path=\"".len();

        // Find closing quote of path attribute
        let Some(quote_end) = text[after_attr..].find('"') else {
            break;
        };
        let path_str = &text[after_attr..after_attr + quote_end];

        // Find end of opening tag (the `>`)
        let tag_close = after_attr + quote_end + 1;
        if tag_close >= text.len() || text.as_bytes()[tag_close] != b'>' {
            search_from = tag_close;
            continue;
        }
        let content_start = tag_close + 1;

        // Strip leading newline from content if present
        let content_start = if text[content_start..].starts_with('\n') {
            content_start + 1
        } else {
            content_start
        };

        // Find closing tag: </file>
        let Some(close_pos) = text[content_start..].find("</file>") else {
            break;
        };
        let content_end = content_start + close_pos;

        // Strip trailing newline from content if present
        let content_end = if content_end > content_start
            && text.as_bytes()[content_end - 1] == b'\n'
        {
            content_end - 1
        } else {
            content_end
        };

        let content = text[content_start..content_end].to_string();
        results.push((PathBuf::from(path_str), content));

        search_from = content_start + close_pos + "</file>".len();
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_delta() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ContentDelta(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected ContentDelta, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_tool_use_start() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_123","name":"Read","input":{}}}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ToolUseStart {
                tool_name,
                tool_use_id,
            } => {
                assert_eq!(tool_name, "Read");
                assert_eq!(tool_use_id, "toolu_123");
            }
            _ => panic!("Expected ToolUseStart, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_system_init() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-sonnet-4-6","tools":[]}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::SystemInit { session_id, model } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(model, "claude-sonnet-4-6");
            }
            _ => panic!("Expected SystemInit, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_result() {
        let line = r#"{"type":"result","subtype":"success","result":"Done!","duration_ms":5000,"cost_usd":0.01}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ResultEvent {
                is_error,
                result_text,
                duration_ms,
                cost_usd,
            } => {
                assert!(!is_error);
                assert_eq!(result_text, "Done!");
                assert_eq!(duration_ms, Some(5000));
                assert!((cost_usd.unwrap() - 0.01).abs() < f64::EPSILON);
            }
            _ => panic!("Expected ResultEvent, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_file_blocks_single() {
        let text = r#"Here is the fix:
<file path="src/main.rs">
fn main() {
    println!("hello");
}
</file>
Done!"#;
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, PathBuf::from("src/main.rs"));
        assert_eq!(
            blocks[0].1,
            "fn main() {\n    println!(\"hello\");\n}"
        );
    }

    #[test]
    fn test_parse_file_blocks_multiple() {
        let text = r#"<file path="a.rs">
aaa
</file>
<file path="b.rs">
bbb
</file>"#;
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, PathBuf::from("a.rs"));
        assert_eq!(blocks[0].1, "aaa");
        assert_eq!(blocks[1].0, PathBuf::from("b.rs"));
        assert_eq!(blocks[1].1, "bbb");
    }

    #[test]
    fn test_parse_file_blocks_empty() {
        let text = "No file blocks here.";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty());
    }
}
