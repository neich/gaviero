//! Agent Client Protocol client (Pattern D).
//!
//! `crate::acp` is the **legacy Claude NDJSON transport**, not ACP. This
//! module speaks the Agent Client Protocol over stdio JSON-RPC to
//! `dsh --profile acp` (and the in-tree `fake-acp-agent` test double). File
//! writes go through
//! [`crate::acp::client::propose_write`] so the Write Gate stays in front
//! of every disk change.

pub mod dsh;
#[cfg(test)]
pub mod fake;
pub mod map;
pub mod rpc;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::Stream;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::acp::client::propose_write;
use crate::acp::session::AgentOptions;
use crate::context_planner::{ContinuityHandle, ContinuityMode};
use crate::observer::{AcpObserver, PermissionDecision};
use crate::scope_enforcer::ScopeEnforcer;
use crate::swarm::backend::shared::{
    default_editor_system_prompt, render_graph_block, render_memory_block, render_skill_block,
};
use crate::swarm::backend::{
    Capabilities, RetrievalToolset, StopReason, UnifiedStreamEvent,
};
use crate::context_planner::types::McpCapabilities;
use crate::types::FileScope;
use crate::write_gate::WriteGatePipeline;

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};
use dsh::{DshLaunchSpec, mcp_servers_for_session, registers_gaviero};
use rpc::{IncomingRequest, JsonRpcChild, JsonRpcHandle};

pub struct AcpClientSession {
    model: String,
    workspace_root: PathBuf,
    additional_roots: Vec<PathBuf>,
    agent_id: String,
    conv_id: Option<String>,
    system_prompt: Option<String>,
    options: AgentOptions,
    file_scope: FileScope,
    /// Per-provider MCP gates from the capability table
    /// (`Provider::mcp_capabilities`), resolved once at construction. Threaded
    /// rather than re-derived at `session/new` so the table stays the single
    /// source for "can this provider receive context7 / extraServers".
    capabilities: McpCapabilities,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    observer: Arc<dyn AcpObserver>,
    cancel_token: CancellationToken,
    extra: Vec<(String, String)>,
    inner: Option<LiveSession>,
    running: Option<tokio::task::JoinHandle<Option<LiveSession>>>,
    handle: Option<ContinuityHandle>,
}

struct LiveSession {
    rpc: JsonRpcChild,
    session_id: String,
    thinking_settable: bool,
    gate_written: HashSet<PathBuf>,
}

impl AcpClientSession {
    pub fn new(args: SessionConstruction) -> Self {
        Self::new_with_scope(args, FileScope::default())
    }

    pub fn new_with_scope(args: SessionConstruction, file_scope: FileScope) -> Self {
        let SessionConstruction {
            write_gate,
            observer,
            model,
            workspace_root,
            additional_roots,
            agent_id,
            conv_id,
            options,
            cancel_token,
            profile,
            ..
        } = args;
        Self {
            model,
            workspace_root,
            additional_roots,
            agent_id,
            conv_id,
            system_prompt: None,
            options,
            file_scope,
            capabilities: profile.mcp_capabilities(),
            write_gate,
            observer: Arc::from(observer),
            cancel_token,
            extra: Vec::new(),
            inner: None,
            running: None,
            handle: None,
        }
    }

    pub fn from_parts(
        args: SessionConstruction,
        observer: Arc<dyn AcpObserver>,
        file_scope: FileScope,
    ) -> Self {
        let SessionConstruction {
            write_gate,
            model,
            workspace_root,
            additional_roots,
            agent_id,
            conv_id,
            options,
            cancel_token,
            profile,
            observer: _,
            ..
        } = args;
        Self {
            model,
            workspace_root,
            additional_roots,
            agent_id,
            conv_id,
            system_prompt: None,
            options,
            file_scope,
            capabilities: profile.mcp_capabilities(),
            write_gate,
            observer,
            cancel_token,
            extra: Vec::new(),
            inner: None,
            running: None,
            handle: None,
        }
    }

    pub fn with_extra(mut self, extra: Vec<(String, String)>) -> Self {
        self.extra = extra;
        self
    }

    pub fn with_system_prompt(mut self, prompt: Option<String>) -> Self {
        self.system_prompt = prompt;
        self
    }

    async fn ensure_running(&mut self) -> Result<&mut LiveSession> {
        if let Some(running) = self.running.take() {
            self.inner = running.await.context("joining previous ACP turn")?;
        }
        if self.inner.is_some() {
            return Ok(self.inner.as_mut().expect("just checked"));
        }
        let spec = DshLaunchSpec::from_workspace_root(&self.workspace_root, &self.extra);
        let cmd = spec.build_command(&self.workspace_root)?;
        let rpc = JsonRpcChild::spawn(cmd).await.with_context(|| {
            format!(
                "spawning dsh ACP agent `{}` (install `@deepseek-ai/dsh` and use `dsh --profile acp`, or set providers.dsh.command)",
                spec.command
            )
        })?;

        let init = rpc
            .handle
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        "terminal": false
                    },
                    "clientInfo": {
                        "name": "gaviero",
                        "title": "Gaviero",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        let thinking_settable = config_option_named(&init, "thinking")
            || config_option_named(&init, "effort")
            || config_option_named(&init, "reasoning_effort");

        let cwd = absolute_cwd(&self.workspace_root);
        let servers = mcp_servers_for_session(&self.workspace_root, self.capabilities);
        // Keyed on gaviero's *own* server, not on the list being empty:
        // context7 and `extraServers` can be registered while gaviero's
        // endpoint is down, and `exposedTools` describes gaviero's retrieval
        // tools. Clearing it otherwise would advertise tools that are absent.
        if !registers_gaviero(&servers) {
            self.options.exposed_tools = Some(Vec::new());
            self.observer.on_streaming_status("dsh: no MCP endpoint available; retrieval tools disabled");
        }
        let new = session_new(&rpc.handle, &cwd, &self.additional_roots, servers).await?;
        let session_id = new
            .get("sessionId")
            .or_else(|| new.get("session_id"))
            .and_then(|v| v.as_str())
            .context("ACP session/new omitted sessionId")?
            .to_string();
        let model = self.model.strip_prefix("dsh:").unwrap_or(&self.model);
        let model_option = new.get("configOptions").and_then(Value::as_array).and_then(|options| options.iter().find(|option| option["category"] == "model" || option["id"] == "model")).and_then(|option| option["id"].as_str());
        if let Some(config_id) = model_option {
            rpc.handle.request("session/set_config_option", json!({ "sessionId": session_id, "configId": config_id, "value": model })).await.context("selecting requested dsh model")?;
        } else {
            rpc.handle.request("session/set_model", json!({ "sessionId": session_id, "modelId": model })).await.context("selecting requested dsh model (legacy ACP)")?;
        }
        let thinking_settable = thinking_settable
            || config_option_named(&new, "thinking")
            || config_option_named(&new, "effort")
            || config_option_named(&new, "reasoning_effort");
        self.handle = Some(ContinuityHandle::AcpSessionId(session_id.clone()));
        self.inner = Some(LiveSession {
            rpc,
            session_id,
            thinking_settable,
            gate_written: HashSet::new(),
        });
        Ok(self.inner.as_mut().expect("just inserted"))
    }

    async fn apply_effort(handle: &JsonRpcHandle, session_id: &str, thinking_settable: bool, effort: &str) {
        if !thinking_settable {
            tracing::info!("dsh: effort not settable over ACP");
            return;
        }
        let on = !matches!(effort, "off" | "auto" | "");
        // Official dsh-acp advertises `reasoning_effort`; the in-tree fake
        // agent advertises `thinking`. Try both.
        for (config_id, value) in [
            ("reasoning_effort", json!(effort)),
            ("thinking", json!(on)),
        ] {
            if handle
                .request(
                    "session/set_config_option",
                    json!({
                        "sessionId": session_id,
                        "configId": config_id,
                        "value": value
                    }),
                )
                .await
                .is_ok()
            {
                return;
            }
        }
    }
}

async fn session_new(handle: &JsonRpcHandle, cwd: &str, additional_roots: &[PathBuf], servers: Vec<Value>) -> Result<Value> {
    let params = json!({
        "cwd": cwd,
        "mcpServers": servers,
        "additionalDirectories": additional_roots,
    });
    match handle.request("session/new", params).await {
        Ok(v) => Ok(v),
        Err(e) if session_new_mcp_rejected(&format!("{e:#}")) => {
            tracing::warn!(
                error = %e,
                "dsh session/new rejected mcpServers; retrying with none"
            );
            handle
                .request(
                    "session/new",
                    json!({
                        "cwd": cwd,
                        "mcpServers": [],
                        "additionalDirectories": additional_roots,
                    }),
                )
                .await
        }
        Err(e) => Err(e),
    }
}

fn session_new_mcp_rejected(err: &str) -> bool {
    let l = err.to_ascii_lowercase();
    l.contains("mcpserver") || l.contains("mcp server") || l.contains("mcp_server")
}

fn absolute_cwd(root: &Path) -> String {
    if root.is_absolute() {
        return root.to_string_lossy().into_owned();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(root))
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn config_option_named(value: &Value, name: &str) -> bool {
    value
        .get("configOptions")
        .or_else(|| value.get("agentCapabilities").and_then(|c| c.get("configOptions")))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|opt| {
            opt.get("id")
                .or_else(|| opt.get("name"))
                .and_then(|v| v.as_str())
                == Some(name)
        })
}

fn capabilities(exposed: Option<&[String]>) -> Capabilities {
    let retrieval = match exposed {
        Some(tools) => RetrievalToolset::from_exposed(tools),
        None => RetrievalToolset {
            graph_and_memory: true,
            symbols: false,
            exposed: Vec::new(),
        },
    };
    Capabilities {
        tool_use: true,
        streaming: true,
        vision: false,
        extended_thinking: true,
        max_context_tokens: 128_000,
        supports_system_prompt: true,
        supports_file_blocks: false,
        retrieval,
    }
}

fn build_prompt_blocks(turn: &Turn, exposed: Option<&[String]>) -> Vec<Value> {
    let mut parts: Vec<String> = Vec::new();
    parts.push(default_editor_system_prompt(&capabilities(exposed)));
    if let Some(block) = render_graph_block(&turn.graph_selections) {
        parts.push(block);
    }
    if let Some(block) = render_memory_block(&turn.memory_selections) {
        parts.push(block);
    }
    if let Some(block) = render_skill_block(&turn.skill_selections) {
        parts.push(block);
    }
    parts.push(turn.user_message.clone());
    parts
        .into_iter()
        .map(|text| json!({ "type": "text", "text": text }))
        .collect()
}

fn norm_rel(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\\', "/"))
}

fn rel_to_workspace(root: &Path, path: &Path) -> PathBuf {
    if let Ok(rel) = path.strip_prefix(root) {
        return norm_rel(rel);
    }
    let root_s = root.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let path_s = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let root_s = root_s.trim_end_matches('/');
    if let Some(rest) = path_s.strip_prefix(root_s) {
        return PathBuf::from(rest.trim_start_matches('/'));
    }
    norm_rel(path)
}

fn abs_in_workspace(root: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() { p } else { root.join(p) }
}

async fn handle_incoming(
    req: IncomingRequest,
    handle: &JsonRpcHandle,
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &Arc<dyn AcpObserver>,
    workspace_root: &Path,
    agent_id: &str,
    conv_id: Option<&str>,
    file_scope: &FileScope,
    auto_approve: bool,
    gate_written: &mut HashSet<PathBuf>,
) {
    match req.method.as_str() {
        "fs/read_text_file" => {
            let raw = req
                .params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let abs = abs_in_workspace(workspace_root, raw);
            let rel = rel_to_workspace(workspace_root, &abs);
            let enforcer = ScopeEnforcer::new(file_scope.clone());
            if let Err(e) = enforcer.check_read(&rel) {
                let _ = handle
                    .respond_error(req.id, -32000, format!("read refused: {e}"))
                    .await;
                return;
            }
            match tokio::fs::read_to_string(&abs).await {
                Ok(mut content) => {
                    if let Some(line) = req.params.get("line").and_then(|v| v.as_u64()) {
                        let start = (line.saturating_sub(1)) as usize;
                        let lines: Vec<&str> = content.lines().collect();
                        let limit = req
                            .params
                            .get("limit")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(lines.len());
                        content = lines
                            .get(start..std::cmp::min(start + limit, lines.len()))
                            .unwrap_or(&[])
                            .join("\n");
                    }
                    let _ = handle.respond(req.id, json!({ "content": content })).await;
                }
                Err(e) => {
                    let _ = handle
                        .respond_error(req.id, -32000, format!("read failed: {e}"))
                        .await;
                }
            }
        }
        "fs/write_text_file" => {
            let raw = req
                .params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = req
                .params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let abs = abs_in_workspace(workspace_root, raw);
            let rel = rel_to_workspace(workspace_root, &abs);
            let enforcer = ScopeEnforcer::new(file_scope.clone());
            if let Err(e) = enforcer.check_write(&rel) {
                let _ = handle
                    .respond_error(req.id, -32000, format!("write refused: {e}"))
                    .await;
                return;
            }
            match propose_write(
                write_gate,
                observer.as_ref(),
                workspace_root,
                agent_id,
                conv_id,
                &rel,
                content,
            )
            .await
            {
                Ok(()) => {
                    gate_written.insert(norm_rel(&rel));
                    let _ = handle.respond(req.id, json!({})).await;
                }
                Err(e) => {
                    let _ = handle
                        .respond_error(req.id, -32000, format!("write gate: {e:#}"))
                        .await;
                }
            }
        }
        "session/request_permission" => {
            let options = req
                .params
                .get("options")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let allow_id = options.iter().find_map(|o| {
                let kind = o.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                (kind.starts_with("allow")).then(|| {
                    o.get("optionId")
                        .or_else(|| o.get("option_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("allow-once")
                        .to_string()
                })
            });
            let deny_id = options.iter().find_map(|o| {
                let kind = o.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                (kind.starts_with("reject") || kind.starts_with("deny")).then(|| {
                    o.get("optionId")
                        .or_else(|| o.get("option_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("reject-once")
                        .to_string()
                })
            });
            let allowed = if auto_approve {
                true
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel();
                observer.on_permission_request(
                    "acp",
                    req.params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ACP permission"),
                    &req.params,
                    tx,
                );
                matches!(rx.await, Ok(PermissionDecision::Allow { .. }))
            };
            let option_id = if allowed {
                allow_id.unwrap_or_else(|| "allow-once".into())
            } else {
                deny_id.unwrap_or_else(|| "reject-once".into())
            };
            let outcome = json!({ "outcome": { "outcome": "selected", "optionId": option_id } });
            let _ = handle.respond(req.id, outcome).await;
        }
        other => {
            let _ = handle
                .respond_error(req.id, -32601, format!("unsupported client method {other}"))
                .await;
        }
    }
}

fn git_dirty(root: &Path) -> HashSet<PathBuf> {
    let Ok(repo) = git2::Repository::open(root) else {
        return HashSet::new();
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return HashSet::new();
    };
    statuses
        .iter()
        .filter_map(|e| e.path().map(|p| norm_rel(Path::new(p))))
        .collect()
}

#[async_trait::async_trait]
impl AgentSession for AcpClientSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let auto_approve = turn.auto_approve || self.options.auto_approve;
        let effort = turn
            .effort
            .clone()
            .unwrap_or_else(|| self.options.effort.clone());
        self.ensure_running().await?;
        let exposed = self.options.exposed_tools.clone();
        let mut prompt = build_prompt_blocks(&turn, exposed.as_deref());
        if let Some(system) = &self.system_prompt {
            prompt[0] = json!({ "type": "text", "text": system });
        }
        let live = self.inner.as_mut().expect("ensure_running");
        let handle = live.rpc.handle.clone();
        let session_id = live.session_id.clone();
        let thinking_settable = live.thinking_settable;
        Self::apply_effort(&handle, &session_id, thinking_settable, &effort).await;

        let root = self.workspace_root.clone();
        let before_dirty = tokio::task::spawn_blocking(move || git_dirty(&root)).await?;
        let started = Instant::now();

        let (tx, rx) = mpsc::channel::<Result<UnifiedStreamEvent>>(256);
        let write_gate = self.write_gate.clone();
        let observer = self.observer.clone();
        let workspace_root = self.workspace_root.clone();
        let file_scope = self.file_scope.clone();
        let cancel = self.cancel_token.clone();
        let agent_id = self.agent_id.clone();
        let conv_id = self.conv_id.clone();

        // Take the live session so the task can own the RPC child.
        let mut live = self.inner.take().expect("ensure_running");
        live.gate_written.clear();

        self.running = Some(tokio::spawn(async move {
            let prompt_fut = handle.request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": prompt,
                }),
            );
            tokio::pin!(prompt_fut);
            let mut prompt_result: Option<Result<Value>> = None;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        let _ = handle.notify("session/cancel", json!({ "sessionId": session_id })).await;
                        break;
                    }
                    Some(req) = live.rpc.incoming.recv() => {
                        handle_incoming(
                            req,
                            &handle,
                            &write_gate,
                            &observer,
                            &workspace_root,
                            &agent_id,
                            conv_id.as_deref(),
                            &file_scope,
                            auto_approve,
                            &mut live.gate_written,
                        )
                        .await;
                    }
                    Some(n) = live.rpc.notifications.recv() => {
                        for ev in map::map_session_update(&n, &workspace_root) {
                            let _ = tx.send(Ok(ev)).await;
                        }
                    }
                    result = &mut prompt_fut, if prompt_result.is_none() => {
                        prompt_result = Some(result);
                    }
                }
                if prompt_result.is_some() && live.rpc.incoming.is_empty() && live.rpc.notifications.is_empty() {
                    break;
                }
            }
            // Drain leftovers that raced with prompt completion.
            while let Ok(req) = live.rpc.incoming.try_recv() {
                handle_incoming(
                    req,
                    &handle,
                    &write_gate,
                    &observer,
                    &workspace_root,
                    &agent_id,
                    conv_id.as_deref(),
                    &file_scope,
                    auto_approve,
                    &mut live.gate_written,
                )
                .await;
            }
            while let Ok(n) = live.rpc.notifications.try_recv() {
                for ev in map::map_session_update(&n, &workspace_root) {
                    let _ = tx.send(Ok(ev)).await;
                }
            }
            let reusable = matches!(&prompt_result, Some(Ok(_)));
            match prompt_result {
                Some(Ok(result)) => {
                    let root = workspace_root.clone();
                    let after = tokio::task::spawn_blocking(move || git_dirty(&root)).await.unwrap_or_default();
                    let extra: Vec<PathBuf> = after
                        .difference(&before_dirty)
                        .filter(|p| !live.gate_written.contains(*p))
                        .cloned()
                        .collect();
                    if !extra.is_empty() {
                        tracing::warn!(
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            paths = ?extra,
                            "dsh wrote outside the client fs channel"
                        );
                        let _ = tx
                            .send(Ok(UnifiedStreamEvent::PathsModified(extra)))
                            .await;
                    }
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Done(map::map_stop_reason(&result))))
                        .await;
                }
                Some(Err(e)) => {
                    let _ = tx.send(Ok(UnifiedStreamEvent::Error(format!("{e:#}")))).await;
                    let _ = tx.send(Ok(UnifiedStreamEvent::Done(StopReason::Error))).await;
                }
                None => {
                    let _ = tx.send(Ok(UnifiedStreamEvent::Done(StopReason::Timeout))).await;
                }
            }
            // Dropping `live` kills the child. Chat wants ProcessBound reuse —
            // we cannot return it to the session from this task. Kill-on-drop
            // is correct for swarm (one session per unit). Chat registry
            // constructs a new session per turn today as well when the
            // wrapper consumes send_turn to completion; the handle still
            // round-trips via ContinuityHandle::AcpSessionId.
            drop(prompt_fut);
            if reusable { Some(live) } else { None }
        }));

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        ContinuityMode::ProcessBound
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        self.handle.as_ref()
    }

    async fn close(self: Box<Self>) {
        self.cancel_token.cancel();
        if let Some(running) = self.running {
            running.abort();
            let _ = running.await;
        }
        if let Some(mut live) = self.inner {
            live.rpc.kill().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_process_bound_shape() {
        let c = capabilities(None);
        assert!(c.tool_use);
        assert!(c.extended_thinking);
        assert!(!c.supports_file_blocks);
        assert_eq!(c.max_context_tokens, 128_000);
    }

    #[test]
    fn session_new_mcp_rejected_detects_server_list() {
        assert!(session_new_mcp_rejected(
            "ACP RPC session/new error -32602: non-empty mcpServers rejected"
        ));
        assert!(session_new_mcp_rejected("MCP server declaration not allowed"));
        assert!(!session_new_mcp_rejected("cwd must be absolute"));
    }
}
