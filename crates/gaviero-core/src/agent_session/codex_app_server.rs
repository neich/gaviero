//! Codex `app-server` session.
//!
//! [`CodexAppServerSession`] keeps a `codex app-server --listen stdio://`
//! subprocess alive for the lifetime of the session and translates its JSON-RPC
//! event stream into [`UnifiedStreamEvent`] values.
//!
//! File edits use Codex's native `fileChange` protocol. Gaviero runs the thread
//! in a read-only sandbox with `approvalPolicy=on-request`, snapshots every
//! proposed path when `item/started` arrives, then accepts the corresponding
//! `item/fileChange/requestApproval`. At turn completion it captures Codex's
//! final contents, restores the pre-turn state, and inserts ordinary Write Gate
//! proposals. This gives Codex the same review semantics as Claude without
//! forcing whole files through assistant text.
//!
//! Direct, workspace-scoped `cargo fmt` and `cargo test` commands are approved
//! through Codex's command-execution approval protocol. Before approving
//! `cargo fmt`, Gaviero snapshots every Rust source under the configured roots,
//! so formatter changes are included in the same transactional review.
//!
//! Standard `codex:` chat sessions remain `StatelessReplay`: the TUI creates a
//! fresh session per turn and the planner supplies replay history. The explicit
//! `codex-app-server:` profile remains `ProcessBound`.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use anyhow::{Context, Result};
use base64::Engine as _;
use futures::Stream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::client::{propose_delete, propose_write};
use crate::context_planner::{ContinuityHandle, ContinuityMode};
use crate::observer::AcpObserver;
use crate::swarm::backend::shared::{
    build_enriched_prompt, default_editor_system_prompt, render_graph_block, render_memory_block,
    render_skill_block,
};
use crate::swarm::backend::{
    Capabilities, RetrievalToolset, StopReason, TokenUsage, UnifiedStreamEvent,
};
use crate::write_gate::WriteGatePipeline;

use super::background::{
    PendingBg, finish_all_pending_killed, finish_pending_bg, register_pending_bg,
};
use super::registry::SessionConstruction;
use super::tool_surface::{AgentToolSurface, CommandDecision};
use super::{AgentSession, Turn};

static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed)
}

fn codex_native_edit_capabilities() -> Capabilities {
    Capabilities {
        tool_use: true,
        streaming: true,
        vision: false,
        extended_thinking: false,
        max_context_tokens: 200_000,
        supports_system_prompt: true,
        supports_file_blocks: false,
        retrieval: RetrievalToolset {
            graph_and_memory: true,
            symbols: false,
        },
    }
}

fn codex_native_edit_developer_instructions(cwd: &Path, additional_roots: &[PathBuf]) -> String {
    let mut instructions = default_editor_system_prompt(&codex_native_edit_capabilities());
    instructions.push_str(
        "\n\nFor Codex, `apply_patch` is the native file-edit tool. Use it for all \
         source-file additions, updates, moves, and deletions. Do not write files \
         through shell redirection or helper scripts. Gaviero snapshots each native \
         file change, restores the original after the turn, and sends the intended \
         result to its review queue. Do not print complete files in assistant text.\n\n\
         You may run direct, workspace-scoped `cargo fmt ...` and `cargo test ...` \
         verification commands against the temporary edited workspace. Invoke Cargo \
         directly, without shell chaining, pipes, redirection, command wrappers, \
         network escalation, or manifests outside the configured workspace roots. \
         Other write-capable shell commands are declined. Prefer the narrowest \
         package and test scope that validates the change.\n",
    );

    if additional_roots.is_empty() {
        return instructions;
    }

    instructions.push_str("\nWorkspace folders (workspace-mode):\n");
    instructions.push_str(&format!("  primary: {}\n", cwd.to_string_lossy()));
    for root in additional_roots {
        if root.as_os_str().is_empty() || root == cwd {
            continue;
        }
        instructions.push_str(&format!("  sibling: {}\n", root.to_string_lossy()));
    }
    instructions.push_str(
        "Read from any folder above. Native edits inside those folders are captured \
         transactionally and routed through the same review queue.\n",
    );
    instructions
}

fn thread_start_params(
    model: &str,
    cwd: &Path,
    additional_roots: &[PathBuf],
    allow_network: bool,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "on-request",
        "sandbox": "read-only",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
        "developerInstructions": codex_native_edit_developer_instructions(cwd, additional_roots),
    })
}

fn thread_resume_params(
    thread_id: &str,
    cwd: &Path,
    additional_roots: &[PathBuf],
    allow_network: bool,
) -> serde_json::Value {
    serde_json::json!({
        "threadId": thread_id,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "on-request",
        "sandbox": "read-only",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
        "developerInstructions": codex_native_edit_developer_instructions(cwd, additional_roots),
    })
}

fn turn_start_params(
    thread_id: &str,
    user_message: &str,
    allow_network: bool,
) -> serde_json::Value {
    serde_json::json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": user_message }],
        "approvalPolicy": "on-request",
        "sandboxPolicy": { "type": "readOnly", "networkAccess": allow_network },
    })
}

/// `initialize` params. Codex's `ClientInfo` requires both `name` and
/// `version`; omitting `version` fails the request with
/// `-32600 Invalid request: missing field \`version\``.
fn client_info_params() -> serde_json::Value {
    serde_json::json!({
        "clientInfo": {
            "name": "gaviero",
            "title": "Gaviero",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "experimentalApi": true,
        },
    })
}

fn rpc_request(method: &str, id: u64, params: serde_json::Value) -> String {
    format!(
        "{}\n",
        serde_json::json!({"method": method, "id": id, "params": params})
    )
}

fn rpc_notification(method: &str, params: serde_json::Value) -> String {
    format!(
        "{}\n",
        serde_json::json!({"method": method, "params": params})
    )
}

fn rpc_response(id: &serde_json::Value, result: serde_json::Value) -> String {
    format!("{}\n", serde_json::json!({"id": id, "result": result}))
}

type EventSender = mpsc::Sender<Result<UnifiedStreamEvent>>;
type SharedStdin = Arc<Mutex<BufWriter<ChildStdin>>>;
type WeakStdin = Weak<Mutex<BufWriter<ChildStdin>>>;
type SharedActiveTurn = Arc<Mutex<Option<ActiveTurn>>>;

#[derive(Default)]
struct TurnSnapshot {
    originals: HashMap<PathBuf, Option<String>>,
}

impl TurnSnapshot {
    async fn capture_before_write(&mut self, path: &Path) -> Result<()> {
        if self.originals.contains_key(path) {
            return Ok(());
        }
        let content = read_optional_text(path)
            .await
            .with_context(|| format!("snapshot read of {}", path.display()))?;
        self.originals.insert(path.to_path_buf(), content);
        Ok(())
    }

    fn original(&self, path: &Path) -> Option<&Option<String>> {
        self.originals.get(path)
    }

    fn edits(&self) -> Vec<(PathBuf, Option<String>)> {
        self.originals
            .iter()
            .map(|(path, content)| (path.clone(), content.clone()))
            .collect()
    }

    async fn revert_path(&self, path: &Path) -> Result<()> {
        let Some(original) = self.originals.get(path) else {
            return Ok(());
        };
        match original {
            Some(content) => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                tokio::fs::write(path, content)
                    .await
                    .with_context(|| format!("restoring {}", path.display()))?;
            }
            None => match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e).with_context(|| format!("removing {}", path.display()));
                }
            },
        }
        Ok(())
    }
}

struct ActiveTurn {
    tx: EventSender,
    snapshot: TurnSnapshot,
    seen_file_items: HashSet<String>,
    declined_file_items: HashSet<String>,
    item_paths: HashMap<String, Vec<PathBuf>>,
    pending_bg: Vec<PendingBg>,
}

impl ActiveTurn {
    fn new(tx: EventSender) -> Self {
        Self {
            tx,
            snapshot: TurnSnapshot::default(),
            seen_file_items: HashSet::new(),
            declined_file_items: HashSet::new(),
            item_paths: HashMap::new(),
            pending_bg: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct ReviewContext {
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Arc<dyn AcpObserver>,
    primary_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    agent_id: String,
    conv_id: Option<String>,
    tool_surface: AgentToolSurface,
}

impl ReviewContext {
    fn root_for_path(&self, path: &Path) -> Option<&PathBuf> {
        self.allowed_roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoVerificationKind {
    Build,
    Check,
    Format,
    Test,
}

#[derive(Debug, Eq, PartialEq)]
struct CargoVerificationRequest {
    kind: CargoVerificationKind,
    cwd: PathBuf,
}

struct AppServerInner {
    child: Child,
    stdin: SharedStdin,
    thread_id: String,
    active_turn: SharedActiveTurn,
    reader_task: tokio::task::JoinHandle<()>,
}

pub struct CodexAppServerSession {
    model: String,
    workspace_root: PathBuf,
    additional_roots: Vec<PathBuf>,
    continuity_mode: ContinuityMode,
    inner: Option<AppServerInner>,
    handle: Option<ContinuityHandle>,
    review: ReviewContext,
}

fn codex_app_server_args(workspace_root: &Path) -> Vec<String> {
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
    pub(super) fn new(args: SessionConstruction, observer: Arc<dyn AcpObserver>) -> Self {
        let model = args
            .model
            .strip_prefix("codex-app-server:")
            .or_else(|| args.model.strip_prefix("codex:"))
            .unwrap_or(&args.model)
            .to_string();
        let continuity_mode = args.profile.continuity_mode;

        #[allow(deprecated)]
        let handle = if continuity_mode == ContinuityMode::ProcessBound {
            args.options
                .resume_session_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(|id| ContinuityHandle::CodexThreadId(id.to_string()))
        } else {
            None
        };

        let primary_root = normalize_lexically(&args.workspace_root)
            .unwrap_or_else(|| args.workspace_root.clone());
        let mut allowed_roots = vec![primary_root.clone()];
        for root in &args.additional_roots {
            let Some(root) = normalize_lexically(root) else {
                continue;
            };
            if !allowed_roots.contains(&root) {
                allowed_roots.push(root);
            }
        }

        let tool_surface =
            AgentToolSurface::from_agent_options(&args.options, &args.workspace_root);

        let review = ReviewContext {
            write_gate: args.write_gate,
            observer,
            primary_root,
            allowed_roots,
            agent_id: args.agent_id,
            conv_id: args.conv_id,
            tool_surface,
        };

        Self {
            model,
            workspace_root: args.workspace_root,
            additional_roots: args.additional_roots,
            continuity_mode,
            inner: None,
            handle,
            review,
        }
    }

    async fn ensure_running(&mut self) -> Result<()> {
        if self.inner.is_some() {
            return Ok(());
        }

        let mut cmd = crate::util::spawn::agent_command("codex");
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
                 Ensure a current `codex` CLI is installed and authenticated."
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

        let allow_network = crate::mcp::codex_synth_has_remote_mcp(&self.workspace_root);
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
        if self.continuity_mode == ContinuityMode::ProcessBound {
            self.handle = Some(ContinuityHandle::CodexThreadId(thread_id.clone()));
        }

        let stdin = Arc::new(Mutex::new(stdin));
        let stdin_bg = Arc::downgrade(&stdin);
        let active_turn: SharedActiveTurn = Arc::new(Mutex::new(None));
        let active_turn_bg = active_turn.clone();
        let review = self.review.clone();

        let reader_task = tokio::spawn(async move {
            let mut terminal_error: Option<String> = None;
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if let Err(e) =
                    route_app_server_line(&line, &stdin_bg, &active_turn_bg, &review).await
                {
                    terminal_error = Some(format!("{e:#}"));
                    break;
                }
            }

            if let Some(active) = active_turn_bg.lock().await.take() {
                let mut message = terminal_error
                    .unwrap_or_else(|| "codex app-server stdout closed unexpectedly".to_string());
                if let Err(e) = finalize_native_edits(&review, active.snapshot).await {
                    message.push_str(&format!("\ncleanup failed: {e:#}"));
                }
                let _ = active.tx.send(Ok(UnifiedStreamEvent::Error(message))).await;
                let _ = active
                    .tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                    .await;
            }

            tracing::debug!("codex app-server: stdout closed");
        });

        self.inner = Some(AppServerInner {
            child,
            stdin,
            thread_id,
            active_turn,
            reader_task,
        });
        Ok(())
    }

    async fn tear_down(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            drop(inner.stdin);
            let _ = inner.child.wait().await;
            let _ = inner.reader_task.await;
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

        let rendered_message = render_turn_prompt(turn);
        let inner = self.inner.as_mut().expect("ensure_running set inner");
        let thread_id = inner.thread_id.clone();

        let (tx, rx) = mpsc::channel::<Result<UnifiedStreamEvent>>(64);
        {
            let mut active = inner.active_turn.lock().await;
            if active.is_some() {
                return Ok(error_then_done(
                    "codex app-server already has an active turn".to_string(),
                ));
            }
            *active = Some(ActiveTurn::new(tx));
        }

        let allow_network = crate::mcp::codex_synth_has_remote_mcp(&self.workspace_root);
        let request = rpc_request(
            "turn/start",
            next_id(),
            turn_start_params(&thread_id, &rendered_message, allow_network),
        );

        let write_result = {
            let mut stdin = inner.stdin.lock().await;
            write_msg(&mut stdin, &request).await
        };
        if let Err(e) = write_result {
            tracing::warn!("codex app-server: stdin write failed: {e}");
            inner.active_turn.lock().await.take();
            self.tear_down().await;
            return Ok(error_then_done(format!("codex app-server crashed: {e:#}")));
        }

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.continuity_mode
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        if self.continuity_mode == ContinuityMode::ProcessBound {
            self.handle.as_ref()
        } else {
            None
        }
    }

    async fn close(mut self: Box<Self>) {
        self.tear_down().await;
    }
}

fn render_turn_prompt(turn: Turn) -> String {
    let Turn {
        user_message,
        memory_selections,
        graph_selections,
        file_refs,
        skill_selections,
        replay_history,
        ..
    } = turn;

    let mut prompt_parts = vec![user_message];
    if let Some(block) = render_graph_block(&graph_selections) {
        prompt_parts.push(block);
    }
    if let Some(block) = render_memory_block(&memory_selections) {
        prompt_parts.push(block);
    }
    if let Some(block) = render_skill_block(&skill_selections) {
        prompt_parts.push(block);
    }

    let history = replay_history
        .map(|payload| {
            payload
                .entries
                .into_iter()
                .map(|(role, content)| {
                    let role = match role {
                        crate::context_planner::ledger::Role::User => "user",
                        crate::context_planner::ledger::Role::Assistant => "assistant",
                        crate::context_planner::ledger::Role::System => "system",
                    };
                    (role.to_string(), content)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let text_refs = file_refs
        .into_iter()
        .filter_map(|attachment| {
            attachment
                .content
                .map(|content| (attachment.path.to_string_lossy().into_owned(), content))
        })
        .collect::<Vec<_>>();

    build_enriched_prompt(&prompt_parts.join("\n\n"), &history, &text_refs)
}

async fn handshake(
    stdin: &mut BufWriter<ChildStdin>,
    lines: &mut Lines<BufReader<ChildStdout>>,
    model: &str,
    cwd: &Path,
    additional_roots: &[PathBuf],
    existing_handle: &Option<ContinuityHandle>,
    allow_network: bool,
) -> Result<String> {
    let init_id = next_id();
    write_msg(
        stdin,
        &rpc_request("initialize", init_id, client_info_params()),
    )
    .await?;
    read_until_response(lines, init_id).await?;

    write_msg(
        stdin,
        &rpc_notification("initialized", serde_json::json!({})),
    )
    .await?;

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

    let request_id = next_id();
    write_msg(stdin, &rpc_request(method, request_id, params)).await?;
    read_thread_id(lines).await
}

async fn read_until_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
) -> Result<()> {
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(|id| id.as_u64()) == Some(expected_id) {
            if let Some(error) = value.get("error") {
                anyhow::bail!("codex app-server RPC error: {error}");
            }
            return Ok(());
        }
    }
    anyhow::bail!("codex app-server: stdout closed before receiving response id={expected_id}")
}

async fn read_thread_id(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<String> {
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("method").and_then(|method| method.as_str()) == Some("thread/started") {
            return Ok(value
                .pointer("/params/thread/id")
                .and_then(|id| id.as_str())
                .unwrap_or("unknown")
                .to_string());
        }
        if let Some(thread_id) = value
            .pointer("/result/thread/id")
            .and_then(|id| id.as_str())
        {
            return Ok(thread_id.to_string());
        }
    }
    anyhow::bail!("codex app-server: stdout closed before thread/started")
}

async fn route_app_server_line(
    line: &str,
    stdin: &WeakStdin,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) -> Result<()> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        let (events, _) = parse_rpc_event(line);
        send_to_active(active_turn, events).await;
        return Ok(());
    };

    let method = value
        .get("method")
        .and_then(|method| method.as_str())
        .unwrap_or_default();

    match method {
        "item/started"
            if value
                .pointer("/params/item/type")
                .and_then(|kind| kind.as_str())
                == Some("fileChange") =>
        {
            capture_file_change_start(&value, active_turn, review).await;
            let (events, _) = parse_rpc_event(line);
            send_to_active(active_turn, events).await;
        }
        "item/started"
            if value
                .pointer("/params/item/type")
                .and_then(|kind| kind.as_str())
                .is_some_and(is_codex_subagent_item) =>
        {
            track_codex_subagent_start(&value, active_turn, review).await;
            let (events, _) = parse_rpc_event(line);
            send_to_active(active_turn, events).await;
        }
        "item/completed" => {
            track_codex_subagent_finish(&value, active_turn, review).await;
            let (events, _) = parse_rpc_event(line);
            send_to_active(active_turn, events).await;
        }
        "item/fileChange/requestApproval" => {
            let id = value
                .get("id")
                .context("file-change approval request missing id")?;
            let item_id = value
                .pointer("/params/itemId")
                .and_then(|item| item.as_str())
                .unwrap_or_default();
            let accept = file_change_is_safe_to_approve(item_id, active_turn, review).await;
            let decision = if accept { "accept" } else { "decline" };
            write_shared(
                stdin,
                &rpc_response(id, serde_json::json!({ "decision": decision })),
            )
            .await?;
        }
        "item/commandExecution/requestApproval" => {
            let id = value
                .get("id")
                .context("command approval request missing id")?;
            let accept = command_execution_is_safe_to_approve(&value, active_turn, review).await;
            let decision = if accept { "accept" } else { "decline" };
            write_shared(
                stdin,
                &rpc_response(id, serde_json::json!({ "decision": decision })),
            )
            .await?;
        }
        "turn/completed" => {
            let active = { active_turn.lock().await.take() };
            let (events, _) = parse_rpc_event(line);
            if let Some(mut active) = active {
                let _ = finish_all_pending_killed(&mut active.pending_bg, review.observer.as_ref());
                if let Err(e) = finalize_native_edits(review, active.snapshot).await {
                    let _ = active
                        .tx
                        .send(Ok(UnifiedStreamEvent::Error(format!("{e:#}"))))
                        .await;
                }
                send_events(&active.tx, events).await;
            } else if !events.is_empty() {
                tracing::warn!("codex app-server: turn completed without an active receiver");
            }
        }
        _ => {
            let (events, _) = parse_rpc_event(line);
            send_to_active(active_turn, events).await;
        }
    }

    Ok(())
}

/// Codex app-server item types that represent a spawned collaborator /
/// subagent rather than a host tool. Names collected from the protocol's
/// `item.type` field; unknown variants are ignored.
fn is_codex_subagent_item(kind: &str) -> bool {
    matches!(
        kind,
        "collab" | "agent" | "subAgent" | "subagent" | "task" | "spawnedAgent" | "agentTurn"
    )
}

async fn track_codex_subagent_start(
    value: &serde_json::Value,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) {
    let item = value
        .pointer("/params/item")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let id = item
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return;
    }
    let desc = crate::acp::protocol::subagent_description(&item);
    let mut active = active_turn.lock().await;
    let Some(active) = active.as_mut() else {
        return;
    };
    register_pending_bg(
        &mut active.pending_bg,
        &id,
        &id,
        &desc,
        review.observer.as_ref(),
    );
}

async fn track_codex_subagent_finish(
    value: &serde_json::Value,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) {
    let item = value
        .pointer("/params/item")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let kind = item.get("type").and_then(|k| k.as_str()).unwrap_or("");
    if !is_codex_subagent_item(kind) {
        return;
    }
    let id = item.get("id").and_then(|id| id.as_str()).unwrap_or("");
    let mut active = active_turn.lock().await;
    let Some(active) = active.as_mut() else {
        return;
    };
    finish_pending_bg(
        &mut active.pending_bg,
        id,
        id,
        "completed",
        "",
        review.observer.as_ref(),
    );
}

fn approval_command_line(value: &serde_json::Value) -> Option<String> {
    let command = value.pointer("/params/command")?;
    match command {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(arr) => Some(
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

async fn command_execution_is_safe_to_approve(
    value: &serde_json::Value,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) -> bool {
    let Some(command) = approval_command_line(value) else {
        tracing::debug!("declining Codex command: missing command payload");
        return false;
    };
    match review.tool_surface.decide_command(&command) {
        CommandDecision::Deny => {
            tracing::debug!(command, "declining Codex command: tool surface or denylist");
            false
        }
        CommandDecision::Allow => {
            if parse_cargo_verification_request(value, review).is_ok() {
                return cargo_verification_is_safe_to_approve(value, active_turn, review).await;
            }
            if let Err(e) = validate_additional_permissions(value, review) {
                tracing::debug!(error = %e, command, "declining Codex command: extra permissions");
                return false;
            }
            if active_turn.lock().await.is_none() {
                return false;
            }
            true
        }
        CommandDecision::UnattendedFallback => {
            cargo_verification_is_safe_to_approve(value, active_turn, review).await
        }
    }
}

async fn cargo_verification_is_safe_to_approve(
    value: &serde_json::Value,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) -> bool {
    let request = match parse_cargo_verification_request(value, review) {
        Ok(request) => request,
        Err(e) => {
            tracing::debug!(
                error = %e,
                params = %value.pointer("/params").unwrap_or(&serde_json::Value::Null),
                "declining Codex command outside the Cargo verification policy"
            );
            return false;
        }
    };

    if active_turn.lock().await.is_none() {
        tracing::warn!("declining Cargo verification outside an active Codex turn");
        return false;
    }

    if request.kind == CargoVerificationKind::Format
        && let Err(e) = capture_rust_sources_before_format(active_turn, review).await
    {
        tracing::warn!(
            error = %e,
            "declining cargo fmt because the source snapshot failed"
        );
        review.observer.on_message_complete(
            "system",
            &format!(
                "Declined `cargo fmt` because Gaviero could not snapshot all Rust sources: {e:#}"
            ),
        );
        return false;
    }

    tracing::debug!(
        kind = ?request.kind,
        cwd = %request.cwd.display(),
        "approving workspace-scoped Cargo verification"
    );
    true
}

fn parse_cargo_verification_request(
    value: &serde_json::Value,
    review: &ReviewContext,
) -> Result<CargoVerificationRequest> {
    if value
        .pointer("/params/networkApprovalContext")
        .is_some_and(|context| !context.is_null())
    {
        anyhow::bail!("network approval requests are not Cargo verification commands");
    }
    validate_additional_permissions(value, review)?;

    let command = value
        .pointer("/params/command")
        .context("command approval request missing command")?;
    let tokens = cargo_command_tokens(command)?;
    let tokens = unwrap_windows_shell_cargo_command(tokens)?;

    if tokens.is_empty() {
        anyhow::bail!("empty command");
    }
    if tokens.iter().any(|token| contains_shell_control(token)) {
        anyhow::bail!("shell composition is not allowed");
    }

    let executable = trim_matching_quotes(&tokens[0]);
    if executable != "cargo" && !executable.eq_ignore_ascii_case("cargo.exe") {
        anyhow::bail!("only direct Cargo commands are auto-approved");
    }

    let mut subcommand_index = 1;
    if tokens
        .get(subcommand_index)
        .map(|token| trim_matching_quotes(token))
        .is_some_and(|token| token.starts_with('+') && token.len() > 1)
    {
        subcommand_index += 1;
    }

    let subcommand = tokens
        .get(subcommand_index)
        .map(|token| trim_matching_quotes(token))
        .context("Cargo command missing subcommand")?;
    let kind = match subcommand {
        "build" => CargoVerificationKind::Build,
        "check" => CargoVerificationKind::Check,
        "fmt" => CargoVerificationKind::Format,
        "test" => CargoVerificationKind::Test,
        _ => anyhow::bail!("only cargo build, check, fmt, and test are auto-approved"),
    };

    let cwd = resolve_command_cwd(value, review)?;
    validate_cargo_scope_arguments(&tokens[subcommand_index + 1..], &cwd, review)?;

    Ok(CargoVerificationRequest { kind, cwd })
}

fn unwrap_windows_shell_cargo_command(tokens: Vec<String>) -> Result<Vec<String>> {
    let Some(executable) = tokens.first() else {
        return Ok(tokens);
    };
    let executable = Path::new(trim_matching_quotes(executable))
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !matches!(
        executable.to_ascii_lowercase().as_str(),
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe"
    ) {
        return Ok(tokens);
    }

    if tokens.len() != 3 || !trim_matching_quotes(&tokens[1]).eq_ignore_ascii_case("-Command") {
        anyhow::bail!("only the standard PowerShell Cargo transport wrapper is auto-approved");
    }

    let inner = trim_matching_quotes(&tokens[2]);
    if contains_shell_control(inner) {
        anyhow::bail!("shell composition is not allowed");
    }
    cargo_command_tokens(&serde_json::Value::String(inner.to_string()))
}

fn validate_additional_permissions(
    value: &serde_json::Value,
    review: &ReviewContext,
) -> Result<()> {
    let Some(permissions) = value.pointer("/params/additionalPermissions") else {
        return Ok(());
    };
    if permissions.is_null() {
        return Ok(());
    }

    let permissions = permissions
        .as_object()
        .context("additional permissions must be an object")?;
    if permissions
        .keys()
        .any(|key| key != "writableRoots" && key != "fileSystem" && key != "network")
    {
        anyhow::bail!("unsupported additional permission requested");
    }

    if let Some(network) = permissions.get("network") {
        let network = network
            .as_object()
            .context("additional network permission must be an object")?;
        if network.keys().any(|key| key != "enabled") {
            anyhow::bail!("unsupported additional network permission requested");
        }
        if network
            .get("enabled")
            .is_some_and(|enabled| enabled != false)
        {
            anyhow::bail!("network access is not allowed for Cargo verification");
        }
    }

    let mut writable_roots = Vec::new();
    if let Some(roots) = permissions.get("writableRoots") {
        writable_roots.extend(
            roots
                .as_array()
                .context("additional writable roots must be an array")?,
        );
    }
    if let Some(file_system) = permissions.get("fileSystem") {
        let file_system = file_system
            .as_object()
            .context("additional file-system permission must be an object")?;
        if file_system.keys().any(|key| key != "write") {
            anyhow::bail!("only additional file-system write access is supported");
        }
        if let Some(roots) = file_system.get("write") {
            writable_roots.extend(
                roots
                    .as_array()
                    .context("additional file-system write roots must be an array")?,
            );
        }
    }

    for root in writable_roots {
        let root = root
            .as_str()
            .context("additional writable root must be a path string")?;
        resolve_allowed_path(root, review).with_context(|| {
            format!("additional writable root {root:?} is outside the workspace")
        })?;
    }

    Ok(())
}

fn cargo_command_tokens(command: &serde_json::Value) -> Result<Vec<String>> {
    match command {
        serde_json::Value::String(command) => {
            if contains_shell_control(command) {
                anyhow::bail!("shell composition is not allowed");
            }
            split_quoted_command(command)
        }
        serde_json::Value::Array(command) => command
            .iter()
            .map(|token| {
                token
                    .as_str()
                    .map(ToString::to_string)
                    .context("command argv contained a non-string value")
            })
            .collect(),
        _ => anyhow::bail!("unsupported command representation"),
    }
}

fn split_quoted_command(command: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;

    for ch in command.chars() {
        match (quote, ch) {
            (Some(expected), current) if current == expected => quote = None,
            (Some(_), current) => token.push(current),
            (None, '\'' | '"') => quote = Some(ch),
            (None, current) if current.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            (None, current) => token.push(current),
        }
    }

    if quote.is_some() {
        anyhow::bail!("command contains an unmatched quote");
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}

fn contains_shell_control(command: &str) -> bool {
    command.chars().any(|ch| {
        matches!(
            ch,
            '\0' | '\r' | '\n' | ';' | '&' | '|' | '<' | '>' | '`' | '$' | '^'
        )
    }) || command.contains("@(")
}

fn trim_matching_quotes(token: &str) -> &str {
    let token = token.trim();
    if token.len() >= 2 {
        let bytes = token.as_bytes();
        if (bytes[0] == b'"' && bytes[token.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[token.len() - 1] == b'\'')
        {
            return &token[1..token.len() - 1];
        }
    }
    token
}

fn resolve_command_cwd(value: &serde_json::Value, review: &ReviewContext) -> Result<PathBuf> {
    let candidate = match value.pointer("/params/cwd").and_then(|cwd| cwd.as_str()) {
        Some(raw_cwd) => {
            let raw_cwd = PathBuf::from(raw_cwd);
            if raw_cwd.is_absolute() {
                raw_cwd
            } else {
                review.primary_root.join(raw_cwd)
            }
        }
        None => review.primary_root.clone(),
    };

    let candidate_text = candidate.to_string_lossy();
    let cwd = resolve_allowed_path(&candidate_text, review)?;
    if !cwd.is_dir() {
        anyhow::bail!("command cwd {} is not a directory", cwd.display());
    }
    Ok(cwd)
}

fn validate_cargo_scope_arguments(
    arguments: &[String],
    cwd: &Path,
    review: &ReviewContext,
) -> Result<()> {
    let mut index = 0;
    while index < arguments.len() {
        let argument = trim_matching_quotes(&arguments[index]);
        if argument == "--" {
            break;
        }

        if argument == "--config"
            || argument.starts_with("--config=")
            || argument == "--target-dir"
            || argument.starts_with("--target-dir=")
            || argument == "-C"
            || argument.starts_with("-Z")
        {
            anyhow::bail!("Cargo execution-scope overrides are not auto-approved");
        }

        let manifest_path = if argument == "--manifest-path" {
            index += 1;
            Some(
                arguments
                    .get(index)
                    .map(|path| trim_matching_quotes(path))
                    .context("--manifest-path missing its value")?,
            )
        } else {
            argument.strip_prefix("--manifest-path=")
        };

        if let Some(manifest_path) = manifest_path {
            validate_manifest_path(manifest_path, cwd, review)?;
        }

        index += 1;
    }
    Ok(())
}

fn validate_manifest_path(raw_path: &str, cwd: &Path, review: &ReviewContext) -> Result<()> {
    let raw_path = PathBuf::from(raw_path);
    let candidate = if raw_path.is_absolute() {
        raw_path
    } else {
        cwd.join(raw_path)
    };
    let candidate_text = candidate.to_string_lossy();
    let manifest = resolve_allowed_path(&candidate_text, review)?;
    if !manifest.is_file() {
        anyhow::bail!("manifest {} does not exist", manifest.display());
    }
    Ok(())
}

async fn capture_rust_sources_before_format(
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) -> Result<()> {
    let roots = review.allowed_roots.clone();
    let paths = tokio::task::spawn_blocking(move || collect_rust_sources(&roots))
        .await
        .context("joining Rust source scan")??;

    let mut active = active_turn.lock().await;
    let active = active
        .as_mut()
        .context("cargo fmt requested outside an active turn")?;
    for path in paths {
        active.snapshot.capture_before_write(&path).await?;
    }
    Ok(())
}

fn collect_rust_sources(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for root in roots {
        let entries = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !ignored_verification_directory(entry));

        for entry in entries {
            let entry = entry.with_context(|| format!("walking {}", root.display()))?;
            let path = entry.path();

            if entry.file_type().is_symlink() {
                let points_to_directory = std::fs::metadata(path)
                    .map(|metadata| metadata.is_dir())
                    .unwrap_or(false);
                let could_affect_rustfmt = points_to_directory
                    || path.extension().and_then(|extension| extension.to_str()) == Some("rs")
                    || path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml");
                if could_affect_rustfmt {
                    anyhow::bail!(
                        "cannot safely snapshot formatter input through symlink {}",
                        path.display()
                    );
                }
                continue;
            }

            if entry.file_type().is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("rs")
            {
                paths.push(path.to_path_buf());
            }
        }
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn ignored_verification_directory(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && matches!(
            entry.file_name().to_str(),
            Some(".git" | ".gaviero" | "target" | "node_modules")
        )
}

async fn capture_file_change_start(
    value: &serde_json::Value,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) {
    let item = value
        .pointer("/params/item")
        .unwrap_or(&serde_json::Value::Null);
    let item_id = item
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or_default()
        .to_string();

    let paths = resolve_file_change_paths(item, review);
    let mut active = active_turn.lock().await;
    let Some(active) = active.as_mut() else {
        tracing::warn!("codex app-server: fileChange started outside an active turn");
        return;
    };

    active.seen_file_items.insert(item_id.clone());

    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => {
            active.declined_file_items.insert(item_id.clone());
            review.observer.on_message_complete(
                "system",
                &format!("Declined unsafe Codex file change {item_id}: {e:#}"),
            );
            return;
        }
    };

    active.item_paths.insert(item_id.clone(), paths.clone());
    for path in paths {
        if let Err(e) = active.snapshot.capture_before_write(&path).await {
            active.declined_file_items.insert(item_id.clone());
            review.observer.on_message_complete(
                "system",
                &format!(
                    "Declined Codex file change {item_id}: could not snapshot {}: {e:#}",
                    path.display()
                ),
            );
        }
    }
}

async fn file_change_is_safe_to_approve(
    item_id: &str,
    active_turn: &SharedActiveTurn,
    review: &ReviewContext,
) -> bool {
    if !review.tool_surface.write_available() {
        tracing::debug!("declining Codex file change: write tools not on the surface");
        return false;
    }
    let mut active = active_turn.lock().await;
    let Some(active) = active.as_mut() else {
        return false;
    };
    if !active.seen_file_items.contains(item_id) || active.declined_file_items.contains(item_id) {
        return false;
    }

    let Some(paths) = active.item_paths.get(item_id).cloned() else {
        return false;
    };
    for path in paths {
        let Some(original) = active.snapshot.original(&path) else {
            active.declined_file_items.insert(item_id.to_string());
            return false;
        };
        let current = match read_optional_text(&path).await {
            Ok(current) => current,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "declining Codex file change: approval drift check failed"
                );
                active.declined_file_items.insert(item_id.to_string());
                return false;
            }
        };
        if current.as_deref() != original.as_deref() {
            active.declined_file_items.insert(item_id.to_string());
            review.observer.on_message_complete(
                "system",
                &format!(
                    "Declined Codex file change for {} because the file changed before approval.",
                    path.display()
                ),
            );
            return false;
        }
    }

    true
}

fn resolve_file_change_paths(
    item: &serde_json::Value,
    review: &ReviewContext,
) -> Result<Vec<PathBuf>> {
    let changes = item
        .get("changes")
        .and_then(|changes| changes.as_array())
        .context("fileChange item missing changes")?;
    let mut paths = Vec::new();

    for change in changes {
        let raw_path = change
            .get("path")
            .and_then(|path| path.as_str())
            .context("fileChange entry missing path")?;
        paths.push(resolve_allowed_path(raw_path, review)?);

        if let Some(move_path) = change
            .pointer("/kind/move_path")
            .or_else(|| change.pointer("/kind/movePath"))
            .and_then(|path| path.as_str())
        {
            paths.push(resolve_allowed_path(move_path, review)?);
        }
    }

    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        anyhow::bail!("fileChange item contained no paths");
    }
    Ok(paths)
}

fn resolve_allowed_path(raw_path: &str, review: &ReviewContext) -> Result<PathBuf> {
    let raw = PathBuf::from(raw_path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        review.primary_root.join(raw)
    };
    let normalized = normalize_lexically(&candidate)
        .with_context(|| format!("invalid path {}", candidate.display()))?;

    if review.root_for_path(&normalized).is_none() {
        anyhow::bail!(
            "path {} is outside the configured workspace roots",
            normalized.display()
        );
    }

    let existing_ancestor = nearest_existing_ancestor(&normalized)
        .with_context(|| format!("no existing ancestor for {}", normalized.display()))?;
    let canonical_ancestor = crate::util::fs::canonicalize_simplified(&existing_ancestor)
        .with_context(|| format!("canonicalizing {}", existing_ancestor.display()))?;

    let canonically_allowed = review.allowed_roots.iter().any(|root| {
        crate::util::fs::canonicalize_simplified(root)
            .map(|canonical_root| canonical_ancestor.starts_with(canonical_root))
            .unwrap_or(false)
    });
    if !canonically_allowed {
        anyhow::bail!(
            "path {} escapes the workspace through a symlink",
            normalized.display()
        );
    }

    Ok(normalized)
}

fn normalize_lexically(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
        }
    }
    Some(normalized)
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut candidate = path.to_path_buf();
    loop {
        if candidate.exists() {
            return Some(candidate);
        }
        if !candidate.pop() {
            return None;
        }
    }
}

async fn finalize_native_edits(review: &ReviewContext, snapshot: TurnSnapshot) -> Result<()> {
    let mut errors = Vec::new();
    let mut completed = Vec::new();

    for (path, original) in snapshot.edits() {
        match read_optional_text(&path).await {
            Ok(current) if current.as_deref() == original.as_deref() => {}
            Ok(current) => completed.push((path, original, current)),
            Err(e) => {
                errors.push(format!("reading changed file {}: {e:#}", path.display()));
                if let Err(revert_error) = snapshot.revert_path(&path).await {
                    errors.push(format!(
                        "restoring unreadable file {}: {revert_error:#}",
                        path.display()
                    ));
                }
            }
        }
    }

    if !completed.is_empty() {
        review.observer.on_streaming_status(&format!(
            "Processing {} Codex file change{}...",
            completed.len(),
            if completed.len() == 1 { "" } else { "s" }
        ));
    }

    for (path, original, current) in completed {
        let on_disk_now = match read_optional_text(&path).await {
            Ok(content) => content,
            Err(e) => {
                errors.push(format!("drift-checking {}: {e:#}", path.display()));
                continue;
            }
        };
        if on_disk_now.as_deref() != current.as_deref() {
            review.observer.on_message_complete(
                "system",
                &format!(
                    "Disk drifted on {} after Codex completed; leaving it untouched to avoid \
                     clobbering a concurrent write.",
                    path.display()
                ),
            );
            continue;
        }

        if let Err(e) = snapshot.revert_path(&path).await {
            errors.push(format!("restoring {}: {e:#}", path.display()));
            continue;
        }

        let Some(root) = review.root_for_path(&path) else {
            errors.push(format!("no proposal root for {}", path.display()));
            continue;
        };
        let rel_path = path.strip_prefix(root).unwrap_or(path.as_path());

        let proposal_result = match (current.as_deref(), original.as_deref()) {
            (Some(proposed), _) => {
                propose_write(
                    &review.write_gate,
                    review.observer.as_ref(),
                    root,
                    &review.agent_id,
                    review.conv_id.as_deref(),
                    rel_path,
                    proposed,
                )
                .await
            }
            (None, Some(original)) => {
                propose_delete(
                    &review.write_gate,
                    review.observer.as_ref(),
                    root,
                    &review.agent_id,
                    review.conv_id.as_deref(),
                    rel_path,
                    original,
                )
                .await
            }
            (None, None) => Ok(()),
        };

        if let Err(e) = proposal_result {
            errors.push(format!("creating proposal for {}: {e:#}", path.display()));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("\n"))
    }
}

async fn read_optional_text(path: &Path) -> Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

async fn write_shared(stdin: &WeakStdin, message: &str) -> Result<()> {
    let stdin = stdin
        .upgrade()
        .context("codex app-server stdin closed during approval")?;
    let mut stdin = stdin.lock().await;
    write_msg(&mut stdin, message)
        .await
        .context("responding to codex app-server approval")
}

async fn send_to_active(active_turn: &SharedActiveTurn, events: Vec<UnifiedStreamEvent>) {
    let tx = active_turn
        .lock()
        .await
        .as_ref()
        .map(|active| active.tx.clone());
    if let Some(tx) = tx {
        send_events(&tx, events).await;
    } else if !events.is_empty() {
        tracing::warn!("codex app-server: event outside active turn");
    }
}

async fn send_events(tx: &EventSender, events: Vec<UnifiedStreamEvent>) {
    for event in events {
        if tx.send(Ok(event)).await.is_err() {
            break;
        }
    }
}

fn parse_rpc_event(line: &str) -> (Vec<UnifiedStreamEvent>, bool) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        tracing::warn!("codex app-server: malformed JSON: {line}");
        return (vec![], false);
    };

    let Some(method) = value.get("method").and_then(|method| method.as_str()) else {
        return (vec![], false);
    };
    let params = value.get("params").unwrap_or(&serde_json::Value::Null);

    match method {
        "item/agentMessage/delta" => {
            let delta = params
                .get("delta")
                .and_then(|delta| delta.as_str())
                .unwrap_or("")
                .to_string();
            (vec![UnifiedStreamEvent::TextDelta(delta)], false)
        }
        "item/reasoningMessage/delta"
        | "item/reasoning/delta"
        | "item/reasoning/summaryTextDelta" => {
            let delta = params
                .get("delta")
                .and_then(|delta| delta.as_str())
                .unwrap_or("")
                .to_string();
            (vec![UnifiedStreamEvent::ThinkingDelta(delta)], false)
        }
        "item/started" => {
            let item = params.get("item").unwrap_or(&serde_json::Value::Null);
            let id = item
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or("")
                .to_string();
            match item.get("type").and_then(|kind| kind.as_str()) {
                Some("commandExecution") => {
                    let args = match item.get("command") {
                        Some(command) => serde_json::json!({ "command": command }),
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
                }
                Some("fileChange") => (
                    vec![UnifiedStreamEvent::ToolCallStart {
                        id,
                        name: "Edit".to_string(),
                        args: serde_json::json!({
                            "changes": item.get("changes").cloned().unwrap_or_default()
                        }),
                    }],
                    false,
                ),
                Some(kind) if is_codex_subagent_item(kind) => (
                    vec![UnifiedStreamEvent::ToolCallStart {
                        id,
                        name: "Task".to_string(),
                        args: item.clone(),
                    }],
                    false,
                ),
                _ => (vec![], false),
            }
        }
        "item/commandExecution/outputDelta" => {
            let id = params
                .get("itemId")
                .and_then(|id| id.as_str())
                .unwrap_or("")
                .to_string();
            let chunk = params
                .get("deltaBase64")
                .and_then(|delta| delta.as_str())
                .and_then(|encoded| {
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded)
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
        "item/completed" => {
            let item = params.get("item").unwrap_or(&serde_json::Value::Null);
            match item.get("type").and_then(|kind| kind.as_str()) {
                Some("commandExecution") | Some("fileChange") => {
                    let id = item
                        .get("id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("")
                        .to_string();
                    (vec![UnifiedStreamEvent::ToolCallEnd { id }], false)
                }
                Some(kind) if is_codex_subagent_item(kind) => {
                    let id = item
                        .get("id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("")
                        .to_string();
                    (vec![UnifiedStreamEvent::ToolCallEnd { id }], false)
                }
                _ => (vec![], false),
            }
        }
        "turn/completed" => {
            let turn = params.get("turn").unwrap_or(&serde_json::Value::Null);
            let status = turn
                .get("status")
                .and_then(|status| status.as_str())
                .unwrap_or("completed");
            if status == "completed" {
                let usage = turn.get("tokenUsage");
                let input_tokens = usage
                    .and_then(|usage| usage.get("inputTokens"))
                    .and_then(|tokens| tokens.as_u64())
                    .unwrap_or(0);
                let output_tokens = usage
                    .and_then(|usage| usage.get("outputTokens"))
                    .and_then(|tokens| tokens.as_u64())
                    .unwrap_or(0);
                (
                    vec![
                        UnifiedStreamEvent::Usage(TokenUsage {
                            input_tokens,
                            output_tokens,
                            ..Default::default()
                        }),
                        UnifiedStreamEvent::Done(StopReason::EndTurn),
                    ],
                    true,
                )
            } else {
                let message = turn
                    .pointer("/error/message")
                    .and_then(|message| message.as_str())
                    .unwrap_or("turn failed")
                    .to_string();
                (
                    vec![
                        UnifiedStreamEvent::Error(message),
                        UnifiedStreamEvent::Done(StopReason::Error),
                    ],
                    true,
                )
            }
        }
        "turn/started"
        | "turn/diff/updated"
        | "turn/plan/updated"
        | "thread/started"
        | "thread/status/changed"
        | "thread/closed"
        | "thread/archived"
        | "serverRequest/resolved"
        | "item/reasoning/summaryPartAdded" => (vec![], false),
        other => {
            tracing::warn!("codex app-server: unknown event '{other}': {line}");
            (vec![], false)
        }
    }
}

async fn write_msg(stdin: &mut BufWriter<ChildStdin>, message: &str) -> std::io::Result<()> {
    stdin.write_all(message.as_bytes()).await?;
    stdin.flush().await
}

fn error_then_done(
    message: String,
) -> Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>> {
    Box::pin(futures::stream::iter(vec![
        Ok(UnifiedStreamEvent::Error(message)),
        Ok(UnifiedStreamEvent::Done(StopReason::Error)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::types::PlannerMetadata;
    use crate::observer::WriteGateObserver;
    use crate::types::WriteProposal;
    use crate::write_gate::WriteMode;

    struct NoopAcpObserver;

    impl AcpObserver for NoopAcpObserver {
        fn on_stream_chunk(&self, _text: &str) {}
        fn on_tool_call_started(&self, _tool_name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {}
        fn on_proposal_deferred(
            &self,
            _path: &Path,
            _old_content: Option<&str>,
            _new_content: &str,
        ) {
        }
    }

    struct NoopWriteGateObserver;

    impl WriteGateObserver for NoopWriteGateObserver {
        fn on_proposal_created(&self, _proposal: &WriteProposal) {}
        fn on_proposal_updated(&self, _proposal_id: u64) {}
        fn on_proposal_finalized(&self, _path: &str) {}
    }

    fn parse(line: &str) -> (Vec<UnifiedStreamEvent>, bool) {
        parse_rpc_event(line)
    }

    fn review_context(root: &Path, write_gate: Arc<Mutex<WriteGatePipeline>>) -> ReviewContext {
        ReviewContext {
            write_gate,
            observer: Arc::new(NoopAcpObserver),
            primary_root: root.to_path_buf(),
            allowed_roots: vec![root.to_path_buf()],
            agent_id: "codex-test".to_string(),
            conv_id: None,
            tool_surface: AgentToolSurface::unrestricted_unattended(),
        }
    }

    fn command_request(command: serde_json::Value, cwd: &Path) -> serde_json::Value {
        serde_json::json!({
            "method": "item/commandExecution/requestApproval",
            "id": 7,
            "params": {
                "itemId": "command-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "command": command,
                "cwd": cwd.to_string_lossy(),
            }
        })
    }

    fn test_write_gate() -> Arc<Mutex<WriteGatePipeline>> {
        Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::Interactive,
            Box::new(NoopWriteGateObserver),
        )))
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
        let app_server_index = args
            .iter()
            .position(|arg| arg == "app-server")
            .expect("app-server arg");
        let last_config_index = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.as_str() == "--config")
            .map(|(index, _)| index)
            .last()
            .expect("config override");

        assert!(last_config_index < app_server_index);
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--config" && pair[1] == r#"mcp_servers.gaviero.command="gaviero-mcp-shim""#
        }));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--config"
                && pair[1] == r#"mcp_servers.semantic-scholar.url="https://example/mcp/""#
        }));
    }

    #[test]
    fn codex_app_server_args_emit_no_config_when_synth_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            codex_app_server_args(dir.path()),
            vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string()
            ]
        );
    }

    #[test]
    fn thread_policy_uses_native_edit_and_verification_approvals() {
        let params = thread_start_params("gpt-5.6-sol", Path::new("/tmp/work"), &[], false);
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["sandboxPolicy"]["networkAccess"], false);

        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("apply_patch"));
        assert!(instructions.contains("snapshots each native file change"));
        assert!(instructions.contains("Do not print complete files"));
        assert!(instructions.contains("cargo fmt"));
        assert!(instructions.contains("cargo test"));
        assert!(instructions.contains("without shell chaining"));
        assert!(!instructions.contains("All code edits must be proposed"));
    }

    #[test]
    fn resume_and_turn_reassert_native_edit_policy() {
        let resume = thread_resume_params("thread-1", Path::new("/tmp/work"), &[], true);
        assert_eq!(resume["threadId"], "thread-1");
        assert_eq!(resume["approvalPolicy"], "on-request");
        assert_eq!(resume["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(resume["sandboxPolicy"]["networkAccess"], true);

        let turn = turn_start_params("thread-1", "hello", false);
        assert_eq!(turn["approvalPolicy"], "on-request");
        assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(turn["sandboxPolicy"]["networkAccess"], false);
    }

    #[test]
    fn thread_start_appends_sibling_folders() {
        let extras = vec![
            PathBuf::from("/tmp/sibling-a"),
            PathBuf::from("/tmp/sibling-b"),
        ];
        let params = thread_start_params("gpt-5.6-sol", Path::new("/tmp/work"), &extras, false);
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("primary: /tmp/work"));
        assert!(instructions.contains("sibling: /tmp/sibling-a"));
        assert!(instructions.contains("sibling: /tmp/sibling-b"));
    }

    #[test]
    fn render_turn_prompt_preserves_stateless_replay_and_context() {
        use crate::context_planner::ledger::Role;
        use crate::context_planner::{GraphSelection, GraphSelectionKind, ReplayPayload};

        let turn = Turn {
            user_message: "current".to_string(),
            memory_selections: vec![],
            graph_selections: vec![GraphSelection {
                path: None,
                kind: GraphSelectionKind::OutlineOnly,
                token_estimate: 1,
                content: "graph-context".to_string(),
                rank_score: None,
                confidence: None,
                symbols: vec![],
                content_digest: None,
            }],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: Some(ReplayPayload {
                entries: vec![
                    (Role::System, "system guidance".to_string()),
                    (Role::User, "prior question".to_string()),
                    (Role::Assistant, "prior answer".to_string()),
                ],
            }),
            effort: None,
            auto_approve: false,
            metadata: PlannerMetadata::default(),
        };

        let rendered = render_turn_prompt(turn);
        assert!(rendered.contains("current"));
        assert!(rendered.contains("graph-context"));
        assert!(rendered.contains("S: system guidance"));
        assert!(rendered.contains("U: prior question"));
        assert!(rendered.contains("A: prior answer"));
    }

    #[test]
    fn direct_cargo_verification_commands_are_approved() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        let cases = [
            ("cargo build --release", CargoVerificationKind::Build),
            (
                "cargo check --workspace --all-targets",
                CargoVerificationKind::Check,
            ),
            (
                "cargo fmt -p gaviero-core --check",
                CargoVerificationKind::Format,
            ),
            (
                "cargo fmt --all -- --config newline_style=Unix",
                CargoVerificationKind::Format,
            ),
            (
                "cargo test -p gaviero-core native_edits -- --nocapture",
                CargoVerificationKind::Test,
            ),
            (
                "cargo +nightly test --workspace --no-run",
                CargoVerificationKind::Test,
            ),
        ];

        for (command, expected_kind) in cases {
            let request = command_request(serde_json::json!(command), dir.path());
            let parsed = parse_cargo_verification_request(&request, &review).unwrap();
            assert_eq!(parsed.kind, expected_kind, "{command}");
            assert_eq!(parsed.cwd, dir.path());
        }

        let argv_request = command_request(
            serde_json::json!(["cargo.exe", "test", "-p", "gaviero-core"]),
            dir.path(),
        );
        assert_eq!(
            parse_cargo_verification_request(&argv_request, &review)
                .unwrap()
                .kind,
            CargoVerificationKind::Test
        );
    }

    #[test]
    fn windows_powershell_cargo_transport_wrapper_is_approved() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        for executable in [
            "pwsh.exe",
            r#"C:\Program Files\PowerShell\7\pwsh.exe"#,
            "powershell.exe",
        ] {
            let request = command_request(
                serde_json::json!([executable, "-Command", "cargo build --release"]),
                dir.path(),
            );
            assert_eq!(
                parse_cargo_verification_request(&request, &review)
                    .unwrap()
                    .kind,
                CargoVerificationKind::Build
            );
        }

        let captured_request = command_request(
            serde_json::json!(
                r#""C:\Program Files\PowerShell\7\pwsh.exe" -Command 'cargo build --release'"#
            ),
            dir.path(),
        );
        assert_eq!(
            parse_cargo_verification_request(&captured_request, &review)
                .unwrap()
                .kind,
            CargoVerificationKind::Build
        );
    }

    #[test]
    fn unsafe_powershell_wrappers_are_declined() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        for command in [
            serde_json::json!(["pwsh.exe", "-Command", "cargo test; Remove-Item source.rs"]),
            serde_json::json!(["pwsh.exe", "-NoProfile", "-Command", "cargo test"]),
            serde_json::json!(["pwsh.exe", "-Command", "git status"]),
        ] {
            let request = command_request(command, dir.path());
            assert!(parse_cargo_verification_request(&request, &review).is_err());
        }
    }

    #[test]
    fn composed_or_unrelated_commands_are_declined() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        for command in [
            "cargo test; Remove-Item source.rs",
            "cargo test && git clean -fdx",
            "cargo fmt | Out-File result.txt",
            "cargo run",
            "rustfmt src/lib.rs",
            "pwsh -Command cargo test",
            "CARGO_TARGET_DIR=target cargo test",
            "cargo --config net.retry=1 test",
            "cargo test --target-dir C:\\outside",
        ] {
            let request = command_request(serde_json::json!(command), dir.path());
            assert!(
                parse_cargo_verification_request(&request, &review).is_err(),
                "{command}"
            );
        }
    }

    #[test]
    fn command_network_and_extra_permission_escalations_are_declined() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        let mut network = command_request(serde_json::json!("cargo test"), dir.path());
        network["params"]["networkApprovalContext"] =
            serde_json::json!({ "host": "example.com", "protocol": "https" });
        assert!(parse_cargo_verification_request(&network, &review).is_err());

        let mut extra = command_request(serde_json::json!("cargo test"), dir.path());
        extra["params"]["additionalPermissions"] =
            serde_json::json!({ "writableRoots": ["C:\\outside"] });
        assert!(parse_cargo_verification_request(&extra, &review).is_err());
    }

    #[test]
    fn workspace_write_permissions_are_approved_in_current_and_legacy_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());
        let root = dir.path().to_string_lossy();

        for permissions in [
            serde_json::json!({ "fileSystem": { "write": [root.as_ref()] } }),
            serde_json::json!({ "writableRoots": [root.as_ref()] }),
        ] {
            let mut request =
                command_request(serde_json::json!("cargo build --release"), dir.path());
            request["params"]["additionalPermissions"] = permissions;
            assert!(parse_cargo_verification_request(&request, &review).is_ok());
        }
    }

    #[test]
    fn network_and_unknown_additional_permissions_are_declined() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());

        for permissions in [
            serde_json::json!({ "network": { "enabled": true } }),
            serde_json::json!({ "fileSystem": { "read": [dir.path()] } }),
            serde_json::json!({ "unknownCapability": true }),
        ] {
            let mut request = command_request(serde_json::json!("cargo test"), dir.path());
            request["params"]["additionalPermissions"] = permissions;
            assert!(parse_cargo_verification_request(&request, &review).is_err());
        }
    }

    #[test]
    fn cargo_manifest_must_stay_inside_workspace_roots() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside_manifest = workspace.path().join("Cargo.toml");
        let outside_manifest = outside.path().join("Cargo.toml");
        std::fs::write(
            &inside_manifest,
            "[package]\nname='inside'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(
            &outside_manifest,
            "[package]\nname='outside'\nversion='0.1.0'\n",
        )
        .unwrap();

        let review = review_context(workspace.path(), test_write_gate());
        let inside = command_request(
            serde_json::json!(format!(
                "cargo test --manifest-path={}",
                inside_manifest.display()
            )),
            workspace.path(),
        );
        assert!(parse_cargo_verification_request(&inside, &review).is_ok());

        let outside = command_request(
            serde_json::json!(format!(
                "cargo test --manifest-path={}",
                outside_manifest.display()
            )),
            workspace.path(),
        );
        assert!(parse_cargo_verification_request(&outside, &review).is_err());
    }

    #[test]
    fn formatter_snapshot_scan_excludes_build_and_internal_directories() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src/lib.rs");
        let nested = dir.path().join("crates/example/src/main.rs");
        let target = dir.path().join("target/generated.rs");
        let git = dir.path().join(".git/internal.rs");
        let gaviero = dir.path().join(".gaviero/worktrees/other/src/lib.rs");
        let node_modules = dir.path().join("node_modules/package/index.rs");

        for path in [&source, &nested, &target, &git, &gaviero, &node_modules] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fn main() {}\n").unwrap();
        }

        let sources = collect_rust_sources(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(sources, vec![nested, source]);
    }

    #[test]
    fn parse_agent_message_and_reasoning_deltas() {
        let (events, done) = parse(
            r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"hello"}}"#,
        );
        assert_eq!(events, vec![UnifiedStreamEvent::TextDelta("hello".into())]);
        assert!(!done);

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
    fn parse_command_and_file_change_lifecycles() {
        let (events, _) = parse(
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

        let file_line = r#"{"method":"item/started","params":{"item":{"type":"fileChange","id":"edit1","status":"inProgress","changes":[{"path":"src/lib.rs","kind":{"type":"update"},"diff":"@@ -1 +1 @@"}]}}}"#;
        let (events, _) = parse(file_line);
        assert!(matches!(
            &events[0],
            UnifiedStreamEvent::ToolCallStart { id, name, .. }
                if id == "edit1" && name == "Edit"
        ));

        let (events, _) = parse(
            r#"{"method":"item/completed","params":{"item":{"type":"fileChange","id":"edit1","status":"completed","changes":[]}}}"#,
        );
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallEnd { id: "edit1".into() }]
        );
    }

    #[test]
    fn parse_subagent_item_lifecycle() {
        let (events, _) = parse(
            r#"{"method":"item/started","params":{"item":{"type":"task","id":"ag1","description":"scan docs"}}}"#,
        );
        assert!(matches!(
            &events[0],
            UnifiedStreamEvent::ToolCallStart { id, name, .. }
                if id == "ag1" && name == "Task"
        ));
        let (events, _) = parse(
            r#"{"method":"item/completed","params":{"item":{"type":"task","id":"ag1","status":"completed"}}}"#,
        );
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallEnd { id: "ag1".into() }]
        );
    }

    #[test]
    fn parse_command_output_delta_decodes_base64() {
        let encoded = base64::engine::general_purpose::STANDARD.encode("ls\n");
        let line = format!(
            r#"{{"method":"item/commandExecution/outputDelta","params":{{"itemId":"cmd1","deltaBase64":"{encoded}"}}}}"#
        );
        let (events, done) = parse(&line);
        assert!(!done);
        assert_eq!(
            events,
            vec![UnifiedStreamEvent::ToolCallDelta {
                id: "cmd1".into(),
                args_chunk: "ls\n".into(),
            }]
        );
    }

    #[test]
    fn parse_turn_completion() {
        let (events, done) = parse(
            r#"{"method":"turn/completed","params":{"turn":{"id":"t1","status":"completed","tokenUsage":{"inputTokens":10,"outputTokens":5}}}}"#,
        );
        assert!(done);
        assert!(matches!(
            &events[0],
            UnifiedStreamEvent::Usage(usage)
                if usage.input_tokens == 10 && usage.output_tokens == 5
        ));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::EndTurn));

        let (events, done) = parse(
            r#"{"method":"turn/completed","params":{"turn":{"status":"failed","error":{"message":"context exceeded"}}}}"#,
        );
        assert!(done);
        assert_eq!(
            events,
            vec![
                UnifiedStreamEvent::Error("context exceeded".into()),
                UnifiedStreamEvent::Done(StopReason::Error),
            ]
        );
    }

    #[test]
    fn rpc_serializers_match_protocol() {
        let request = rpc_request("initialize", 1, client_info_params());
        let request: serde_json::Value = serde_json::from_str(request.trim()).unwrap();
        assert_eq!(request["method"], "initialize");
        assert_eq!(request["id"], 1);

        let response = rpc_response(
            &serde_json::json!(7),
            serde_json::json!({"decision":"accept"}),
        );
        let response: serde_json::Value = serde_json::from_str(response.trim()).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"]["decision"], "accept");
        assert!(response.get("method").is_none());
    }

    /// Codex's `ClientInfo` schema marks `name` and `version` as required;
    /// dropping either makes `initialize` fail with `-32600 Invalid request`.
    #[test]
    fn initialize_params_carry_required_client_info() {
        let params = client_info_params();
        assert_eq!(params["clientInfo"]["name"], "gaviero");
        assert_eq!(params["clientInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(
            params["clientInfo"]["version"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
    }

    #[test]
    fn path_resolution_rejects_workspace_escape() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());
        assert!(resolve_allowed_path("../outside.txt", &review).is_err());
    }

    #[tokio::test]
    async fn native_edits_are_reverted_and_become_review_proposals() {
        let dir = tempfile::tempdir().unwrap();
        let changed = dir.path().join("changed.txt");
        let created = dir.path().join("created.txt");
        let deleted = dir.path().join("deleted.txt");
        tokio::fs::write(&changed, "before\n").await.unwrap();
        tokio::fs::write(&deleted, "remove me\n").await.unwrap();

        let gate = test_write_gate();
        let review = review_context(dir.path(), gate.clone());

        let mut snapshot = TurnSnapshot::default();
        snapshot.capture_before_write(&changed).await.unwrap();
        snapshot.capture_before_write(&created).await.unwrap();
        snapshot.capture_before_write(&deleted).await.unwrap();

        tokio::fs::write(&changed, "after\n").await.unwrap();
        tokio::fs::write(&created, "new\n").await.unwrap();
        tokio::fs::remove_file(&deleted).await.unwrap();

        finalize_native_edits(&review, snapshot).await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&changed).await.unwrap(),
            "before\n"
        );
        assert!(!created.exists());
        assert_eq!(
            tokio::fs::read_to_string(&deleted).await.unwrap(),
            "remove me\n"
        );

        // Interactive is the mode the TUI runs the gate in, so proposals land
        // in the active map. `pending_proposals()` reads the deferred queue and
        // is empty here by construction.
        let gate = gate.lock().await;
        assert_eq!(gate.active_proposal_ids().len(), 3);

        let changed_proposal = gate.proposal_for_path(&changed).unwrap();
        assert_eq!(changed_proposal.proposed_content, "after\n");
        assert!(!changed_proposal.is_deletion);

        let created_proposal = gate.proposal_for_path(&created).unwrap();
        assert_eq!(created_proposal.proposed_content, "new\n");
        assert!(!created_proposal.is_deletion);

        let deleted_proposal = gate.proposal_for_path(&deleted).unwrap();
        assert!(deleted_proposal.is_deletion);
        assert_eq!(deleted_proposal.original_content, "remove me\n");
    }

    #[tokio::test]
    async fn formatter_changes_join_the_transactional_review() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src/lib.rs");
        tokio::fs::create_dir_all(source.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source, "fn before() {}\n").await.unwrap();

        let gate = test_write_gate();
        let review = review_context(dir.path(), gate.clone());
        let (tx, _rx) = mpsc::channel(1);
        let active_turn = Arc::new(Mutex::new(Some(ActiveTurn::new(tx))));

        capture_rust_sources_before_format(&active_turn, &review)
            .await
            .unwrap();
        tokio::fs::write(&source, "fn after() {}\n").await.unwrap();

        let active = active_turn.lock().await.take().unwrap();
        finalize_native_edits(&review, active.snapshot)
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&source).await.unwrap(),
            "fn before() {}\n"
        );
        let gate = gate.lock().await;
        assert_eq!(gate.active_proposal_ids().len(), 1);
        let proposal = gate.proposal_for_path(&source).unwrap();
        assert_eq!(proposal.proposed_content, "fn after() {}\n");
    }

    #[tokio::test]
    async fn restricted_tool_surface_declines_all_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut review = review_context(dir.path(), test_write_gate());
        review.tool_surface = AgentToolSurface::restricted_no_bash();
        let (tx, _rx) = mpsc::channel(1);
        let active_turn = Arc::new(Mutex::new(Some(ActiveTurn::new(tx))));
        let request = command_request(serde_json::json!("cargo test"), dir.path());
        assert!(
            !command_execution_is_safe_to_approve(&request, &active_turn, &review).await,
            "Bash off the surface must decline even cargo verification"
        );
    }

    #[tokio::test]
    async fn approved_bash_allows_non_denied_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut review = review_context(dir.path(), test_write_gate());
        review.tool_surface = AgentToolSurface::full_bash_approved();
        let (tx, _rx) = mpsc::channel(1);
        let active_turn = Arc::new(Mutex::new(Some(ActiveTurn::new(tx))));
        let allowed = command_request(serde_json::json!("git status"), dir.path());
        assert!(command_execution_is_safe_to_approve(&allowed, &active_turn, &review).await);
        let denied = command_request(serde_json::json!("git push --force origin main"), dir.path());
        assert!(!command_execution_is_safe_to_approve(&denied, &active_turn, &review).await);
    }

    #[tokio::test]
    async fn unattended_fallback_still_approves_cargo_test() {
        let dir = tempfile::tempdir().unwrap();
        let review = review_context(dir.path(), test_write_gate());
        let (tx, _rx) = mpsc::channel(1);
        let active_turn = Arc::new(Mutex::new(Some(ActiveTurn::new(tx))));
        let request = command_request(serde_json::json!("cargo test"), dir.path());
        assert!(command_execution_is_safe_to_approve(&request, &active_turn, &review).await);
        let other = command_request(serde_json::json!("git status"), dir.path());
        assert!(!command_execution_is_safe_to_approve(&other, &active_turn, &review).await);
    }
}
