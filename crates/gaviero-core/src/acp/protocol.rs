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
    /// Claude's `tool_use` id (`toolu_…`). Empty when the block omitted it.
    pub id: String,
}

/// Claude `Task`/`Agent` and Cursor `taskToolCall`/`agentToolCall` display names.
pub fn is_subagent_tool_name(name: &str) -> bool {
    matches!(name, "Task" | "Agent")
}

/// True when this tool call is a Task/Agent launched in the background —
/// the parent turn can emit `result` before the subagent finishes.
///
/// Accepts both Claude's `run_in_background` and Cursor's `runInBackground`.
pub fn is_background_subagent_tool(name: &str, input: &Value) -> bool {
    is_subagent_tool_name(name) && subagent_runs_in_background(input)
}

fn subagent_runs_in_background(input: &Value) -> bool {
    ["run_in_background", "runInBackground", "background"]
        .iter()
        .any(|key| input.get(*key).and_then(|v| v.as_bool()).unwrap_or(false))
}

/// One-line label for a Task/Agent spawn (description, else truncated prompt).
pub fn subagent_description(input: &Value) -> String {
    for key in ["description", "task", "title", "prompt"] {
        if let Some(d) = input.get(key).and_then(|v| v.as_str()) {
            let d = d.trim();
            if !d.is_empty() {
                let mut s: String = d.chars().take(80).collect();
                if d.chars().count() > 80 {
                    s.push('…');
                }
                return s;
            }
        }
    }
    "subagent".to_string()
}

/// Token usage for one chat turn, normalised to a single-iteration view.
///
/// Claude Code's `result.usage` is the **sum across all internal API
/// iterations** within a turn — when Claude makes N tool-call rounds,
/// each round re-sends the same cached prefix and the top-level
/// `cache_read_input_tokens` accumulates N copies of it. That sum is
/// useful for billing but is not the context window size.
///
/// To get a meaningful "context window used" indicator, the parser
/// looks at the per-iteration `iterations` array (when present) and
/// stores the **last iteration's** per-call values into the
/// `input_tokens` / `cache_creation_input_tokens` /
/// `cache_read_input_tokens` fields. Their sum (`prefix_tokens()`) is
/// then the actual prefix the model saw at the end of the turn —
/// bounded by the model's context window.
///
/// `output_tokens` stays as the summed top-level total ("tokens
/// generated this turn"), since output isn't re-sent across iterations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    /// Tokens the model was conditioned on at the end of the turn
    /// (fresh input + cache writes + cache reads, for the last
    /// iteration). Bounded by the model's context window.
    pub fn prefix_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
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
        /// Set on forwarded subagent messages (`--forward-subagent-text`).
        /// `None` for the parent conversation.
        parent_tool_use_id: Option<String>,
    },

    /// Background / subagent task began (`type: system`, `subtype: task_started`).
    TaskStarted {
        task_id: String,
        tool_use_id: String,
        description: String,
    },

    /// Background / subagent task reached a terminal state
    /// (`type: system`, `subtype: task_notification`).
    TaskNotification {
        task_id: String,
        tool_use_id: String,
        status: String,
        summary: String,
    },

    /// User message carrying `tool_result` blocks. In `--print` mode this is
    /// often the only completion signal for a subagent (no `task_notification`).
    UserToolResults { tool_use_ids: Vec<String> },

    /// Final result (type: "result").
    ResultEvent {
        is_error: bool,
        result_text: String,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        /// Server-reported token usage. `None` when the `usage` object was
        /// absent or unparseable. Present means Claude told us exactly how
        /// many tokens the session was conditioned on this turn.
        usage: Option<TokenUsage>,
    },

    /// Permission request — Claude wants to execute a tool and needs user approval.
    /// The pipeline pauses until the user approves or denies via stdin.
    ///
    /// Emitted for both the legacy `type: permission_request` NDJSON line and
    /// the modern control protocol (`type: control_request`,
    /// `request.subtype: can_use_tool`).
    PermissionRequest {
        tool_name: String,
        description: String,
        request_id: String,
        /// Tool arguments from the control request (empty object for legacy).
        input: Value,
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

fn nonempty_opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Parse a Claude Code `usage` object into a single-iteration
/// [`TokenUsage`].
///
/// When the object carries an `iterations` array (multi-tool-call
/// turns), the last entry's per-call prefix breakdown is used for
/// `input_tokens` / `cache_creation_input_tokens` /
/// `cache_read_input_tokens` so `prefix_tokens()` reflects the actual
/// context window used at end-of-turn — not the sum of every cached
/// re-send. `output_tokens` stays as the top-level total (output isn't
/// re-sent across iterations).
///
/// Falls back to top-level fields when `iterations` is absent or empty
/// (single-iteration turns, older Claude Code releases).
fn parse_usage_object(obj: &serde_json::Map<String, Value>) -> TokenUsage {
    let top_u64 = |k: &str| obj.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
    let last_iter = obj
        .get("iterations")
        .and_then(|i| i.as_array())
        .and_then(|arr| arr.last())
        .and_then(|it| it.as_object());

    let (input_tokens, cache_creation_input_tokens, cache_read_input_tokens) = match last_iter {
        Some(it) => {
            let get = |k: &str| it.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
            (
                get("input_tokens"),
                get("cache_creation_input_tokens"),
                get("cache_read_input_tokens"),
            )
        }
        None => (
            top_u64("input_tokens"),
            top_u64("cache_creation_input_tokens"),
            top_u64("cache_read_input_tokens"),
        ),
    };

    TokenUsage {
        input_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        output_tokens: top_u64("output_tokens"),
    }
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
            } else if subtype == "task_started" {
                Ok(StreamEvent::TaskStarted {
                    task_id: opt_str(&v, "task_id").to_string(),
                    tool_use_id: opt_str(&v, "tool_use_id").to_string(),
                    description: {
                        let d = opt_str(&v, "description");
                        if d.is_empty() {
                            "subagent".to_string()
                        } else {
                            d.to_string()
                        }
                    },
                })
            } else if subtype == "task_notification" {
                Ok(StreamEvent::TaskNotification {
                    task_id: opt_str(&v, "task_id").to_string(),
                    tool_use_id: opt_str(&v, "tool_use_id").to_string(),
                    status: opt_str(&v, "status").to_string(),
                    summary: opt_str(&v, "summary").to_string(),
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
            let parent_tool_use_id = nonempty_opt_str(&v, "parent_tool_use_id")
                .or_else(|| nonempty_opt_str(&message, "parent_tool_use_id"));

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
                            let id = block
                                .get("id")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            tool_uses.push(ToolUseInfo { name, input, id });
                        }
                        _ => {}
                    }
                }
            }

            Ok(StreamEvent::AssistantMessage {
                text,
                tool_uses,
                parent_tool_use_id,
            })
        }

        // User message: we only need tool_result ids (subagent completion in
        // --print mode). Other user lines pass through as Unknown.
        "user" => {
            let message = v.get("message").cloned().unwrap_or(Value::Null);
            let content = message.get("content").and_then(|c| c.as_array());
            let mut tool_use_ids = Vec::new();
            if let Some(blocks) = content {
                for block in blocks {
                    if opt_str(block, "type") == "tool_result"
                        && let Some(id) = block.get("tool_use_id").and_then(|t| t.as_str())
                        && !id.is_empty()
                    {
                        tool_use_ids.push(id.to_string());
                    }
                }
            }
            if tool_use_ids.is_empty() {
                Ok(StreamEvent::Unknown(v))
            } else {
                Ok(StreamEvent::UserToolResults { tool_use_ids })
            }
        }

        // Final result. The top-level `usage` is summed across the
        // `iterations` array (one entry per internal API call within
        // the turn); the last iteration's per-call breakdown is what
        // the model saw at end-of-turn. See `TokenUsage` docs.
        //
        // {"type":"result","subtype":"success","result":"...","duration_ms":1234,
        //  "cost_usd":0.01,
        //  "usage":{
        //    "input_tokens":..,                "cache_creation_input_tokens":..,
        //    "cache_read_input_tokens":..,     "output_tokens":..,
        //    "iterations":[{"input_tokens":..,"cache_creation_input_tokens":..,
        //                   "cache_read_input_tokens":..,"output_tokens":..}, ...]
        //  }}
        "result" => {
            // The top-level `is_error` flag is authoritative. Claude Code
            // reports an API failure with that flag set while `subtype` still
            // reads "success" — verified on 2.1.235, where an unknown
            // `--model` yields
            //   {"type":"result","is_error":true,"subtype":"success",
            //    "api_error_status":404,"result":"There's an issue with the
            //    selected model (…)","total_cost_usd":0}
            // Deriving the flag from `subtype` alone parsed those as clean
            // turns: the swarm runner saw an empty transcript, fell through to
            // the `produces` check, and blamed the agent for not writing its
            // declared output. `subtype == "error"` is kept as a fallback for
            // CLI versions that only set that.
            let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
                || v.get("subtype").and_then(|s| s.as_str()) == Some("error");
            let result_text = required_str(&v, "result")?;
            let duration_ms = v.get("duration_ms").and_then(|d| d.as_u64());
            let cost_usd = v.get("cost_usd").and_then(|c| c.as_f64());
            let usage = v
                .get("usage")
                .and_then(|u| u.as_object())
                .map(parse_usage_object);

            Ok(StreamEvent::ResultEvent {
                is_error,
                result_text,
                duration_ms,
                cost_usd,
                usage,
            })
        }

        // Permission request (legacy): Claude wants to run a tool and needs approval.
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
            let input = v
                .get("input")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            Ok(StreamEvent::PermissionRequest {
                tool_name,
                description,
                request_id,
                input,
            })
        }

        // Modern Agent SDK / CLI control protocol (Claude Code ≥2.1):
        // {"type":"control_request","request_id":"...","request":{"subtype":"can_use_tool",
        //   "tool_name":"Bash","input":{...},"description":"..."}}
        "control_request" => parse_control_request(&v),

        _ => Ok(StreamEvent::Unknown(v)),
    }
}

/// Parse a `control_request` envelope. Only `can_use_tool` is mapped to a
/// typed event; other subtypes (initialize ACKs from the CLI, interrupt, …)
/// pass through as [`StreamEvent::Unknown`].
fn parse_control_request(v: &Value) -> Result<StreamEvent> {
    let request = v.get("request").cloned().unwrap_or(Value::Null);
    let subtype = opt_str(&request, "subtype");
    if subtype != "can_use_tool" {
        return Ok(StreamEvent::Unknown(v.clone()));
    }

    let tool_name = opt_str(&request, "tool_name").to_string();
    let input = request
        .get("input")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let description = permission_description(&tool_name, &request, &input);
    let request_id = v
        .get("request_id")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    Ok(StreamEvent::PermissionRequest {
        tool_name,
        description,
        request_id,
        input,
    })
}

/// Build a short human-readable description for the TUI permission overlay.
pub fn permission_description(tool_name: &str, request: &Value, input: &Value) -> String {
    if let Some(d) = request.get("description").and_then(|d| d.as_str())
        && !d.is_empty()
    {
        return d.to_string();
    }
    if let Some(d) = input.get("description").and_then(|d| d.as_str())
        && !d.is_empty()
    {
        return d.to_string();
    }
    match tool_name {
        "Bash" => input
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("(bash)")
            .to_string(),
        "AskUserQuestion" => {
            let n = input
                .get("questions")
                .and_then(|q| q.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            format!(
                "Clarifying question{s} ({n})",
                s = if n == 1 { "" } else { "s" }
            )
        }
        "Write" | "Edit" | "MultiEdit" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .unwrap_or(tool_name)
            .to_string(),
        _ => tool_name.to_string(),
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
                if q + fence_len <= bytes.len() && bytes[q..q + fence_len].iter().all(|&c| c == fc)
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
    fn test_parse_task_started() {
        let line = r#"{"type":"system","subtype":"task_started","task_id":"task_abc","tool_use_id":"toolu_abc","description":"Running background analysis","task_type":"background"}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::TaskStarted {
                task_id,
                tool_use_id,
                description,
            } => {
                assert_eq!(task_id, "task_abc");
                assert_eq!(tool_use_id, "toolu_abc");
                assert_eq!(description, "Running background analysis");
            }
            _ => panic!("Expected TaskStarted, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_task_notification() {
        let line = r#"{"type":"system","subtype":"task_notification","task_id":"task_abc","tool_use_id":"toolu_abc","status":"completed","summary":"done","output_file":""}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::TaskNotification {
                task_id,
                status,
                summary,
                ..
            } => {
                assert_eq!(task_id, "task_abc");
                assert_eq!(status, "completed");
                assert_eq!(summary, "done");
            }
            _ => panic!("Expected TaskNotification, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_user_tool_results() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"ok"}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::UserToolResults { tool_use_ids } => {
                assert_eq!(tool_use_ids, vec!["toolu_abc"]);
            }
            _ => panic!("Expected UserToolResults, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_assistant_task_tool_use() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Task","input":{"description":"search papers","run_in_background":true,"prompt":"go"}}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::AssistantMessage { tool_uses, .. } => {
                assert_eq!(tool_uses.len(), 1);
                assert_eq!(tool_uses[0].id, "toolu_1");
                assert!(is_background_subagent_tool(
                    &tool_uses[0].name,
                    &tool_uses[0].input
                ));
                assert_eq!(subagent_description(&tool_uses[0].input), "search papers");
            }
            _ => panic!("Expected AssistantMessage, got {:?}", event),
        }
    }

    #[test]
    fn cursor_run_in_background_camel_case() {
        let input = serde_json::json!({
            "description": "scan docs",
            "runInBackground": true
        });
        assert!(is_subagent_tool_name("Task"));
        assert!(is_background_subagent_tool("Task", &input));
        assert_eq!(subagent_description(&input), "scan docs");
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
                usage,
            } => {
                assert!(!is_error);
                assert_eq!(result_text, "Done!");
                assert_eq!(duration_ms, Some(5000));
                assert!((cost_usd.unwrap() - 0.01).abs() < f64::EPSILON);
                assert!(usage.is_none(), "no usage object in line → None");
            }
            _ => panic!("Expected ResultEvent, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_result_honours_top_level_is_error_over_subtype() {
        // Real Claude Code 2.1.235 line for `--model sonnet-5` (unknown id):
        // `is_error` is true while `subtype` still says "success". Reading
        // only `subtype` made this parse as a successful, empty turn.
        let line = r#"{"is_error":true,"subtype":"success","api_error_status":404,
            "result":"There's an issue with the selected model (sonnet-5).",
            "total_cost_usd":0,"type":"result","duration_ms":1737}"#;
        match parse_stream_line(line).unwrap() {
            StreamEvent::ResultEvent {
                is_error,
                result_text,
                ..
            } => {
                assert!(is_error, "top-level is_error must win over subtype");
                assert!(result_text.contains("sonnet-5"));
            }
            other => panic!("expected ResultEvent, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_error_subtype_still_recognised() {
        // Fallback path for CLI versions that signal only through `subtype`.
        let line = r#"{"type":"result","subtype":"error","result":"boom"}"#;
        match parse_stream_line(line).unwrap() {
            StreamEvent::ResultEvent { is_error, .. } => assert!(is_error),
            other => panic!("expected ResultEvent, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_with_usage() {
        let line = r#"{"type":"result","subtype":"success","result":"ok",
            "duration_ms":1234,"cost_usd":0.05,
            "usage":{"input_tokens":1500,"cache_creation_input_tokens":3000,
                     "cache_read_input_tokens":42000,"output_tokens":200}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ResultEvent { usage: Some(u), .. } => {
                assert_eq!(u.input_tokens, 1500);
                assert_eq!(u.cache_creation_input_tokens, 3000);
                assert_eq!(u.cache_read_input_tokens, 42_000);
                assert_eq!(u.output_tokens, 200);
                assert_eq!(u.prefix_tokens(), 1500 + 3000 + 42_000);
            }
            other => panic!("expected ResultEvent with usage, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_partial_usage_defaults_zero() {
        // Forward-compat: if Claude drops or renames a field, treat absent as 0
        // rather than fail the whole event.
        let line = r#"{"type":"result","subtype":"success","result":"ok",
            "usage":{"input_tokens":100,"output_tokens":10}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ResultEvent { usage: Some(u), .. } => {
                assert_eq!(u.input_tokens, 100);
                assert_eq!(u.cache_creation_input_tokens, 0);
                assert_eq!(u.cache_read_input_tokens, 0);
                assert_eq!(u.output_tokens, 10);
            }
            other => panic!("expected ResultEvent with usage, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_uses_last_iteration_prefix_not_summed_total() {
        // Claude Code's top-level `usage` is summed across `iterations`.
        // When a turn includes N tool-call rounds each carrying the same
        // ~50K cached prefix, the top-level `cache_read_input_tokens`
        // accumulates N copies. Picking that up as "context window
        // used" produces values orders of magnitude larger than the
        // model's actual context — e.g. 5083K on a 200K-window model.
        //
        // The parser must use the **last iteration's** per-call breakdown
        // instead, so `prefix_tokens()` stays bounded by the context
        // window.
        let line = r#"{"type":"result","subtype":"success","result":"ok",
            "usage":{
                "input_tokens":50, "cache_creation_input_tokens":12000,
                "cache_read_input_tokens":150000, "output_tokens":300,
                "iterations":[
                    {"input_tokens":20,"cache_creation_input_tokens":12000,
                     "cache_read_input_tokens":30000,"output_tokens":100},
                    {"input_tokens":15,"cache_creation_input_tokens":0,
                     "cache_read_input_tokens":42000,"output_tokens":80},
                    {"input_tokens":15,"cache_creation_input_tokens":0,
                     "cache_read_input_tokens":78000,"output_tokens":120}
                ]
            }}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ResultEvent { usage: Some(u), .. } => {
                // Last iteration's prefix — what the model actually saw.
                assert_eq!(u.input_tokens, 15);
                assert_eq!(u.cache_creation_input_tokens, 0);
                assert_eq!(u.cache_read_input_tokens, 78_000);
                assert_eq!(u.prefix_tokens(), 78_015);
                // output stays as the summed top-level total — output
                // isn't re-sent across iterations, so summing is fine.
                assert_eq!(u.output_tokens, 300);
            }
            other => panic!("expected ResultEvent with usage, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_empty_iterations_falls_back_to_top_level() {
        // Defensive: if the array is empty for some reason, treat the
        // usage as a single-iteration turn and read top-level fields.
        let line = r#"{"type":"result","subtype":"success","result":"ok",
            "usage":{
                "input_tokens":5, "cache_creation_input_tokens":12735,
                "cache_read_input_tokens":17922, "output_tokens":8,
                "iterations":[]
            }}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            StreamEvent::ResultEvent { usage: Some(u), .. } => {
                assert_eq!(u.input_tokens, 5);
                assert_eq!(u.cache_creation_input_tokens, 12_735);
                assert_eq!(u.cache_read_input_tokens, 17_922);
                assert_eq!(u.output_tokens, 8);
            }
            other => panic!("expected ResultEvent with usage, got {:?}", other),
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

    #[test]
    fn test_parse_legacy_permission_request() {
        let line = r#"{"type":"permission_request","tool_name":"Bash","description":"Run ls","id":"req_1"}"#;
        let ev = parse_stream_line(line).unwrap();
        match ev {
            StreamEvent::PermissionRequest {
                tool_name,
                description,
                request_id,
                input,
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(description, "Run ls");
                assert_eq!(request_id, "req_1");
                assert!(input.as_object().unwrap().is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_parse_control_request_can_use_tool() {
        let line = r#"{"type":"control_request","request_id":"8cf30a4a-ff9d-4066-87bd-2f2e76c904eb","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Which?","options":[{"label":"A","description":"a"}]}]},"description":"pick one"}}"#;
        let ev = parse_stream_line(line).unwrap();
        match ev {
            StreamEvent::PermissionRequest {
                tool_name,
                description,
                request_id,
                input,
            } => {
                assert_eq!(tool_name, "AskUserQuestion");
                assert_eq!(description, "pick one");
                assert_eq!(request_id, "8cf30a4a-ff9d-4066-87bd-2f2e76c904eb");
                assert!(input.get("questions").is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_parse_control_request_other_subtype_is_unknown() {
        let line =
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"interrupt"}}"#;
        let ev = parse_stream_line(line).unwrap();
        assert!(matches!(ev, StreamEvent::Unknown(_)));
    }

    #[test]
    fn test_permission_description_bash_falls_back_to_command() {
        let input = serde_json::json!({"command": "echo hi"});
        let request = serde_json::json!({});
        assert_eq!(permission_description("Bash", &request, &input), "echo hi");
    }
}
