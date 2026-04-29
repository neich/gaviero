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
    SystemInit { session_id: String, model: String },

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

    /// Permission request — Claude wants to execute a tool and needs user approval.
    /// The pipeline pauses until the user approves or denies via stdin.
    PermissionRequest {
        tool_name: String,
        description: String,
        request_id: String,
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
                        Ok(StreamEvent::ContentDelta(required_str(&delta, "text")?))
                    } else if delta_type == "thinking_delta" {
                        Ok(StreamEvent::ThinkingDelta(required_str(
                            &delta, "thinking",
                        )?))
                    } else if delta_type == "input_json_delta" {
                        Ok(StreamEvent::ToolInputDelta(
                            delta
                                .get("partial_json")
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
                            let name = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            tool_uses.push(ToolUseInfo { name, input });
                        }
                        _ => {}
                    }
                }
            }

            Ok(StreamEvent::AssistantMessage { text, tool_uses })
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

        // Permission request: Claude wants to run a tool and needs approval.
        // {"type":"permission_request","tool_name":"Bash","description":"Run ...","id":"req_123"}
        "permission_request" => {
            let tool_name = opt_str(&v, "tool_name").to_string();
            let description = opt_str(&v, "description").to_string();
            // Accept either "id" or "permission_request_id" for robustness
            let request_id = v
                .get("permission_request_id")
                .or_else(|| v.get("id"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            Ok(StreamEvent::PermissionRequest {
                tool_name,
                description,
                request_id,
            })
        }

        _ => Ok(StreamEvent::Unknown(v)),
    }
}

/// Try to extract the next complete `<file>` block starting at `from` offset.
/// Returns `(path, content, end_position)` if a complete block is found.
///
/// Skips opener candidates inside fenced markdown code blocks (``` and ~~~)
/// so illustrative examples in chat prose aren't applied as real proposals.
/// Also rejects malformed paths (empty, absolute, or containing `..`
/// traversal segments) — silently advancing past the offending opener.
pub fn find_next_file_block(text: &str, from: usize) -> Option<(PathBuf, String, usize)> {
    let regions = code_suppression_regions(text);
    let mut start = from;
    loop {
        let rel = text[start..].find("<file path=\"")?;
        let tag_start = start + rel;

        if let Some(end) = region_end_containing(&regions, tag_start) {
            start = end;
            continue;
        }

        let after_attr = tag_start + "<file path=\"".len();
        let quote_end = text[after_attr..].find('"')?;
        let path_str = &text[after_attr..after_attr + quote_end];

        if !is_valid_proposal_path(path_str) {
            tracing::warn!(
                "Skipping file-block proposal with invalid path attribute: {:?}",
                path_str
            );
            start = after_attr + quote_end + 1;
            continue;
        }

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
        return Some((PathBuf::from(path_str), content, block_end));
    }
}

/// Extract `<file path="...">content</file>` blocks from text.
///
/// The system prompt instructs Claude to output proposed file changes
/// in this format. We parse them and route each through the Write Gate.
///
/// Same protections as [`find_next_file_block`]: openers inside fenced code
/// blocks are skipped, and paths that are empty / absolute / contain `..`
/// traversal segments are rejected.
pub fn parse_file_blocks(text: &str) -> Vec<(PathBuf, String)> {
    let regions = code_suppression_regions(text);
    let mut results = Vec::new();
    let mut search_from = 0;

    loop {
        // Find opening tag: <file path="...">
        let Some(rel) = text[search_from..].find("<file path=\"") else {
            break;
        };
        let tag_start = search_from + rel;

        if let Some(end) = region_end_containing(&regions, tag_start) {
            search_from = end;
            continue;
        }

        let after_attr = tag_start + "<file path=\"".len();

        // Find closing quote of path attribute
        let Some(quote_end) = text[after_attr..].find('"') else {
            break;
        };
        let path_str = &text[after_attr..after_attr + quote_end];

        if !is_valid_proposal_path(path_str) {
            tracing::warn!(
                "Skipping file-block proposal with invalid path attribute: {:?}",
                path_str
            );
            search_from = after_attr + quote_end + 1;
            continue;
        }

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
        let content_end =
            if content_end > content_start && text.as_bytes()[content_end - 1] == b'\n' {
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

/// Whether `path_str` is acceptable as a proposal target.
///
/// Reject empty, absolute, traversal (`..`), and any embedded control
/// characters / quotes / angle brackets. The checks ensure that the
/// resulting `workspace_root.join(path)` stays inside the workspace and
/// can't be subverted by hallucinated paths in chat output.
fn is_valid_proposal_path(path_str: &str) -> bool {
    if path_str.is_empty() {
        return false;
    }
    if path_str.chars().any(|c| {
        c == '\0'
            || c == '"'
            || c == '<'
            || c == '>'
            || c == '\n'
            || c == '\r'
            || ((c as u32) < 0x20 && c != '\t')
    }) {
        return false;
    }
    let p = std::path::Path::new(path_str);
    if p.is_absolute() {
        return false;
    }
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => return false,
            _ => {}
        }
    }
    true
}

fn region_end_containing(regions: &[(usize, usize)], pos: usize) -> Option<usize> {
    regions
        .iter()
        .find(|(s, e)| pos >= *s && pos < *e)
        .map(|(_, e)| *e)
}

/// Compute byte ranges of fenced markdown code blocks in `text`.
///
/// Combined suppression regions: union of fenced code blocks and inline
/// code spans. `<file>` openers inside any of these are skipped.
///
/// Inline-span coverage was added because triple-fence skipping alone wasn't
/// enough — when the agent quoted the format back inside single-backtick
/// inline spans (e.g. `` `<file path="x">y</file>` ``), the parser still
/// picked it up and produced a real proposal. Same bug class as the fenced
/// case, just a different markdown construct.
fn code_suppression_regions(text: &str) -> Vec<(usize, usize)> {
    let mut regions = fenced_code_regions(text);
    regions.extend(inline_code_regions(text));
    regions
}

/// Find inline code spans (paired backtick runs of equal length).
///
/// Mirrors CommonMark inline code semantics conservatively: a backtick run
/// of length N is closed by the next backtick run of length N. Unmatched
/// runs are treated as literal backticks (no region emitted).
///
/// Spans may cross newlines (CommonMark allows it). We don't attempt to
/// honour escaped backticks (`\``) — chat output rarely contains them, and
/// the conservative direction here is to over-suppress.
fn inline_code_regions(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run_start = i;
        let mut run_len = 0;
        while i < bytes.len() && bytes[i] == b'`' {
            run_len += 1;
            i += 1;
        }
        // Look for a closing run of exactly `run_len` backticks.
        let mut j = i;
        let mut close_end: Option<usize> = None;
        while j < bytes.len() {
            if bytes[j] != b'`' {
                j += 1;
                continue;
            }
            let close_start = j;
            let mut close_len = 0;
            while j < bytes.len() && bytes[j] == b'`' {
                close_len += 1;
                j += 1;
            }
            if close_len == run_len {
                close_end = Some(j);
                break;
            }
            // Different length — keep scanning. `close_start` not used,
            // but kept for clarity; `j` already advanced past the run.
            let _ = close_start;
        }
        if let Some(end) = close_end {
            regions.push((run_start, end));
            i = end;
        }
        // Unmatched opener: leave `i` past the opening run; treat the
        // backticks as literal.
    }
    regions
}

/// Each range is `[line_start, close_line_end_exclusive)` and covers from
/// the opening fence line through the closing fence line. Unclosed fences
/// extend to EOF. Recognises both ``` and ~~~ fences and tolerates up to
/// 3 leading spaces of indentation per CommonMark.
///
/// The parser uses these regions to suppress `<file>` openers found inside
/// fenced examples — the most common source of accidental proposals when
/// the agent quotes the format back to the user.
fn fenced_code_regions(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let line_start = i;
        let mut p = i;
        let mut indent = 0;
        while p < bytes.len() && bytes[p] == b' ' && indent < 3 {
            p += 1;
            indent += 1;
        }
        let fence_char = if p + 3 <= bytes.len() && &bytes[p..p + 3] == b"```" {
            Some(b'`')
        } else if p + 3 <= bytes.len() && &bytes[p..p + 3] == b"~~~" {
            Some(b'~')
        } else {
            None
        };
        if let Some(fc) = fence_char {
            let mut fence_len = 0;
            while p + fence_len < bytes.len() && bytes[p + fence_len] == fc {
                fence_len += 1;
            }
            let after_open_line = match bytes[p + fence_len..].iter().position(|&b| b == b'\n') {
                Some(n) => p + fence_len + n + 1,
                None => bytes.len(),
            };
            let mut k = after_open_line;
            let mut close_end = bytes.len();
            while k < bytes.len() {
                let ls = k;
                let mut q = ls;
                let mut ind = 0;
                while q < bytes.len() && bytes[q] == b' ' && ind < 3 {
                    q += 1;
                    ind += 1;
                }
                if q + fence_len <= bytes.len()
                    && bytes[q..q + fence_len].iter().all(|&c| c == fc)
                {
                    let after = q + fence_len;
                    let line_end = bytes[after..]
                        .iter()
                        .position(|&b| b == b'\n')
                        .map(|n| after + n)
                        .unwrap_or(bytes.len());
                    if bytes[after..line_end]
                        .iter()
                        .all(|&b| b == b' ' || b == b'\t')
                    {
                        close_end = line_end;
                        k = if line_end < bytes.len() {
                            line_end + 1
                        } else {
                            bytes.len()
                        };
                        break;
                    }
                }
                k = match bytes[ls..].iter().position(|&b| b == b'\n') {
                    Some(n) => ls + n + 1,
                    None => bytes.len(),
                };
            }
            regions.push((line_start, close_end));
            i = k;
            continue;
        }
        i = match bytes[i..].iter().position(|&b| b == b'\n') {
            Some(n) => i + n + 1,
            None => bytes.len(),
        };
    }
    regions
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
        assert_eq!(blocks[0].1, "fn main() {\n    println!(\"hello\");\n}");
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

    // Regression: triple-backtick fenced examples must not be parsed as
    // real proposals — common failure mode when the agent quotes the
    // format back to the user. See acp/protocol.rs::fenced_code_regions.
    #[test]
    fn test_parse_file_blocks_skips_triple_backtick_fence() {
        let text = "Format example:\n\
                    ```\n\
                    <file path=\"src/example.rs\">illustrative body</file>\n\
                    ```\n\
                    end.";
        let blocks = parse_file_blocks(text);
        assert!(
            blocks.is_empty(),
            "fenced examples must not parse; got {blocks:?}"
        );
    }

    #[test]
    fn test_parse_file_blocks_skips_tilde_fence() {
        let text = "Format example:\n\
                    ~~~\n\
                    <file path=\"src/example.rs\">illustrative body</file>\n\
                    ~~~\n\
                    end.";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_parse_file_blocks_skips_fenced_with_lang_tag() {
        // Code-fence info string after the opener (```xml, ```rust, etc.)
        // is the typical form when explaining the format.
        let text = "```xml\n<file path=\"a.rs\">ex</file>\n```";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_parse_file_blocks_fenced_then_real() {
        // Fenced example is ignored; the unfenced block after it is real.
        let text = "Example:\n\
                    ```xml\n\
                    <file path=\"a.rs\">ex</file>\n\
                    ```\n\
                    Real:\n\
                    <file path=\"b.rs\">\n\
                    fn main() {}\n\
                    </file>";
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, PathBuf::from("b.rs"));
        assert_eq!(blocks[0].1, "fn main() {}");
    }

    #[test]
    fn test_parse_file_blocks_rejects_empty_path() {
        let text = "<file path=\"\">junk</file>\n<file path=\"v.rs\">code</file>";
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, PathBuf::from("v.rs"));
    }

    #[test]
    fn test_parse_file_blocks_rejects_absolute_path() {
        let text = "<file path=\"/etc/passwd\">root</file>";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty(), "absolute paths must be rejected");
    }

    #[test]
    fn test_parse_file_blocks_rejects_traversal() {
        let text = "<file path=\"../../etc/passwd\">root</file>";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty(), "traversal paths must be rejected");
    }

    #[test]
    fn test_parse_file_blocks_rejects_path_with_angle_brackets() {
        let text = "<file path=\"a<b\">x</file>";
        let blocks = parse_file_blocks(text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_parse_file_blocks_skips_inline_backtick_span() {
        // The format-quoting incident: an opener wrapped in single
        // backticks must NOT produce a real proposal.
        let text = "Format example: `<file path=\"x\">y</file>` end.";
        let blocks = parse_file_blocks(text);
        assert!(
            blocks.is_empty(),
            "inline single-backtick spans must suppress openers"
        );
    }

    #[test]
    fn test_parse_file_blocks_skips_inline_double_backtick_span() {
        // Double-backtick spans wrap content that itself contains a
        // backtick. The opener inside must still be suppressed.
        let text = "See ``<file path=\"x\">contains ` tick</file>`` here.";
        let blocks = parse_file_blocks(text);
        assert!(
            blocks.is_empty(),
            "inline double-backtick spans must suppress openers"
        );
    }

    #[test]
    fn test_parse_file_blocks_inline_then_real() {
        // Inline-quoted example is ignored; real unfenced block after it
        // is still picked up.
        let text = "Use `<file path=\"a.rs\">ex</file>` like so.\n\
                    <file path=\"b.rs\">\n\
                    fn main() {}\n\
                    </file>";
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, PathBuf::from("b.rs"));
        assert_eq!(blocks[0].1, "fn main() {}");
    }

    #[test]
    fn test_parse_file_blocks_unmatched_inline_tick_does_not_swallow_real_block() {
        // A stray unmatched backtick must not suppress a later real block.
        let text = "stray ` tick\n<file path=\"r.rs\">code</file>";
        let blocks = parse_file_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, PathBuf::from("r.rs"));
    }

    #[test]
    fn test_inline_code_regions_pairs_runs_of_equal_length() {
        let text = "a `b` c ``d`` e ``` f ```";
        let regions = inline_code_regions(text);
        assert_eq!(regions.len(), 3, "expected three paired runs");
    }

    #[test]
    fn test_inline_code_regions_unmatched_run_is_dropped() {
        // Single unmatched backtick produces no region.
        let text = "stray ` and nothing else";
        let regions = inline_code_regions(text);
        assert!(regions.is_empty());
    }

    #[test]
    fn test_find_next_file_block_skips_fenced_block() {
        let text = "Format:\n\
                    ```\n\
                    <file path=\"ex.rs\">ex</file>\n\
                    ```\n\
                    Then:\n\
                    <file path=\"real.rs\">code</file>";
        let result = find_next_file_block(text, 0);
        assert!(result.is_some(), "real block should be found");
        let (path, content, _end) = result.unwrap();
        assert_eq!(path, PathBuf::from("real.rs"));
        assert_eq!(content, "code");
    }

    #[test]
    fn test_find_next_file_block_rejects_absolute_path_returns_next_valid() {
        let text = "<file path=\"/etc/passwd\">root</file>\n<file path=\"ok.rs\">x</file>";
        let result = find_next_file_block(text, 0);
        assert!(result.is_some());
        let (path, _content, _end) = result.unwrap();
        assert_eq!(path, PathBuf::from("ok.rs"));
    }

    #[test]
    fn test_fenced_code_regions_unclosed_extends_to_eof() {
        let text = "before\n```\ndangling forever";
        let regions = fenced_code_regions(text);
        assert_eq!(regions.len(), 1);
        let (_, e) = regions[0];
        assert_eq!(e, text.len());
    }
}
