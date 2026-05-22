//! Cursor CLI subprocess backend.
//!
//! Spawns `agent -p --output-format stream-json --stream-partial-output …`
//! and maps the NDJSON event stream into normalized [`UnifiedStreamEvent`]s.
//!
//! The Cursor CLI emits its own native tool calls (`read`, `write`, `edit`,
//! `bash`, `search`) which write directly to disk in `-p`/`--print` mode
//! — there is no "propose-only" headless mode (`-f`/`--force` only auto-
//! approves Bash commands, not write proposals).
//!
//! **Swarm path safety:** in the swarm pipeline the agent runs inside a
//! per-agent worktree, so direct disk writes are bounded by the worktree
//! and merged later — same pattern Claude uses. `Capabilities` therefore
//! advertises `supports_file_blocks=false` and the backend never emits
//! `FileBlock` events.
//!
//! **Chat path:** the chat path uses [`crate::agent_session::cursor::CursorSession`]
//! which re-parses the same NDJSON with full tool-args fidelity (see
//! [`CursorEvent`] below) so it can snapshot files before tool calls and
//! revert them after the stream ends, routing the proposed content through
//! the Write Gate.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use futures::Stream;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_stream::wrappers::ReceiverStream;

use super::shared::{build_enriched_prompt, default_editor_system_prompt};
use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
};

/// Default Cursor model when no model is supplied with the `cursor:`
/// prefix. `composer-2.5` is Cursor's current general-purpose default
/// (paid plans). Free-tier accounts must explicitly opt in to
/// `cursor:auto` since named models reject with "Named models
/// unavailable Free plans can only use Auto" on free accounts.
pub(crate) const DEFAULT_CURSOR_MODEL: &str = "composer-2.5";

/// The Cursor CLI takes its prompt as a positional argv (no `--prompt-file`
/// or stdin fallback documented). Linux's `MAX_ARG_STRLEN` is 128 KB per
/// argument; 96 KB leaves headroom for the rest of the argv (flags,
/// `--workspace`, model spec, etc.). Prompts above this size are rejected
/// with an explicit error rather than truncated.
pub(crate) const CURSOR_ARGV_LIMIT: usize = 96 * 1024;

/// Backend that spawns the Cursor CLI as a subprocess.
pub struct CursorBackend {
    model: String,
    display_name: String,
}

impl CursorBackend {
    pub fn new(model: &str) -> Self {
        let m = if model.is_empty() {
            DEFAULT_CURSOR_MODEL
        } else {
            model
        };
        Self {
            model: m.to_string(),
            display_name: format!("cursor:{}", m),
        }
    }
}

#[async_trait::async_trait]
impl AgentBackend for CursorBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let system_prompt = request
            .system_prompt
            .clone()
            .unwrap_or_else(|| default_editor_system_prompt(&self.capabilities()));

        let user_prompt = build_enriched_prompt(
            &request.prompt,
            &request.conversation_history,
            &request.file_refs,
        );

        let combined_prompt = format!("{system_prompt}\n\n{user_prompt}");

        if combined_prompt.len() >= CURSOR_ARGV_LIMIT {
            anyhow::bail!(
                "cursor prompt is {} bytes which exceeds the {}-byte argv limit. \
                 The `agent` CLI has no stdin or `--prompt-file` fallback, so \
                 trim the user message, the context bundle, or switch to a provider \
                 with stdin support (claude, codex, ollama).",
                combined_prompt.len(),
                CURSOR_ARGV_LIMIT,
            );
        }

        let mut cmd = Command::new("agent");
        for arg in cursor_argv(&self.model, &request.workspace_root, None) {
            cmd.arg(arg);
        }
        cmd.arg(&combined_prompt);

        cmd.current_dir(&request.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let prompt_len = combined_prompt.len();
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "spawning cursor `agent` subprocess: {e}\n\
                     The `agent` CLI binary was not found on PATH. \
                     Install it from https://cursor.com/cli (curl https://cursor.com/install -fsS | bash), \
                     or switch provider by setting agent.model to a `claude:...` / `codex:...` / `ollama:...` spec."
                )
            } else {
                anyhow::anyhow!(
                    "spawning cursor `agent` subprocess (prompt {} bytes): {e}",
                    prompt_len,
                )
            }
        })?;

        let stdout = child.stdout.take().context("cursor stdout unavailable")?;
        let stderr = child.stderr.take().context("cursor stderr unavailable")?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<UnifiedStreamEvent>>(64);

        // Drain stderr concurrently so the buffer doesn't fill and block the subprocess.
        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let result = drive_cursor_stdout_unified(stdout, tx_clone.clone()).await;
            let exit_status = child.wait().await;
            let stderr_text = stderr_handle.await.unwrap_or_default();

            let duration_ms = Some(start.elapsed().as_millis() as u64);

            match result {
                Ok(()) => {
                    let ok = exit_status.as_ref().map(|s| s.success()).unwrap_or(false);
                    if ok {
                        // If the result event already supplied a Usage event,
                        // a second one here would double-count duration. Send
                        // a duration-only Usage so the UI still sees timing
                        // even on the (unlikely) path where the stream ended
                        // without a `result` event.
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Usage(TokenUsage {
                                duration_ms,
                                ..Default::default()
                            })))
                            .await;
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)))
                            .await;
                    } else {
                        let msg = format_exit_error(&exit_status, &stderr_text);
                        let _ = tx_clone.send(Ok(UnifiedStreamEvent::Error(msg))).await;
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                            .await;
                    }
                }
                Err(e) => {
                    let _ = tx_clone
                        .send(Ok(UnifiedStreamEvent::Error(format!("{e:#}"))))
                        .await;
                    let _ = tx_clone
                        .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                        .await;
                }
            }
        });

        drop(tx);
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
            extended_thinking: true,
            // Cursor's hosted models vary by account; 200k is a safe upper
            // bound matching the Claude / Codex defaults. Future milestones
            // may refine per-model from `--list-models` output.
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            // Cursor uses native tool calls only — no in-band <file> block
            // channel. The chat path (CursorSession) handles edits via
            // snapshot+revert; the swarm path relies on worktree isolation.
            supports_file_blocks: false,
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let output = Command::new("agent")
            .arg("--version")
            .output()
            .await
            .context("cursor `agent` binary not found on PATH")?;
        if output.status.success() {
            Ok(())
        } else {
            anyhow::bail!("agent --version exited with {}", output.status)
        }
    }
}

/// Build the argv the Cursor CLI runs with, excluding the trailing prompt
/// positional. Extracted so the chat path (`CursorSession`) can reuse the
/// exact same flag composition, and so tests can pin the argv without
/// spawning a subprocess.
pub(crate) fn cursor_argv(
    model: &str,
    workspace_root: &Path,
    resume_session_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--stream-partial-output".to_string(),
        // `--trust` is required in headless mode for the workspace to be
        // writable; we re-trust the cwd here because gaviero already gave
        // its own workspace consent at the host layer.
        "--trust".to_string(),
        "--workspace".to_string(),
        workspace_root.to_string_lossy().into_owned(),
        "--model".to_string(),
        if model.is_empty() {
            DEFAULT_CURSOR_MODEL.to_string()
        } else {
            model.to_string()
        },
    ];

    if let Some(id) = resume_session_id
        && !id.is_empty()
    {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }

    args
}

/// Drive the subprocess stdout and translate every parsed [`CursorEvent`]
/// into a [`UnifiedStreamEvent`]. The chat path bypasses this and consumes
/// `CursorEvent` directly via [`drive_cursor_stdout_events`] so it can
/// snapshot tool-call arguments.
async fn drive_cursor_stdout_unified(
    stdout: tokio::process::ChildStdout,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<()> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await.context("reading cursor stdout")? {
        let Some(event) = parse_cursor_event(&line) else {
            continue;
        };
        for unified in cursor_event_to_unified(event) {
            if tx.send(Ok(unified)).await.is_err() {
                return Ok(()); // receiver dropped
            }
        }
    }
    Ok(())
}

/// Map a [`CursorEvent`] into the zero-or-more [`UnifiedStreamEvent`]s the
/// swarm executor consumes. The result event splits into `Usage` then
/// `Done` so the upstream `complete_to_write_gate` flow gets both signals.
fn cursor_event_to_unified(event: CursorEvent) -> Vec<UnifiedStreamEvent> {
    match event {
        CursorEvent::SystemInit { .. } => vec![],
        CursorEvent::UserEcho => vec![],
        CursorEvent::AssistantDelta(text) => vec![UnifiedStreamEvent::TextDelta(text)],
        // Drop the consolidated `assistant` event (no `timestamp_ms`) to
        // avoid double-printing the same segment we already streamed via
        // deltas.
        CursorEvent::AssistantSegment(_) => vec![],
        CursorEvent::ThinkingDelta(text) => vec![UnifiedStreamEvent::ThinkingDelta(text)],
        CursorEvent::ThinkingCompleted => vec![],
        CursorEvent::ToolCallStarted { id, name, args_json } => {
            let args = serde_json::from_str::<Value>(&args_json).unwrap_or(Value::Null);
            vec![UnifiedStreamEvent::ToolCallStart {
                id,
                name: tool_display_name(&name),
                args,
            }]
        }
        CursorEvent::ToolCallCompleted { id, .. } => {
            vec![UnifiedStreamEvent::ToolCallEnd { id }]
        }
        CursorEvent::ResultSuccess { usage, .. } => vec![
            UnifiedStreamEvent::Usage(usage.unwrap_or_default()),
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ],
        CursorEvent::ResultError { message, usage } => match usage {
            Some(u) => vec![
                UnifiedStreamEvent::Usage(u),
                UnifiedStreamEvent::Error(message),
                UnifiedStreamEvent::Done(StopReason::Error),
            ],
            None => vec![
                UnifiedStreamEvent::Error(message),
                UnifiedStreamEvent::Done(StopReason::Error),
            ],
        },
    }
}

/// Map a Cursor tool key (e.g. `editToolCall`) to a Claude-style display
/// name (`Edit`) so the UI's tool-name vocabulary stays provider-agnostic.
/// Unknown keys are PascalCased with the `ToolCall` suffix stripped.
pub(crate) fn tool_display_name(key: &str) -> String {
    let stripped = key.strip_suffix("ToolCall").unwrap_or(key);
    match stripped {
        "read" => "Read".into(),
        "write" => "Write".into(),
        "edit" => "Edit".into(),
        "bash" => "Bash".into(),
        "search" => "Search".into(),
        "glob" => "Glob".into(),
        "grep" => "Grep".into(),
        "" => "Tool".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => "Tool".into(),
            }
        }
    }
}

fn format_exit_error(
    exit_status: &std::io::Result<std::process::ExitStatus>,
    stderr_text: &str,
) -> String {
    let status_line = match exit_status {
        Ok(s) => format!("cursor `agent` exited with {s}"),
        Err(e) => format!("failed to wait for cursor `agent`: {e}"),
    };
    let auth_hint = if is_auth_error(stderr_text) {
        "\nTo re-authenticate, run `agent login` in a terminal, then retry."
    } else {
        ""
    };
    if stderr_text.trim().is_empty() {
        format!(
            "{status_line}\nCheck that the `agent` CLI is installed and `agent whoami` reports a logged-in account.{auth_hint}"
        )
    } else {
        format!("{status_line}\n{}{}", stderr_text.trim(), auth_hint)
    }
}

/// Detect Cursor auth failures in stderr text so the error surface can
/// suggest `agent login`. Conservative pattern match — false negatives are
/// fine (the user still sees the raw error), false positives would
/// mislead so we keep the substrings narrow.
pub(crate) fn is_auth_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("not logged in")
        || lower.contains("please log in")
        || lower.contains("login required")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
}

// ── Parser (shared with the chat-path session) ────────────────────────────────

/// Snapshot of a parsed Cursor NDJSON event with enough fidelity for both
/// the swarm path (which discards tool args) and the chat path (which
/// snapshots files based on `started` events).
///
/// `pub(crate)` — consumed by `agent_session::cursor` and the unit tests in
/// this file. Do not widen the visibility unless a clear cross-crate
/// consumer appears.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CursorEvent {
    /// `{"type":"system","subtype":"init",session_id,model,...}`
    SystemInit { session_id: String, model: String },
    /// `{"type":"user",message:{...},session_id}` — echo of the prompt we
    /// just sent. Discarded by both consumers; kept here so the parser
    /// reports unknown variants accurately.
    UserEcho,
    /// `{"type":"assistant","message":{...,"content":[{"text":"..."}]},timestamp_ms,...}`
    /// — incremental text delta when `--stream-partial-output` is enabled.
    AssistantDelta(String),
    /// `{"type":"assistant","message":{...}}` *without* `timestamp_ms` —
    /// the consolidated segment summary emitted after the deltas. Consumers
    /// must drop this to avoid double-printing.
    AssistantSegment(String),
    /// `{"type":"thinking","subtype":"delta","text":"..."}`
    ThinkingDelta(String),
    /// `{"type":"thinking","subtype":"completed"}`
    ThinkingCompleted,
    /// `{"type":"tool_call","subtype":"started",call_id,tool_call:{<key>:{args:{...}}}}`
    ///
    /// `args_json` is the raw `args` object as serialized JSON so the chat
    /// path can pull `path` / `streamContent` / `fileText` without this
    /// parser pinning every tool schema. `name` is the tool object key
    /// (e.g. `editToolCall`).
    ToolCallStarted {
        id: String,
        name: String,
        args_json: String,
    },
    /// `{"type":"tool_call","subtype":"completed",call_id,tool_call:{<key>:{result:{...}}}}`
    ToolCallCompleted {
        id: String,
        name: String,
        result_json: String,
    },
    /// Terminal `{"type":"result","subtype":"success",result,usage,...}`.
    ResultSuccess {
        text: String,
        usage: Option<TokenUsage>,
        session_id: String,
    },
    /// Terminal `{"type":"result",is_error:true,...}` (or `subtype:"error"`).
    ResultError {
        message: String,
        usage: Option<TokenUsage>,
    },
}

/// Parse one NDJSON line. Returns `None` for blank lines, non-JSON output
/// (`agent` emits informational stderr-style lines like `"S: Named models
/// unavailable …"` and a trailing `"Shell cwd was reset to …"` postscript
/// to stdout), and JSON shapes we deliberately ignore.
///
/// The function never panics — malformed input downgrades to `None` and
/// is logged at `debug` level so a backend update can be diagnosed
/// without crashing the user's turn.
pub(crate) fn parse_cursor_event(line: &str) -> Option<CursorEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        // Postscript / status / informational lines — log at trace so we
        // don't drown the user when cursor's CLI gets chatty.
        if !trimmed.is_empty() {
            tracing::trace!(target: "backend.cursor", "non-json stdout: {}", trimmed);
        }
        return None;
    }

    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(target: "backend.cursor", "skipping malformed json: {} ({})", trimmed, e);
            return None;
        }
    };

    let kind = value.get("type")?.as_str()?;
    match kind {
        "system" => parse_system(&value),
        "user" => Some(CursorEvent::UserEcho),
        "assistant" => parse_assistant(&value),
        "thinking" => parse_thinking(&value),
        "tool_call" => parse_tool_call(&value),
        "result" => parse_result(&value),
        other => {
            tracing::debug!(target: "backend.cursor", "ignoring unknown event type: {}", other);
            None
        }
    }
}

fn parse_system(value: &Value) -> Option<CursorEvent> {
    if value.get("subtype")?.as_str()? != "init" {
        return None;
    }
    let session_id = value.get("session_id")?.as_str()?.to_string();
    let model = value
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    Some(CursorEvent::SystemInit { session_id, model })
}

fn parse_assistant(value: &Value) -> Option<CursorEvent> {
    let text = value
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                item.get("text").and_then(|t| t.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    let is_delta = value.get("timestamp_ms").is_some();
    if is_delta {
        Some(CursorEvent::AssistantDelta(text))
    } else {
        Some(CursorEvent::AssistantSegment(text))
    }
}

fn parse_thinking(value: &Value) -> Option<CursorEvent> {
    match value.get("subtype")?.as_str()? {
        "delta" => {
            let text = value.get("text")?.as_str()?.to_string();
            Some(CursorEvent::ThinkingDelta(text))
        }
        "completed" => Some(CursorEvent::ThinkingCompleted),
        other => {
            tracing::debug!(target: "backend.cursor", "ignoring thinking subtype: {}", other);
            None
        }
    }
}

fn parse_tool_call(value: &Value) -> Option<CursorEvent> {
    let subtype = value.get("subtype")?.as_str()?;
    let id = value.get("call_id")?.as_str()?.to_string();
    let tool_obj = value.get("tool_call")?.as_object()?;

    // The tool_call object has exactly one key like `editToolCall`,
    // `readToolCall`, `writeToolCall`, etc. Pick the first one
    // defensively rather than panicking.
    let (name, body) = tool_obj.iter().next()?;
    match subtype {
        "started" => {
            let args = body.get("args").cloned().unwrap_or(Value::Null);
            Some(CursorEvent::ToolCallStarted {
                id,
                name: name.clone(),
                args_json: args.to_string(),
            })
        }
        "completed" => {
            let result = body.get("result").cloned().unwrap_or(Value::Null);
            Some(CursorEvent::ToolCallCompleted {
                id,
                name: name.clone(),
                result_json: result.to_string(),
            })
        }
        other => {
            tracing::debug!(target: "backend.cursor", "ignoring tool_call subtype: {}", other);
            None
        }
    }
}

fn parse_result(value: &Value) -> Option<CursorEvent> {
    let subtype = value.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
    let is_error = value
        .get("is_error")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let usage = value.get("usage").and_then(parse_usage);
    let result_text = value
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = value
        .get("session_id")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    if subtype == "error" || is_error {
        let message = if result_text.is_empty() {
            "cursor `agent` returned an error".to_string()
        } else {
            result_text
        };
        Some(CursorEvent::ResultError { message, usage })
    } else {
        Some(CursorEvent::ResultSuccess {
            text: result_text,
            usage,
            session_id,
        })
    }
}

fn parse_usage(value: &Value) -> Option<TokenUsage> {
    let obj = value.as_object()?;
    let input = obj
        .get("inputTokens")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let output = obj
        .get("outputTokens")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 && !obj.contains_key("inputTokens") {
        return None;
    }
    Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cost_usd: None,
        duration_ms: None,
    })
}

/// Pull `path` and the proposed file content out of a serialized
/// `ToolCallStarted::args_json` for the write / edit tools. Returns
/// `None` for tools that don't propose file content (read, bash, search).
///
/// `pub(crate)` so the chat-path session can snapshot files at `started`
/// time without re-implementing the per-tool argument shape.
pub(crate) fn write_tool_args(name: &str, args_json: &str) -> Option<(PathBuf, String)> {
    let value: Value = serde_json::from_str(args_json).ok()?;
    let obj = value.as_object()?;

    // Cursor's write/edit tools both put the absolute or workspace-relative
    // path under `path`. Content lives under different keys depending on
    // the tool — `fileText` for `writeToolCall`, `streamContent` for
    // `editToolCall` — but we accept either so a future tool variant that
    // mixes the keys still snapshots correctly.
    let path = obj.get("path").and_then(|v| v.as_str())?;
    let content = obj
        .get("fileText")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("streamContent").and_then(|v| v.as_str()))
        .or_else(|| obj.get("content").and_then(|v| v.as_str()))?;

    match name {
        "writeToolCall" | "editToolCall" => Some((PathBuf::from(path), content.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn backend_name_carries_model_prefix() {
        let b = CursorBackend::new("auto");
        assert!(b.name().starts_with("cursor:"));
        assert!(b.name().contains("auto"));
    }

    #[test]
    fn empty_model_falls_back_to_default() {
        let b = CursorBackend::new("");
        assert!(b.name().ends_with(DEFAULT_CURSOR_MODEL));
    }

    #[test]
    fn capabilities_disable_in_band_file_blocks() {
        let b = CursorBackend::new("auto");
        let caps = b.capabilities();
        assert!(caps.tool_use);
        assert!(caps.streaming);
        assert!(caps.supports_system_prompt);
        assert!(!caps.supports_file_blocks);
    }

    #[test]
    fn argv_contains_print_stream_trust_and_workspace() {
        let workspace = PathBuf::from("/tmp/wt");
        let args = cursor_argv("auto", &workspace, None);
        // Argv pins:
        //   * `-p` / stream-json / --stream-partial-output for the NDJSON
        //     contract the parser depends on,
        //   * `--trust` so headless writes aren't blocked by the workspace
        //     trust prompt,
        //   * `--workspace` so the agent's cwd survives shell-cwd-reset
        //     postscripts.
        assert!(args.iter().any(|a| a == "-p"));
        assert!(
            args.windows(2)
                .any(|w| w == ["--output-format", "stream-json"])
        );
        assert!(args.iter().any(|a| a == "--stream-partial-output"));
        assert!(args.iter().any(|a| a == "--trust"));
        assert!(args.windows(2).any(|w| w[0] == "--workspace"));
        assert!(args.windows(2).any(|w| w == ["--model", "auto"]));
        assert!(!args.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn argv_appends_resume_when_session_id_present() {
        let args = cursor_argv("auto", Path::new("/tmp/wt"), Some("abc-123"));
        assert!(args.windows(2).any(|w| w == ["--resume", "abc-123"]));
    }

    #[test]
    fn argv_skips_resume_for_empty_session_id() {
        let args = cursor_argv("auto", Path::new("/tmp/wt"), Some(""));
        assert!(!args.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn parse_system_init() {
        let line = r#"{"type":"system","subtype":"init","apiKeySource":"login","cwd":"/tmp","session_id":"s-1","model":"Auto","permissionMode":"default"}"#;
        let ev = parse_cursor_event(line).expect("should parse");
        assert_eq!(
            ev,
            CursorEvent::SystemInit {
                session_id: "s-1".into(),
                model: "Auto".into()
            }
        );
    }

    #[test]
    fn parse_assistant_delta_keeps_timestamp_signal() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]},"session_id":"s-1","timestamp_ms":1779194246942}"#;
        let ev = parse_cursor_event(line).expect("should parse");
        assert_eq!(ev, CursorEvent::AssistantDelta("hi".into()));
    }

    #[test]
    fn parse_assistant_consolidated_segment_when_no_timestamp() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi42"}]},"session_id":"s-1"}"#;
        let ev = parse_cursor_event(line).expect("should parse");
        assert_eq!(ev, CursorEvent::AssistantSegment("hi42".into()));
    }

    #[test]
    fn parse_thinking_delta_and_completed() {
        let delta = r#"{"type":"thinking","subtype":"delta","text":"reasoning","session_id":"s-1","timestamp_ms":1}"#;
        let done = r#"{"type":"thinking","subtype":"completed","session_id":"s-1","timestamp_ms":1}"#;
        assert_eq!(
            parse_cursor_event(delta),
            Some(CursorEvent::ThinkingDelta("reasoning".into()))
        );
        assert_eq!(parse_cursor_event(done), Some(CursorEvent::ThinkingCompleted));
    }

    #[test]
    fn parse_tool_call_started_carries_args() {
        let line = r#"{"type":"tool_call","subtype":"started","call_id":"c-1","tool_call":{"editToolCall":{"args":{"path":"/tmp/x.txt","streamContent":"hello\n"}}},"session_id":"s-1","timestamp_ms":1}"#;
        let ev = parse_cursor_event(line).expect("should parse");
        let CursorEvent::ToolCallStarted { id, name, args_json } = ev else {
            panic!("expected ToolCallStarted");
        };
        assert_eq!(id, "c-1");
        assert_eq!(name, "editToolCall");
        let parsed: Value = serde_json::from_str(&args_json).unwrap();
        assert_eq!(parsed.get("path").unwrap().as_str().unwrap(), "/tmp/x.txt");
        assert_eq!(
            parsed.get("streamContent").unwrap().as_str().unwrap(),
            "hello\n"
        );
    }

    #[test]
    fn parse_result_success_extracts_usage_and_text() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":7948,"duration_api_ms":7948,"is_error":false,"result":"done","session_id":"s-1","request_id":"r-1","usage":{"inputTokens":10,"outputTokens":3,"cacheReadTokens":0,"cacheWriteTokens":0}}"#;
        let ev = parse_cursor_event(line).expect("should parse");
        let CursorEvent::ResultSuccess { text, usage, session_id } = ev else {
            panic!("expected ResultSuccess");
        };
        assert_eq!(text, "done");
        assert_eq!(session_id, "s-1");
        let usage = usage.expect("usage present");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 3);
    }

    #[test]
    fn parse_result_error_uses_result_text_or_default_message() {
        let with_text = r#"{"type":"result","is_error":true,"result":"bad model","session_id":"s-1"}"#;
        let CursorEvent::ResultError { message, .. } =
            parse_cursor_event(with_text).expect("should parse")
        else {
            panic!("expected ResultError");
        };
        assert_eq!(message, "bad model");

        let no_text = r#"{"type":"result","subtype":"error","session_id":"s-1"}"#;
        let CursorEvent::ResultError { message, .. } =
            parse_cursor_event(no_text).expect("should parse")
        else {
            panic!("expected ResultError");
        };
        assert!(message.contains("cursor"));
    }

    #[test]
    fn parse_skips_non_json_noise() {
        // The CLI emits status/postscript lines that are not NDJSON. The
        // parser must drop them rather than bailing out the whole turn.
        assert!(parse_cursor_event("").is_none());
        assert!(parse_cursor_event("Shell cwd was reset to /home/x").is_none());
        assert!(parse_cursor_event("S: Named models unavailable …").is_none());
        assert!(parse_cursor_event("not even close to json").is_none());
        assert!(parse_cursor_event("{ malformed json").is_none());
    }

    #[test]
    fn parse_skips_unknown_event_types() {
        let line = r#"{"type":"newfangled","session_id":"s-1"}"#;
        assert!(parse_cursor_event(line).is_none());
    }

    #[test]
    fn tool_display_name_maps_known_keys() {
        assert_eq!(tool_display_name("readToolCall"), "Read");
        assert_eq!(tool_display_name("writeToolCall"), "Write");
        assert_eq!(tool_display_name("editToolCall"), "Edit");
        assert_eq!(tool_display_name("bashToolCall"), "Bash");
        assert_eq!(tool_display_name("searchToolCall"), "Search");
    }

    #[test]
    fn tool_display_name_pascal_cases_unknown_keys() {
        assert_eq!(tool_display_name("planToolCall"), "Plan");
        assert_eq!(tool_display_name("noSuffix"), "NoSuffix");
    }

    #[test]
    fn write_tool_args_extracts_path_and_content_from_edit() {
        let args = r#"{"path":"/tmp/x.txt","streamContent":"hello\n"}"#;
        let (path, content) = write_tool_args("editToolCall", args).expect("edit returns path");
        assert_eq!(path, PathBuf::from("/tmp/x.txt"));
        assert_eq!(content, "hello\n");
    }

    #[test]
    fn write_tool_args_extracts_path_and_content_from_write() {
        let args = r#"{"path":"a/b.rs","fileText":"fn main() {}"}"#;
        let (path, content) = write_tool_args("writeToolCall", args).expect("write returns path");
        assert_eq!(path, PathBuf::from("a/b.rs"));
        assert_eq!(content, "fn main() {}");
    }

    #[test]
    fn write_tool_args_returns_none_for_non_write_tools() {
        let args = r#"{"path":"x"}"#;
        assert!(write_tool_args("readToolCall", args).is_none());
        assert!(write_tool_args("bashToolCall", args).is_none());
    }

    #[test]
    fn write_tool_args_returns_none_when_content_missing() {
        let args = r#"{"path":"a/b.rs"}"#;
        assert!(write_tool_args("writeToolCall", args).is_none());
    }

    #[test]
    fn is_auth_error_catches_common_phrases() {
        assert!(is_auth_error("Not logged in"));
        assert!(is_auth_error("please log in"));
        assert!(is_auth_error("Unauthorized"));
        assert!(is_auth_error("Invalid API key"));
        assert!(!is_auth_error("network error"));
        assert!(!is_auth_error(""));
    }

    #[test]
    fn cursor_event_to_unified_drops_consolidated_assistant() {
        // Sanity check that the unified mapping respects the dedup rule the
        // swarm path depends on — without this the user sees every segment
        // twice in the swarm dashboard.
        let delta = CursorEvent::AssistantDelta("hi".into());
        let segment = CursorEvent::AssistantSegment("hi".into());
        assert_eq!(
            cursor_event_to_unified(delta),
            vec![UnifiedStreamEvent::TextDelta("hi".into())]
        );
        assert_eq!(cursor_event_to_unified(segment), vec![]);
    }

    #[test]
    fn cursor_event_to_unified_emits_usage_then_done_on_success() {
        let ev = CursorEvent::ResultSuccess {
            text: "ok".into(),
            usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cost_usd: None,
                duration_ms: None,
            }),
            session_id: "s-1".into(),
        };
        let out = cursor_event_to_unified(ev);
        assert!(matches!(out[0], UnifiedStreamEvent::Usage(_)));
        assert!(matches!(
            out[1],
            UnifiedStreamEvent::Done(StopReason::EndTurn)
        ));
    }

    #[test]
    fn cursor_event_to_unified_emits_error_and_done_on_failure() {
        let ev = CursorEvent::ResultError {
            message: "boom".into(),
            usage: None,
        };
        let out = cursor_event_to_unified(ev);
        // Error event must precede Done so the executor records the
        // failure message before short-circuiting on Done(Error).
        assert!(matches!(out[0], UnifiedStreamEvent::Error(_)));
        assert!(matches!(out[1], UnifiedStreamEvent::Done(StopReason::Error)));
    }
}
