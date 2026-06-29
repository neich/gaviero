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
use crate::swarm::backend::shared::default_editor_system_prompt;
use crate::swarm::backend::{
    Capabilities, RetrievalToolset, StopReason, TokenUsage, UnifiedStreamEvent,
};

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};

// ── JSON-RPC helpers ──────────────────────────────────────────────────────────

/// Global monotonic request ID counter (per process, not per session).
static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed)
}

fn codex_file_block_capabilities() -> Capabilities {
    Capabilities {
        tool_use: true,
        streaming: true,
        vision: false,
        extended_thinking: false,
        max_context_tokens: 200_000,
        supports_system_prompt: true,
        supports_file_blocks: true,
        // PUSH→PULL Phase 1: the gaviero MCP server is wired for Codex, so the
        // always-on retrieval tools are live.
        retrieval: RetrievalToolset {
            graph_and_memory: true,
            symbols: false,
        },
    }
}

fn codex_file_block_developer_instructions(
    cwd: &PathBuf,
    additional_roots: &[PathBuf],
) -> String {
    let base = default_editor_system_prompt(&codex_file_block_capabilities());
    if additional_roots.is_empty() {
        return base;
    }
    // Workspace-mode multi-folder: the codex app-server protocol takes a
    // single `cwd` field, and the only protocol-level "additional writable
    // roots" is `WorkspaceWriteSandboxPolicy.writableRoots` — which would
    // require switching the sandbox to write-mode and bypass the Write
    // Gate. Instead, surface the sibling folders as a workspace hint in
    // the developer instructions so the model knows it can read/edit
    // across them. File edits still flow through `<file>` proposals.
    let mut hint = String::from("\n\nWorkspace folders (workspace-mode):\n");
    hint.push_str(&format!("  primary: {}\n", cwd.to_string_lossy()));
    for r in additional_roots {
        if r.as_os_str().is_empty() || r == cwd {
            continue;
        }
        hint.push_str(&format!("  sibling: {}\n", r.to_string_lossy()));
    }
    hint.push_str(
        "Read freely from any folder above. File edits across any folder are emitted as <file path=\"...\"> ... </file> proposals — gaviero routes them through its review queue.\n",
    );
    format!("{base}{hint}")
}

fn thread_start_params(
    model: &str,
    cwd: &PathBuf,
    additional_roots: &[PathBuf],
    allow_network: bool,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
        "developerInstructions": codex_file_block_developer_instructions(cwd, additional_roots),
    })
}

fn thread_resume_params(
    thread_id: &str,
    cwd: &PathBuf,
    additional_roots: &[PathBuf],
    allow_network: bool,
) -> serde_json::Value {
    serde_json::json!({
        "threadId": thread_id,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
        "developerInstructions": codex_file_block_developer_instructions(cwd, additional_roots),
    })
}

fn turn_start_params(thread_id: &str, user_message: &str, allow_network: bool) -> serde_json::Value {
    serde_json::json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": user_message }],
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
    })
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
    /// Sibling workspace folders (workspace-mode multi-folder). The codex
    /// app-server RPC has no `--add-dir` analog while in `read-only`
    /// sandbox, so these are passed through `developerInstructions` as a
    /// workspace hint. File edits still flow through the in-band `<file>`
    /// proposal channel and gaviero's Write Gate.
    additional_roots: Vec<PathBuf>,
    inner: Option<AppServerInner>,
    handle: Option<ContinuityHandle>,
}

/// Build the argv (after the `codex` binary) for the `app-server` invocation.
///
/// Top-level `-c mcp_servers.X.Y=Z` overrides go **before** the `app-server`
/// subcommand because codex CLI applies `--config` on the top-level command
/// and dispatches to subcommands afterwards. The MCP entries themselves come
/// from the synthesized `<workspace_root>/.codex/config.toml`, since codex's
/// CLI only auto-loads `$CODEX_HOME/config.toml` — without these overrides
/// the per-worktree MCP servers stay invisible to the chat session.
fn codex_app_server_args(workspace_root: &std::path::Path) -> Vec<String> {
    let mut args = Vec::new();
    let codex_config = workspace_root.join(".codex/config.toml");
    for pair in crate::mcp::codex_mcp_overrides_from_config_file(&codex_config) {
        args.push("--config".to_string());
        args.push(pair);
    }
    args.push("app-server".to_string());
    args.push("--listen".to_string());
    args.push("stdio://".to_string());
    args
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
            additional_roots: args.additional_roots,
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
        for arg in codex_app_server_args(&self.workspace_root) {
            cmd.arg(arg);
        }
        cmd.current_dir(&self.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "spawning codex app-server: {e}\n\
                 Ensure `codex` is on PATH and OPENAI_API_KEY is set.\n\
                 Use `codex:` prefix to fall back to `codex exec`."
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

        // Grant network access at thread start when the synthesized MCP
        // config declares an HTTP server. Same logic as `send_turn` so
        // the policy stays consistent between the initial handshake and
        // every subsequent turn.
        let allow_network = crate::mcp::codex_synth_has_remote_mcp(&self.workspace_root);

        // Run the startup handshake synchronously before handing stdout to the
        // background reader — avoids races between init messages and turn events.
        let thread_id = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            handshake(
                &mut stdin,
                &mut lines,
                &self.model,
                &self.workspace_root,
                &self.additional_roots,
                &self.handle,
                allow_network,
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

        // Grant network access to read-only sandbox when the synthesized
        // MCP config declares an HTTP server — otherwise Codex's network
        // sandbox blocks MCP tool calls before they leave the process.
        let allow_network = crate::mcp::codex_synth_has_remote_mcp(&self.workspace_root);

        // Send turn/start request.
        let req = rpc_request(
            "turn/start",
            next_id(),
            turn_start_params(&thread_id, &turn.user_message, allow_network),
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
    additional_roots: &[PathBuf],
    existing_handle: &Option<ContinuityHandle>,
    allow_network: bool,
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
        Some(ContinuityHandle::CodexThreadId(id)) => (
            "thread/resume",
            thread_resume_params(id, cwd, additional_roots, allow_network),
        ),
        _ => (
            "thread/start",
            thread_start_params(model, cwd, additional_roots, allow_network),
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
                let args = match item.get("command") {
                    Some(cmd) => serde_json::json!({ "command": cmd }),
                    None => serde_json::Value::Null,
                };
                (
                    vec![UnifiedStreamEvent::ToolCallStart {
                        id,
                        name: "Bash".to_string(),
                        args,
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
    fn codex_app_server_args_place_config_overrides_before_subcommand() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            r#"
[mcp_servers.gaviero]
command = "gaviero-mcp-shim"
args = ["--socket", "/tmp/mcp.sock"]

[mcp_servers.semantic-scholar]
url = "https://example/mcp/"
"#,
        )
        .unwrap();
        let args = codex_app_server_args(dir.path());
        // The `--config` pairs must come before `app-server` so codex parses
        // them at the top-level command rather than passing them to the
        // subcommand (where they would be rejected as unknown flags).
        let app_server_idx = args.iter().position(|a| a == "app-server").expect("app-server arg");
        let last_config_idx = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "--config")
            .map(|(i, _)| i)
            .last()
            .expect("at least one --config pair");
        assert!(
            last_config_idx < app_server_idx,
            "--config must precede app-server in {args:?}",
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--config" && w[1] == r#"mcp_servers.gaviero.command="gaviero-mcp-shim""#),
            "missing gaviero.command override in {args:?}",
        );
        assert!(
            args.windows(2).any(|w| w[0] == "--config"
                && w[1] == r#"mcp_servers.semantic-scholar.url="https://example/mcp/""#),
            "missing semantic-scholar.url override in {args:?}",
        );
        // The literal subcommand wiring stays intact.
        assert_eq!(
            args.iter().rev().take(3).cloned().collect::<Vec<_>>(),
            vec!["stdio://".to_string(), "--listen".to_string(), "app-server".to_string()],
        );
    }

    #[test]
    fn codex_app_server_args_emit_no_config_when_synth_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let args = codex_app_server_args(dir.path());
        assert!(
            !args.iter().any(|a| a == "--config"),
            "expected no --config overrides, got {args:?}",
        );
        assert_eq!(
            args,
            vec!["app-server".to_string(), "--listen".to_string(), "stdio://".to_string()],
        );
    }

    #[test]
    fn thread_start_policy_forces_file_blocks_and_read_only() {
        let params = thread_start_params("gpt-5.5", &PathBuf::from("/tmp/work"), &[], false);
        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], false);
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("All code edits must be proposed"));
        assert!(instructions.contains("<file path=\"relative/path\">...</file>"));
        // Single-folder mode: no workspace-folders hint appended.
        assert!(!instructions.contains("Workspace folders"));
    }

    #[test]
    fn thread_start_grants_network_when_remote_mcp_present() {
        let params = thread_start_params("gpt-5.5", &PathBuf::from("/tmp/work"), &[], true);
        // Filesystem stays read-only — only the network layer opens up.
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], true);
    }

    #[test]
    fn thread_resume_policy_reasserts_file_blocks_and_read_only() {
        let params = thread_resume_params("thread-1", &PathBuf::from("/tmp/work"), &[], false);
        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], false);
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("All code edits must be proposed"));
    }

    #[test]
    fn thread_resume_grants_network_when_remote_mcp_present() {
        let params = thread_resume_params("thread-1", &PathBuf::from("/tmp/work"), &[], true);
        assert_eq!(params["sandboxPolicy"]["networkAccess"], true);
    }

    #[test]
    fn thread_start_appends_sibling_folders_to_developer_instructions() {
        let extras = vec![
            PathBuf::from("/tmp/sibling-a"),
            PathBuf::from("/tmp/sibling-b"),
        ];
        let params = thread_start_params("gpt-5.5", &PathBuf::from("/tmp/work"), &extras, false);
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("Workspace folders"));
        assert!(instructions.contains("primary: /tmp/work"));
        assert!(instructions.contains("sibling: /tmp/sibling-a"));
        assert!(instructions.contains("sibling: /tmp/sibling-b"));
    }

    #[test]
    fn thread_start_skips_empty_or_duplicate_sibling_folders() {
        let extras = vec![
            PathBuf::new(),
            PathBuf::from("/tmp/work"), // same as cwd; should be skipped
            PathBuf::from("/tmp/sibling"),
        ];
        let params = thread_start_params("gpt-5.5", &PathBuf::from("/tmp/work"), &extras, false);
        let instructions = params["developerInstructions"].as_str().unwrap();
        // The cwd appears once as "primary"; not as a sibling line.
        let sibling_lines = instructions.matches("sibling:").count();
        assert_eq!(sibling_lines, 1, "only /tmp/sibling is a real sibling");
        assert!(instructions.contains("sibling: /tmp/sibling"));
    }

    #[test]
    fn turn_start_policy_reasserts_read_only_sandbox() {
        let params = turn_start_params("thread-1", "hello", false);
        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], false);
    }

    #[test]
    fn turn_start_grants_network_when_remote_mcp_present() {
        let params = turn_start_params("thread-1", "hello", true);
        // Filesystem stays read-only — only the network bit flips.
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], true);
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
                name: "Bash".into(),
                args: serde_json::json!({ "command": "ls" }),
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
