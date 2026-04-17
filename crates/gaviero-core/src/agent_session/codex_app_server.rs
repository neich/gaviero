//! Codex `app-server` session (V9 §11 M8).
//!
//! [`CodexAppServerSession`] implements [`AgentSession`] by keeping a single
//! `codex app-server --listen stdio://` subprocess alive across turns.
//! This achieves `ProcessBound` continuity: the model retains in-context state
//! for the lifetime of the subprocess.
//!
//! # Protocol (JSON-RPC 2.0 over NDJSON stdio)
//!
//! The `codex app-server` protocol is JSON-RPC 2.0 (not bare NDJSON with a
//! `"type"` field). **Note: V9 §6's event-mapping table uses shorthand names
//! that do not match the real method names exactly — this file implements
//! the actual protocol sourced from the openai/codex README.**
//!
//! ## Startup handshake
//! ```text
//! → {"method":"initialize","id":0,"params":{"clientInfo":{"name":"gaviero"}}}
//! ← {"id":0,"result":{"userAgent":"...","codexHome":"...",...}}
//! → {"method":"initialized","params":{}}
//! → {"method":"thread/start","id":1,"params":{"model":"...","cwd":"...","approvalPolicy":"never"}}
//! ←  (response) + {"method":"thread/started","params":{"thread":{"id":"<threadId>","status":"idle"}}}
//! ```
//! When a `CodexThreadId` continuity handle exists, `thread/resume` is sent
//! instead of `thread/start`.
//!
//! ## Per turn
//! ```text
//! → {"method":"turn/start","id":N,"params":{"threadId":"...","input":[{"type":"text","text":"..."}]}}
//! ← {"method":"turn/started","params":{"turn":{"id":"...","status":"inProgress"}}}
//! ← {"method":"item/started","params":{"item":{"type":"agentMessage","id":"...","text":""}}}
//! ← {"method":"item/agentMessage/delta","params":{"itemId":"...","delta":"..."}}
//! ← {"method":"item/completed","params":{"item":{"type":"agentMessage","id":"...","text":"..."}}}
//! ← {"method":"turn/completed","params":{"turn":{"status":"completed","tokenUsage":{...}}}}
//! ```
//!
//! ## Event → UnifiedStreamEvent mapping (V9 §6, corrected to real method names)
//!
//! | JSON-RPC method | UnifiedStreamEvent |
//! |---|---|
//! | `item/agentMessage/delta` (`params.delta`) | `TextDelta` |
//! | `item/reasoningMessage/delta` (`params.delta`) | `ThinkingDelta` |
//! | `item/started` (item.type=`commandExecution`) | `ToolCallStart` |
//! | `item/commandExecution/outputDelta` (`params.deltaBase64`, base64) | `ToolCallDelta` |
//! | `item/completed` (item.type=`commandExecution`) | `ToolCallEnd` |
//! | `turn/completed` (turn.status=`completed`) | `Usage + Done(EndTurn)` |
//! | `turn/completed` (turn.status=`failed`) | `Error + Done(Error)` |
//!
//! Unknown methods are logged at `warn!` per V9 §6.
//!
//! ## Reconnect on crash
//!
//! Stdin write failure (broken pipe) → `Error + Done(Error)` → `inner = None`.
//! Next `send_turn` spawns a fresh subprocess.
//!
//! ## `CodexThreadId` persistence
//!
//! `continuity_handle()` returns `Some(ContinuityHandle::CodexThreadId(id))`
//! once `thread/started` is received. M4's `StoredConversation::continuity_handle`
//! persists it across restarts without additional `session_state.rs` changes.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use base64::Engine as _;
use futures::Stream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::context_planner::{ContinuityHandle, ContinuityMode};
use crate::swarm::backend::{StopReason, TokenUsage, UnifiedStreamEvent};

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};

// ── JSON-RPC helpers ──────────────────────────────────────────────────────────

/// Global monotonic request ID counter (per process, not per session).
static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed)
}

/// Serialize a JSON-RPC 2.0 request (with `id`, expects a response).
fn rpc_request(method: &str, id: u64, params: serde_json::Value) -> String {
    format!(
        "{}\n",
        serde_json::json!({"method": method, "id": id, "params": params})
    )
}

/// Serialize a JSON-RPC 2.0 notification (no `id`, no response expected).
fn rpc_notification(method: &str, params: serde_json::Value) -> String {
    format!(
        "{}\n",
        serde_json::json!({"method": method, "params": params})
    )
}

// ── Internal subprocess state ─────────────────────────────────────────────────

struct AppServerInner {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    thread_id: String,
    /// Current per-turn event channel. Replaced each turn by `send_turn`.
    active_tx: Arc<Mutex<Option<mpsc::Sender<Result<UnifiedStreamEvent>>>>>,
}

// ── CodexAppServerSession ─────────────────────────────────────────────────────

/// M8 `AgentSession` for Codex `app-server` mode (`codex-app-server:` prefix).
pub struct CodexAppServerSession {
    model: String,
    workspace_root: PathBuf,
    inner: Option<AppServerInner>,
    handle: Option<ContinuityHandle>,
}

impl CodexAppServerSession {
    pub(super) fn new(args: SessionConstruction) -> Self {
        let model = args
            .model
            .strip_prefix("codex-app-server:")
            .unwrap_or(&args.model)
            .to_string();

        // Restore a previously persisted CodexThreadId so `ensure_running`
        // can send `thread/resume` instead of `thread/start`.
        #[allow(deprecated)]
        let handle = args
            .options
            .resume_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|id| ContinuityHandle::CodexThreadId(id.to_string()));

        Self {
            model,
            workspace_root: args.workspace_root,
            inner: None,
            handle,
        }
    }

    /// Spawn the subprocess and run the full startup handshake.
    ///
    /// On return, `self.inner` is `Some(_)` and `self.handle` holds the
    /// `CodexThreadId`.
    async fn ensure_running(&mut self) -> Result<()> {
        if self.inner.is_some() {
            return Ok(());
        }

        let mut cmd = Command::new("codex");
        cmd.arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&self.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "spawning codex app-server: {e}\n\
                 Ensure `codex` is on PATH and OPENAI_API_KEY is set.\n\
                 Use `codex:` / `codex-cli:` prefix to fall back to `codex exec`."
            )
        })?;

        let stdout = child
            .stdout
            .take()
            .context("codex app-server stdout unavailable")?;
        let stdin = child
            .stdin
            .take()
            .context("codex app-server stdin unavailable")?;
        let mut stdin = BufWriter::new(stdin);
        let mut lines = BufReader::new(stdout).lines();

        // Run the startup handshake synchronously before handing stdout to the
        // background reader — avoids races between init messages and turn events.
        let thread_id = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            handshake(
                &mut stdin,
                &mut lines,
                &self.model,
                &self.workspace_root,
                &self.handle,
            ),
        )
        .await
        .context("codex app-server: handshake timed out")??;

        tracing::debug!(thread_id, "codex app-server: ready");
        self.handle = Some(ContinuityHandle::CodexThreadId(thread_id.clone()));

        let active_tx: Arc<Mutex<Option<mpsc::Sender<Result<UnifiedStreamEvent>>>>> =
            Arc::new(Mutex::new(None));
        let active_tx_bg = active_tx.clone();

        // Background stdout reader: routes events to the per-turn channel.
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let (events, is_done) = parse_rpc_event(&line);
                {
                    let guard = active_tx_bg.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        for ev in events {
                            if tx.send(Ok(ev)).await.is_err() {
                                break;
                            }
                        }
                    } else if !events.is_empty() {
                        tracing::warn!("codex app-server: event outside active turn: {line}");
                    }
                }
                if is_done {
                    active_tx_bg.lock().await.take();
                }
            }
            tracing::debug!("codex app-server: stdout closed");
        });

        self.inner = Some(AppServerInner {
            child,
            stdin,
            thread_id,
            active_tx,
        });
        Ok(())
    }

    async fn tear_down(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            drop(inner.stdin);
            let _ = inner.child.wait().await;
            tracing::debug!("codex app-server: subprocess reaped");
        }
    }
}

#[async_trait::async_trait]
impl AgentSession for CodexAppServerSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        if let Err(e) = self.ensure_running().await {
            let msg = format!("codex app-server: spawn failed: {e:#}");
            tracing::warn!("{msg}");
            return Ok(error_then_done(msg));
        }

        let inner = self.inner.as_mut().unwrap();
        let thread_id = inner.thread_id.clone();

        // Install per-turn channel before writing stdin.
        let (tx, rx) = mpsc::channel::<Result<UnifiedStreamEvent>>(64);
        *inner.active_tx.lock().await = Some(tx);

        // Send turn/start request.
        let req = rpc_request(
            "turn/start",
            next_id(),
            serde_json::json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": turn.user_message }],
            }),
        );
        if let Err(e) = write_msg(&mut inner.stdin, &req).await {
            tracing::warn!("codex app-server: stdin write failed (crash?): {e}");
            inner.active_tx.lock().await.take();
            self.tear_down().await;
            return Ok(error_then_done(format!("codex app-server crashed: {e:#}")));
        }

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        ContinuityMode::ProcessBound
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        self.handle.as_ref()
    }

    async fn close(mut self: Box<Self>) {
        self.tear_down().await;
    }
}

// ── Startup handshake ─────────────────────────────────────────────────────────

/// Run the full JSON-RPC 2.0 startup sequence and return the thread ID.
async fn handshake(
    stdin: &mut BufWriter<ChildStdin>,
    lines: &mut Lines<BufReader<ChildStdout>>,
    model: &str,
    cwd: &PathBuf,
    existing_handle: &Option<ContinuityHandle>,
) -> Result<String> {
    // 1. initialize
    let init_id = next_id();
    write_msg(
        stdin,
        &rpc_request(
            "initialize",
            init_id,
            serde_json::json!({ "clientInfo": { "name": "gaviero" } }),
        ),
    )
    .await?;

    // Read until we see the response for init_id (has "id" == init_id).
    read_until_response(lines, init_id).await?;

    // 2. initialized notification (no response expected)
    write_msg(
        stdin,
        &rpc_notification("initialized", serde_json::json!({})),
    )
    .await?;

    // 3. thread/start or thread/resume
    let (method, params) = match existing_handle {
        Some(ContinuityHandle::CodexThreadId(id)) => {
            ("thread/resume", serde_json::json!({ "threadId": id }))
        }
        _ => (
            "thread/start",
            serde_json::json!({
                "model": model,
                "cwd": cwd.to_string_lossy(),
                "approvalPolicy": "never",
            }),
        ),
    };

    let thread_req_id = next_id();
    write_msg(stdin, &rpc_request(method, thread_req_id, params)).await?;

    // Read until thread/started notification, capture thread_id.
    read_thread_id(lines).await
}

/// Discard lines until a response for `expected_id` arrives (has `"id"` field).
async fn read_until_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
) -> Result<()> {
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if val.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
            if let Some(err) = val.get("error") {
                anyhow::bail!("codex app-server RPC error: {err}");
            }
            return Ok(());
        }
    }
    anyhow::bail!("codex app-server: stdout closed before receiving response id={expected_id}")
}

/// Read lines until `thread/started` notification, extract and return thread ID.
async fn read_thread_id(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<String> {
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if val.get("method").and_then(|v| v.as_str()) == Some("thread/started") {
            let id = val
                .pointer("/params/thread/id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            return Ok(id);
        }
        // Also accept a response that carries the thread id directly.
        if let Some(thread_id) = val.pointer("/result/thread/id").and_then(|v| v.as_str()) {
            return Ok(thread_id.to_string());
        }
    }
    anyhow::bail!("codex app-server: stdout closed before thread/started")
}

// ── Event parser ──────────────────────────────────────────────────────────────

/// Parse one NDJSON line into `(events, is_done)`.
///
/// Implements the corrected V9 §6 mapping using real JSON-RPC 2.0 method names.
/// Unknown methods are logged at `warn!` per V9 §6.
fn parse_rpc_event(line: &str) -> (Vec<UnifiedStreamEvent>, bool) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        tracing::warn!("codex app-server: malformed JSON: {line}");
        return (vec![], false);
    };

    // Skip responses (they have an "id" but no "method").
    let Some(method) = val.get("method").and_then(|v| v.as_str()) else {
        return (vec![], false);
    };

    let params = val.get("params").unwrap_or(&serde_json::Value::Null);

    match method {
        // V9 §6: item/agentMessage/delta → TextDelta(delta)
        "item/agentMessage/delta" => {
            let delta = params
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (vec![UnifiedStreamEvent::TextDelta(delta)], false)
        }

        // V9 §6: item/reasoningMessage/delta → ThinkingDelta(delta)
        "item/reasoningMessage/delta" | "item/reasoning/delta" => {
            let delta = params
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (vec![UnifiedStreamEvent::ThinkingDelta(delta)], false)
        }

        // V9 §6: item/commandExecution start → ToolCallStart { id, name:"Bash" }
        // Real method: item/started with item.type == "commandExecution"
        "item/started" => {
            let item = params.get("item").unwrap_or(&serde_json::Value::Null);
            if item.get("type").and_then(|v| v.as_str()) == Some("commandExecution") {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (
                    vec![UnifiedStreamEvent::ToolCallStart {
                        id,
                        name: "Bash".to_string(),
                    }],
                    false,
                )
            } else {
                (vec![], false)
            }
        }

        // V9 §6: item/commandExecution/outputDelta → ToolCallDelta
        // Real: params.deltaBase64 (base64-encoded stdout/stderr)
        "item/commandExecution/outputDelta" => {
            let id = params
                .get("itemId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let chunk = params
                .get("deltaBase64")
                .and_then(|v| v.as_str())
                .and_then(|b64| {
                    base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                })
                .unwrap_or_default();
            (
                vec![UnifiedStreamEvent::ToolCallDelta {
                    id,
                    args_chunk: chunk,
                }],
                false,
            )
        }

        // V9 §6: item/commandExecution final → ToolCallEnd { id }
        // Real method: item/completed with item.type == "commandExecution"
        "item/completed" => {
            let item = params.get("item").unwrap_or(&serde_json::Value::Null);
            if item.get("type").and_then(|v| v.as_str()) == Some("commandExecution") {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (vec![UnifiedStreamEvent::ToolCallEnd { id }], false)
            } else {
                (vec![], false)
            }
        }

        // V9 §6: turn/completed (status=completed) → Usage + Done(EndTurn)
        //        turn/completed (status=failed)    → Error + Done(Error)
        "turn/completed" => {
            let turn = params.get("turn").unwrap_or(&serde_json::Value::Null);
            let status = turn
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("completed");
            let events: Vec<UnifiedStreamEvent> = if status == "completed" {
                let usage = turn.get("tokenUsage");
                let input_tokens = usage
                    .and_then(|u| u.get("inputTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output_tokens = usage
                    .and_then(|u| u.get("outputTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                vec![
                    UnifiedStreamEvent::Usage(TokenUsage {
                        input_tokens,
                        output_tokens,
                        ..Default::default()
                    }),
                    UnifiedStreamEvent::Done(StopReason::EndTurn),
                ]
            } else {
                let msg = turn
                    .pointer("/error/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("turn failed")
                    .to_string();
                vec![
                    UnifiedStreamEvent::Error(msg),
                    UnifiedStreamEvent::Done(StopReason::Error),
                ]
            };
            (events, true)
        }

        // Informational lifecycle events — no UnifiedStreamEvent equivalent.
        "turn/started"
        | "thread/started"
        | "thread/status/changed"
        | "thread/closed"
        | "thread/archived" => (vec![], false),

        // V9 §6: "Unknown events: log at warn! Do not silently drop."
        other => {
            tracing::warn!("codex app-server: unknown event '{other}': {line}");
            (vec![], false)
        }
    }
}

// ── I/O helpers ───────────────────────────────────────────────────────────────

async fn write_msg(stdin: &mut BufWriter<ChildStdin>, msg: &str) -> std::io::Result<()> {
    stdin.write_all(msg.as_bytes()).await?;
    stdin.flush().await
}

fn error_then_done(msg: String) -> Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>> {
    Box::pin(futures::stream::iter(vec![
        Ok(UnifiedStreamEvent::Error(msg)),
        Ok(UnifiedStreamEvent::Done(StopReason::Error)),
    ]))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: parse and assert event list + done flag.
    fn parse(line: &str) -> (Vec<UnifiedStreamEvent>, bool) {
        parse_rpc_event(line)
    }

    #[test]
    fn parse_agent_message_delta() {
        let (events, done) = parse(
            r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"hello"}}"#,
        );
        assert_eq!(events, vec![UnifiedStreamEvent::TextDelta("hello".into())]);
        assert!(!done);
    }

    #[test]
    fn parse_reasoning_delta() {
        let (events, done) = parse(
            r#"{"method":"item/reasoningMessage/delta","params":{"itemId":"i2","delta":"thinking"}}"#,
        );
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ThinkingDelta("thinking".into())]
        );
        assert!(!done);
    }

    #[test]
    fn parse_item_started_command_execution() {
        let (events, done) = parse(
            r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"cmd1","command":"ls"}}}"#,
        );
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallStart {
                id: "cmd1".into(),
                name: "Bash".into()
            }]
        );
        assert!(!done);
    }

    #[test]
    fn parse_item_started_non_command_produces_no_event() {
        let (events, _) = parse(
            r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1","text":""}}}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn parse_command_output_delta_decodes_base64() {
        // "ls\n" in base64 is "bHMK"
        let b64 = base64::engine::general_purpose::STANDARD.encode("ls\n");
        let line = format!(
            r#"{{"method":"item/commandExecution/outputDelta","params":{{"itemId":"cmd1","stream":"stdout","deltaBase64":"{b64}"}}}}"#
        );
        let (events, done) = parse(&line);
        assert!(!done);
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallDelta {
                id: "cmd1".into(),
                args_chunk: "ls\n".into()
            }]
        );
    }

    #[test]
    fn parse_item_completed_command_execution() {
        let (events, done) = parse(
            r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"cmd1","status":"completed","exitCode":0}}}"#,
        );
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallEnd { id: "cmd1".into() }]
        );
        assert!(!done);
    }

    #[test]
    fn parse_turn_completed_success() {
        let (events, done) = parse(
            r#"{"method":"turn/completed","params":{"turn":{"id":"t1","status":"completed","tokenUsage":{"inputTokens":10,"outputTokens":5}}}}"#,
        );
        assert!(done);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            UnifiedStreamEvent::Usage(u) if u.input_tokens == 10 && u.output_tokens == 5
        ));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    #[test]
    fn parse_turn_completed_failed() {
        let (events, done) = parse(
            r#"{"method":"turn/completed","params":{"turn":{"status":"failed","error":{"message":"context exceeded"}}}}"#,
        );
        assert!(done);
        assert_eq!(
            events[0],
            UnifiedStreamEvent::Error("context exceeded".into())
        );
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::Error));
    }

    #[test]
    fn parse_informational_events_emit_nothing() {
        for line in &[
            r#"{"method":"turn/started","params":{"turn":{"id":"t1"}}}"#,
            r#"{"method":"thread/started","params":{"thread":{"id":"th1"}}}"#,
            r#"{"method":"thread/status/changed","params":{"threadId":"th1","status":{"type":"idle"}}}"#,
            r#"{"method":"thread/closed","params":{"threadId":"th1"}}"#,
        ] {
            let (events, done) = parse(line);
            assert!(events.is_empty(), "expected empty for {line}");
            assert!(!done);
        }
    }

    #[test]
    fn parse_rpc_response_emits_nothing() {
        // Responses have "id" but no "method" — should be silently skipped.
        let (events, done) = parse(r#"{"id":1,"result":{"userAgent":"codex/1.0"}}"#);
        assert!(events.is_empty());
        assert!(!done);
    }

    #[test]
    fn parse_unknown_method_emits_nothing_but_is_logged() {
        let (events, done) = parse(r#"{"method":"future/unknown","params":{}}"#);
        assert!(events.is_empty());
        assert!(!done);
    }

    #[test]
    fn parse_malformed_json_emits_nothing() {
        let (events, done) = parse("not json");
        assert!(events.is_empty());
        assert!(!done);
    }

    #[test]
    fn rpc_request_format() {
        let msg = rpc_request(
            "initialize",
            0,
            serde_json::json!({"clientInfo":{"name":"x"}}),
        );
        let val: serde_json::Value = serde_json::from_str(msg.trim()).unwrap();
        assert_eq!(val["method"], "initialize");
        assert_eq!(val["id"], 0);
        assert!(val.get("params").is_some());
    }

    #[test]
    fn rpc_notification_has_no_id() {
        let msg = rpc_notification("initialized", serde_json::json!({}));
        let val: serde_json::Value = serde_json::from_str(msg.trim()).unwrap();
        assert_eq!(val["method"], "initialized");
        assert!(val.get("id").is_none());
    }
}
