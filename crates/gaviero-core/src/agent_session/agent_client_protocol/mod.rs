//! Agent Client Protocol client (Pattern D).
//!
//! `crate::acp` is the **legacy Claude NDJSON transport**, not ACP. This
//! module speaks the Agent Client Protocol over stdio JSON-RPC to
//! `dsh --profile acp` (and the in-tree `fake-acp-agent` test double). File
//! writes go through
//! [`crate::acp::client::propose_write`] so the Write Gate stays in front
//! of every disk change.
//!
//! # Two write channels, one gate
//!
//! The client advertises ACP `fs.writeTextFile` (`initialize`), and a child
//! that uses it is gated inline: the request blocks on `propose_write` before
//! the host answers (see [`handle_incoming`]).
//!
//! Official `dsh` does **not** call client filesystem ops for its own edits —
//! it writes them itself — so a second, post-hoc channel exists for the same
//! intent: [`reconcile_out_of_band_writes`] turns the turn's git dirty-set
//! into the same `WriteProposal`s by restoring each path to its pre-turn
//! content and re-submitting the change. Without it, "the Write Gate is in
//! front of every disk change" would be false for exactly the provider this
//! module exists for, and a long turn's edits would land with no diff to
//! review.

pub mod dsh;
#[cfg(test)]
pub mod fake;
pub mod map;
pub mod rpc;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
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
use crate::agent_session::reconcile::{DirectWrite, read_text_capped, reconcile_direct_writes};
use crate::context_planner::compaction::CompactionPolicy;
use crate::context_planner::types::McpCapabilities;
use crate::context_planner::{ContinuityHandle, ContinuityMode};
use crate::observer::{AcpObserver, PermissionDecision};
use crate::scope_enforcer::ScopeEnforcer;
use crate::swarm::backend::shared::{
    default_editor_system_prompt, render_graph_block, render_memory_block, render_skill_block,
};
use crate::swarm::backend::{Capabilities, RetrievalToolset, StopReason, UnifiedStreamEvent};
use crate::types::FileScope;
use crate::write_gate::WriteGatePipeline;

use super::registry::SessionConstruction;
use super::replay_compaction::compact_turn_replay;
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
    /// `ProviderProfile::max_context_tokens`, for the replay bound.
    max_context_tokens: Option<usize>,
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
    /// ACP `session/new.cwd`. Equal to the primary workspace unless sibling
    /// folders forced an enclosing parent (live dsh rejects
    /// `additionalDirectories`).
    session_cwd: PathBuf,
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
            max_context_tokens: profile.max_context_tokens,
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
            max_context_tokens: profile.max_context_tokens,
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

    async fn ensure_running(&mut self) -> Result<bool> {
        if let Some(running) = self.running.take() {
            self.inner = running.await.context("joining previous ACP turn")?;
        }
        if self.inner.is_some() {
            return Ok(false);
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

        let primary_cwd = absolute_cwd(&self.workspace_root);
        let session_cwd = enclosing_workspace_cwd(&self.workspace_root, &self.additional_roots)
            .unwrap_or_else(|| PathBuf::from(&primary_cwd));
        let lifted = !paths_eq(&session_cwd, Path::new(&primary_cwd));
        // Live dsh rejects a non-empty `additionalDirectories` list. When the
        // session cwd already contains every sibling (enclosing parent), skip
        // the field so `session/new` succeeds on the first try. Nested extras
        // stay inside the primary cwd and are still forwarded for agents that
        // accept the field (in-tree fake).
        let extra_dirs = if lifted {
            Vec::new()
        } else {
            self.additional_roots.clone()
        };
        let servers = mcp_servers_for_session(&self.workspace_root, self.capabilities);
        // Keyed on gaviero's *own* server, not on the list being empty:
        // context7 and `extraServers` can be registered while gaviero's
        // endpoint is down, and `exposedTools` describes gaviero's retrieval
        // tools. Clearing it otherwise would advertise tools that are absent.
        if !registers_gaviero(&servers) {
            self.options.exposed_tools = Some(Vec::new());
            self.observer
                .on_streaming_status("dsh: no MCP endpoint available; retrieval tools disabled");
        }
        let new = session_new(
            &rpc.handle,
            &session_cwd.to_string_lossy(),
            &extra_dirs,
            servers,
        )
        .await?;
        if lifted {
            self.observer.on_streaming_status(&format!(
                "dsh: additionalDirectories unsupported; sibling folders mounted via enclosing cwd {}",
                session_cwd.display()
            ));
        } else if new.additional_dirs_dropped
            && !additional_roots_covered(&session_cwd, &self.additional_roots)
        {
            self.observer.on_streaming_status(
                "dsh: additionalDirectories is not supported; sibling folders are not mounted",
            );
        }
        if new.mcp_dropped {
            self.options.exposed_tools = Some(Vec::new());
            self.observer.on_streaming_status(
                "dsh: session/new rejected mcpServers; retrieval tools disabled",
            );
        }
        let new = new.value;
        let session_id = new
            .get("sessionId")
            .or_else(|| new.get("session_id"))
            .and_then(|v| v.as_str())
            .context("ACP session/new omitted sessionId")?
            .to_string();
        let model = self.model.strip_prefix("dsh:").unwrap_or(&self.model);
        // dsh advertises opaque select values (`JSON.stringify([provider, model])`),
        // not the bare API id. Sending `deepseek-flash` verbatim is
        // `unknown model option` and aborts the turn.
        if let Err(e) = apply_session_model(&rpc.handle, &session_id, &new, model).await {
            tracing::warn!(
                error = %e,
                requested = %model,
                "dsh: could not select requested model; continuing with session default"
            );
            self.observer.on_streaming_status(&format!(
                "dsh: could not select {model}; using the session default"
            ));
        }
        let thinking_settable = thinking_settable
            || config_option_named(&new, "thinking")
            || config_option_named(&new, "effort")
            || config_option_named(&new, "reasoning_effort");
        self.handle = Some(ContinuityHandle::AcpSessionId(session_id.clone()));
        self.inner = Some(LiveSession {
            rpc,
            session_id,
            session_cwd,
            thinking_settable,
            gate_written: HashSet::new(),
        });
        Ok(true)
    }

    async fn apply_effort(
        handle: &JsonRpcHandle,
        session_id: &str,
        thinking_settable: bool,
        effort: &str,
    ) {
        if !thinking_settable {
            tracing::info!("dsh: effort not settable over ACP");
            return;
        }
        let on = !matches!(effort, "off" | "auto" | "");
        // Official dsh-acp advertises `reasoning_effort`; the in-tree fake
        // agent advertises `thinking`. Try both.
        for (config_id, value) in [("reasoning_effort", json!(effort)), ("thinking", json!(on))] {
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

struct SessionNew {
    value: Value,
    mcp_dropped: bool,
    additional_dirs_dropped: bool,
}

async fn session_new(
    handle: &JsonRpcHandle,
    cwd: &str,
    additional_roots: &[PathBuf],
    mut servers: Vec<Value>,
) -> Result<SessionNew> {
    // Live `dsh --profile acp` accepts `additionalDirectories: []` and
    // rejects a non-empty list (`Invalid params: additionalDirectories is
    // not supported`). Callers that can mount siblings by lifting `cwd` to
    // an enclosing parent pass an empty list. Nested extras (still inside
    // the primary cwd) are forwarded, and this retry strips them if the
    // child refuses. The same strip-and-retry applies to `mcpServers`.
    let mut extra_dirs = additional_roots.to_vec();
    let mut mcp_dropped = false;
    let mut additional_dirs_dropped = false;
    loop {
        let params = json!({
            "cwd": cwd,
            "mcpServers": servers,
            "additionalDirectories": extra_dirs,
        });
        match handle.request("session/new", params).await {
            Ok(value) => {
                return Ok(SessionNew {
                    value,
                    mcp_dropped,
                    additional_dirs_dropped,
                });
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if !extra_dirs.is_empty() && session_new_additional_dirs_rejected(&msg) {
                    tracing::warn!(
                        error = %e,
                        "dsh session/new rejected additionalDirectories; retrying with none"
                    );
                    extra_dirs.clear();
                    additional_dirs_dropped = true;
                    continue;
                }
                if !servers.is_empty() && session_new_mcp_rejected(&msg) {
                    tracing::warn!(
                        error = %e,
                        "dsh session/new rejected mcpServers; retrying with none"
                    );
                    servers = Vec::new();
                    mcp_dropped = true;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

fn session_new_mcp_rejected(err: &str) -> bool {
    let l = err.to_ascii_lowercase();
    l.contains("mcpserver") || l.contains("mcp server") || l.contains("mcp_server")
}

fn session_new_additional_dirs_rejected(err: &str) -> bool {
    let l = err.to_ascii_lowercase();
    l.contains("additionaldirectories") || l.contains("additional directories")
}

async fn apply_session_model(
    handle: &JsonRpcHandle,
    session_id: &str,
    session_new: &Value,
    requested: &str,
) -> Result<()> {
    if let Some(option) = find_model_config_option(session_new) {
        let config_id = config_option_id(option).unwrap_or("model");
        if option_already_selects_model(option, requested) {
            return Ok(());
        }
        let value = match_advertised_model(option, requested)
            .ok_or_else(|| anyhow::anyhow!("dsh catalog has no option for {requested}"))?;
        handle
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": session_id,
                    "configId": config_id,
                    "value": value,
                }),
            )
            .await
            .with_context(|| format!("selecting dsh model {requested}"))?;
        return Ok(());
    }
    handle
        .request(
            "session/set_model",
            json!({
                "sessionId": session_id,
                "modelId": requested,
            }),
        )
        .await
        .context("selecting requested dsh model (legacy ACP)")?;
    Ok(())
}

fn find_model_config_option(session_new: &Value) -> Option<&Value> {
    session_new
        .get("configOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|option| {
            option.get("category").and_then(Value::as_str) == Some("model")
                || config_option_id(option) == Some("model")
        })
}

fn config_option_id(option: &Value) -> Option<&str> {
    option
        .get("configId")
        .or_else(|| option.get("id"))
        .and_then(Value::as_str)
}

fn option_already_selects_model(option: &Value, requested: &str) -> bool {
    option
        .get("currentValue")
        .and_then(Value::as_str)
        .is_some_and(|value| model_ids_equivalent(&model_id_from_option_value(value), requested))
}

/// Pick the advertised select value for `requested`.
///
/// Live `dsh --profile acp` (0.1.5-rc.1) keys choices by
/// `JSON.stringify([provider, model])`, grouped under `options`. Bare API ids
/// such as `deepseek-flash` are not in that map. `deepseek-v4-flash` is the
/// catalog alias DeepSeek still serves for V4.1 Flash.
fn match_advertised_model(option: &Value, requested: &str) -> Option<String> {
    let mut best: Option<(u8, u8, String)> = None;
    for value in collect_select_values(option.get("options").unwrap_or(&Value::Null)) {
        let (provider, model) = split_option_route(&value);
        let exact = model == requested;
        if !exact && !model_ids_equivalent(&model, requested) {
            continue;
        }
        let exact_rank = u8::from(!exact);
        let official_rank = u8::from(provider != "deepseek-official" && !provider.is_empty());
        let candidate = (exact_rank, official_rank, value);
        if best.as_ref().is_none_or(|current| candidate < *current) {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, value)| value)
}

fn collect_select_values(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_select_values_into(node, &mut out);
    out
}

fn collect_select_values_into(node: &Value, out: &mut Vec<String>) {
    match node {
        Value::Array(items) => {
            for item in items {
                collect_select_values_into(item, out);
            }
        }
        Value::Object(obj) => {
            if let Some(value) = obj.get("value").and_then(Value::as_str) {
                out.push(value.to_string());
            }
            if let Some(nested) = obj.get("options") {
                collect_select_values_into(nested, out);
            }
        }
        _ => {}
    }
}

fn split_option_route(value: &str) -> (String, String) {
    if let Ok(parts) = serde_json::from_str::<Vec<String>>(value)
        && parts.len() >= 2
    {
        return (parts[0].clone(), parts[1].clone());
    }
    (String::new(), value.to_string())
}

fn model_id_from_option_value(value: &str) -> String {
    split_option_route(value).1
}

fn model_ids_equivalent(left: &str, right: &str) -> bool {
    equivalent_dsh_model_ids(left).contains(&right)
}

fn equivalent_dsh_model_ids(id: &str) -> Vec<&str> {
    match id {
        "deepseek-flash" | "deepseek-v4-flash" => vec!["deepseek-flash", "deepseek-v4-flash"],
        other => vec![other],
    }
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

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn paths_eq(a: &Path, b: &Path) -> bool {
    normalize_path_key(a) == normalize_path_key(b)
}

fn path_is_under(root: &Path, path: &Path) -> bool {
    if path == root || path.starts_with(root) {
        return true;
    }
    let root_s = normalize_path_key(root);
    let path_s = normalize_path_key(path);
    path_s == root_s || path_s.starts_with(&format!("{root_s}/"))
}

fn is_filesystem_root(path: &Path) -> bool {
    let comps: Vec<_> = path.components().collect();
    matches!(
        comps.as_slice(),
        [] | [Component::RootDir]
            | [Component::Prefix(_)]
            | [Component::Prefix(_), Component::RootDir]
    )
}

fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
    a == b || a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// Longest shared prefix of `paths` that is not the drive / filesystem root.
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    let mut prefix: Vec<Component<'_>> = paths[0].components().collect();
    for p in &paths[1..] {
        let comps: Vec<_> = p.components().collect();
        let mut i = 0;
        while i < prefix.len() && i < comps.len() && component_eq(prefix[i], comps[i]) {
            i += 1;
        }
        prefix.truncate(i);
        if prefix.is_empty() {
            return None;
        }
    }
    let ancestor: PathBuf = prefix.into_iter().collect();
    if ancestor.as_os_str().is_empty() || is_filesystem_root(&ancestor) {
        return None;
    }
    Some(ancestor)
}

/// When live dsh cannot take `additionalDirectories`, the nearest common
/// parent of the primary folder and every sibling is used as `session/new.cwd`
/// so tools stay inside one sandbox that still contains the whole workspace.
fn enclosing_workspace_cwd(primary: &Path, additional_roots: &[PathBuf]) -> Option<PathBuf> {
    if additional_roots.is_empty() {
        return None;
    }
    let mut paths = Vec::with_capacity(additional_roots.len() + 1);
    paths.push(PathBuf::from(absolute_cwd(primary)));
    for r in additional_roots {
        if r.as_os_str().is_empty() {
            continue;
        }
        paths.push(PathBuf::from(absolute_cwd(r)));
    }
    if paths.len() < 2 {
        return None;
    }
    common_ancestor(&paths)
}

fn additional_roots_covered(session_cwd: &Path, additional_roots: &[PathBuf]) -> bool {
    !additional_roots.is_empty()
        && additional_roots.iter().all(|r| {
            if r.as_os_str().is_empty() {
                return true;
            }
            path_is_under(session_cwd, Path::new(&absolute_cwd(r)))
        })
}

fn rel_under(root: &Path, path: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        if rel.as_os_str().is_empty() {
            return None;
        }
        return Some(rel.to_path_buf());
    }
    let rel = rel_to_workspace(root, path);
    if rel.as_os_str().is_empty() || rel.is_absolute() {
        None
    } else {
        Some(rel)
    }
}

/// Prompt hint listing sibling folders. When `session_cwd` is an enclosing
/// parent, tools resolve relative paths against that parent, not the primary.
fn workspace_folders_hint(
    workspace_root: &Path,
    additional_roots: &[PathBuf],
    session_cwd: &Path,
) -> Option<String> {
    if additional_roots.is_empty() {
        return None;
    }
    let mut hint = String::from("<workspace_folders>\n");
    hint.push_str(&format!("cwd: {}\n", session_cwd.display()));
    hint.push_str(&format!("primary: {}\n", workspace_root.display()));
    if let Some(rel) = rel_under(session_cwd, workspace_root) {
        hint.push_str(&format!("primary_rel: {}\n", rel.display()));
    }
    for r in additional_roots {
        if r.as_os_str().is_empty() || r == workspace_root {
            continue;
        }
        hint.push_str(&format!("sibling: {}\n", r.display()));
        if let Some(rel) = rel_under(session_cwd, r) {
            hint.push_str(&format!("sibling_rel: {}\n", rel.display()));
        }
    }
    hint.push_str("</workspace_folders>\n");
    if paths_eq(session_cwd, workspace_root) {
        hint.push_str(
            "Read freely from any folder above. File edits land in the primary cwd by default.",
        );
    } else {
        let primary_rel = rel_under(session_cwd, workspace_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| {
                workspace_root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        hint.push_str(&format!(
            "The ACP working directory is {} so tools can reach every folder above. \
             Use paths relative to that cwd (for example `{primary_rel}/src/lib.rs`). \
             Do not assume the primary folder is cwd.",
            session_cwd.display()
        ));
    }
    Some(hint)
}

fn config_option_named(value: &Value, name: &str) -> bool {
    value
        .get("configOptions")
        .or_else(|| {
            value
                .get("agentCapabilities")
                .and_then(|c| c.get("configOptions"))
        })
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

fn build_prompt_blocks(
    turn: &Turn,
    exposed: Option<&[String]>,
    include_replay: bool,
    workspace_root: &Path,
    additional_roots: &[PathBuf],
    session_cwd: &Path,
) -> Vec<Value> {
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
    // Chat constructs a new `AcpClientSession` per turn, so the ACP child
    // starts at `session/new` with empty agent-side history. Host replay is
    // the only continuity; `/reset` clears it by watermarking the panel
    // transcript. A reused live child (swarm, consecutive send_turn on one
    // session) already holds that history — restuffing it would duplicate.
    if include_replay && let Some(payload) = &turn.replay_history {
        for (role, content) in &payload.entries {
            let tag = match role {
                crate::context_planner::ledger::Role::User => "user",
                crate::context_planner::ledger::Role::Assistant => "assistant",
                crate::context_planner::ledger::Role::System => "system",
            };
            parts.push(format!("<{tag}>\n{content}\n</{tag}>"));
        }
    }
    if let Some(hint) = workspace_folders_hint(workspace_root, additional_roots, session_cwd) {
        parts.push(hint);
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
    let root_s = root
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let path_s = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
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

/// Resolve an ACP `fs/*` path. Relative paths are against `session_cwd`
/// (which may be an enclosing parent). If that miss would hide a file that
/// still lives under the primary folder, fall back so a model that still
/// thinks cwd is the primary can read/edit existing files.
fn resolve_acp_path(session_cwd: &Path, workspace_root: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        return p;
    }
    let from_session = abs_in_workspace(session_cwd, raw);
    if paths_eq(session_cwd, workspace_root) || from_session.exists() {
        return from_session;
    }
    let from_primary = abs_in_workspace(workspace_root, raw);
    if from_primary.exists() {
        return from_primary;
    }
    from_session
}

#[allow(clippy::too_many_arguments)]
async fn handle_incoming(
    req: IncomingRequest,
    handle: &JsonRpcHandle,
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &Arc<dyn AcpObserver>,
    workspace_root: &Path,
    session_cwd: &Path,
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
            let abs = resolve_acp_path(session_cwd, workspace_root, raw);
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
            let abs = resolve_acp_path(session_cwd, workspace_root, raw);
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
                    gate_written.insert(norm_rel(&abs));
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
        .filter_map(|e| {
            let p = e.path()?;
            let rel = Path::new(p);
            let abs = if rel.is_absolute() {
                rel.to_path_buf()
            } else {
                root.join(rel)
            };
            Some(norm_rel(&abs))
        })
        .collect()
}

fn git_dirty_roots(roots: &[PathBuf]) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut seen = HashSet::new();
    for root in roots {
        if root.as_os_str().is_empty() {
            continue;
        }
        let key = normalize_path_key(root);
        if !seen.insert(key) {
            continue;
        }
        out.extend(git_dirty(root));
    }
    out
}

// ── Out-of-band write reconciliation ────────────────────────────────────────

/// Total budget for the pre-turn baseline capture. A workspace with a very
/// large dirty set (a mass rename, a regenerated tree) would otherwise pay a
/// full read of every dirty file on every turn; paths past the budget lose
/// their baseline and are consequently left unreconciled.
const RECONCILE_BASELINE_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// Pre-turn content of the paths that were already dirty when the turn began.
///
/// A path that was *clean* at turn start can be reconstructed from git's index.
/// A path that was already dirty cannot: its turn-start bytes exist nowhere
/// else, so they are captured here before the prompt is sent.
///
/// This is the common case, not an edge case — a chat workspace usually has
/// uncommitted accepted edits, and every one of them is a potential target for
/// the next turn.
#[derive(Debug, Default)]
struct PreTurnContent {
    /// `None` = the path did not exist when the turn started.
    known: HashMap<PathBuf, Option<String>>,
    /// Paths that were dirty at turn start but could not be read (binary,
    /// oversized, over budget). A change to one of these has no reconstructible
    /// baseline, so the reconciler must leave the agent's bytes in place.
    unknown: HashSet<PathBuf>,
}

/// Capture the pre-turn content of the currently-dirty paths.
async fn capture_pre_turn_content(root: &Path, dirty: &HashSet<PathBuf>) -> PreTurnContent {
    let mut state = PreTurnContent::default();
    let mut budget = RECONCILE_BASELINE_TOTAL_BYTES;
    let mut order: Vec<&PathBuf> = dirty.iter().collect();
    order.sort();
    for rel in order {
        let abs = if rel.is_absolute() {
            rel.clone()
        } else {
            root.join(rel)
        };
        let size = tokio::fs::metadata(&abs)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if size > budget {
            tracing::debug!(
                path = %rel.display(),
                "dsh: pre-turn baseline budget exhausted; this path will not be reconciled"
            );
            state.unknown.insert(rel.clone());
            continue;
        }
        budget = budget.saturating_sub(size);
        match read_text_capped(&abs).await {
            Ok(content) => {
                state.known.insert(rel.clone(), content);
            }
            Err(e) => {
                tracing::debug!(
                    path = %rel.display(),
                    "dsh: no pre-turn baseline ({e:#}); a change here will be left in place"
                );
                state.unknown.insert(rel.clone());
            }
        }
    }
    state
}

/// The pre-turn content of `rel` as recorded in git's index (stage 0).
///
/// `Some(None)` = not tracked, so the path did not exist in the repository and
/// must have been created during this turn. `None` = git could not answer
/// (no repository, non-UTF-8 blob); the caller must not guess.
fn index_content(root: &Path, rel: &Path) -> Option<Option<String>> {
    let abs = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        root.join(rel)
    };
    let start = if abs.exists() {
        abs.clone()
    } else {
        abs.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(root)
            .to_path_buf()
    };
    let repo = crate::git::GitRepo::open(&start).ok()?;
    let workdir = repo.workdir()?;
    let index_rel = abs.strip_prefix(workdir).ok()?;
    let key = index_rel.to_string_lossy().replace('\\', "/");
    match repo.index_stage_content(&key, 0) {
        Ok(found) => Some(found),
        Err(e) => {
            tracing::warn!(
                path = %rel.display(),
                "dsh: git index lookup failed ({e:#}); leaving the direct write in place"
            );
            None
        }
    }
}

/// Route the paths `dsh` wrote *outside* the ACP client `fs` channel through
/// the Write Gate.
///
/// Detection is the turn's git dirty-set (minus what the child asked the host
/// to write), which says *which* files changed but not *what* changed. To make
/// the change reviewable, each path is:
///
/// 1. mapped to its pre-turn content — the [`PreTurnContent`] baseline for a
///    path that was already dirty, git's index for one that was clean, absence
///    for one created during the turn;
/// 2. restored to that content, so the agent's bytes leave the tree;
/// 3. re-submitted as a `WriteProposal` carrying the real diff.
///
/// The gate then owns the change, which is what makes the mode meaningful:
/// `AutoAccept` writes it straight back (headless/swarm callers observe the
/// same tree as before this pass existed), while `Deferred` / `Interactive` /
/// `RejectAll` leave the tree at its pre-turn state until a human accepts. A
/// path the gate refuses (sensitive, out of scope) therefore stays restored —
/// the refusal *is* the rejection.
///
/// Returns the paths that still carry the agent's bytes: ones whose baseline
/// could not be established (binary, oversized, unknown git state), and ones
/// the gate accepted back onto disk.
async fn reconcile_out_of_band_writes(
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &Arc<dyn AcpObserver>,
    workspace_root: &Path,
    agent_id: &str,
    conv_id: Option<&str>,
    pre_turn: &PreTurnContent,
    changed: &[PathBuf],
) -> Vec<PathBuf> {
    let mut ordered: Vec<PathBuf> = changed.to_vec();
    ordered.sort();
    ordered.dedup();

    // Resolve each path's turn-start baseline from whichever source this
    // pattern actually has. Everything after that point — read the agent's
    // bytes, restore, re-propose — is shared with Pattern C
    // (`agent_session::reconcile`), so the two DeepSeek paths cannot drift
    // into two different review models.
    let mut left_in_place = Vec::new();
    let mut writes = Vec::new();

    for rel in ordered {
        if pre_turn.unknown.contains(&rel) {
            tracing::warn!(
                path = %rel.display(),
                "dsh: no usable pre-turn baseline for a path the agent wrote directly;                  leaving it in place unreviewed"
            );
            left_in_place.push(rel);
            continue;
        }

        let before = match pre_turn.known.get(&rel) {
            Some(content) => content.clone(),
            None => {
                // Clean at turn start: the index is the turn-start content. Not
                // in the index at all means the path appeared during the turn.
                let root = workspace_root.to_path_buf();
                let key = rel.clone();
                let looked_up =
                    tokio::task::spawn_blocking(move || index_content(&root, &key)).await;
                match looked_up.unwrap_or(None) {
                    Some(content) => content,
                    None => {
                        tracing::warn!(
                            path = %rel.display(),
                            "dsh: cannot reconstruct the pre-turn content of a directly written                              path; leaving it in place unreviewed"
                        );
                        left_in_place.push(rel);
                        continue;
                    }
                }
            }
        };

        writes.push(DirectWrite {
            rel_path: rel,
            before,
        });
    }

    let outcome = reconcile_direct_writes(
        write_gate,
        observer.as_ref(),
        workspace_root,
        agent_id,
        conv_id,
        writes,
    )
    .await;

    left_in_place.extend(outcome.left_in_place);
    left_in_place
}

#[async_trait::async_trait]
impl AgentSession for AcpClientSession {
    async fn send_turn(
        &mut self,
        mut turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        // Chat replays the transcript into every fresh dsh session; bound it
        // like the other replaying sessions.
        compact_turn_replay(
            &mut turn,
            &CompactionPolicy::default(),
            self.max_context_tokens,
            "dsh",
        );
        let auto_approve = turn.auto_approve || self.options.auto_approve;
        let effort = turn
            .effort
            .clone()
            .unwrap_or_else(|| self.options.effort.clone());
        let include_replay = self.ensure_running().await?;
        let exposed = self.options.exposed_tools.clone();
        let session_cwd = self
            .inner
            .as_ref()
            .expect("ensure_running")
            .session_cwd
            .clone();
        let mut prompt = build_prompt_blocks(
            &turn,
            exposed.as_deref(),
            include_replay,
            &self.workspace_root,
            &self.additional_roots,
            &session_cwd,
        );
        if let Some(system) = &self.system_prompt {
            prompt[0] = json!({ "type": "text", "text": system });
        }
        let live = self.inner.as_mut().expect("ensure_running");
        let handle = live.rpc.handle.clone();
        let session_id = live.session_id.clone();
        let thinking_settable = live.thinking_settable;
        Self::apply_effort(&handle, &session_id, thinking_settable, &effort).await;

        // Host capture: the host diffs the tree itself after the turn, so the
        // git dirty-set baseline and the out-of-band reconcile are skipped.
        let host_capture = self.options.host_capture;
        let (before_dirty, pre_turn) = if host_capture {
            (HashSet::new(), PreTurnContent::default())
        } else {
            let mut dirty_roots = Vec::with_capacity(1 + self.additional_roots.len());
            dirty_roots.push(self.workspace_root.clone());
            dirty_roots.extend(self.additional_roots.iter().cloned());
            let before_dirty =
                tokio::task::spawn_blocking(move || git_dirty_roots(&dirty_roots)).await?;
            // Baseline for the post-turn reconcile. Captured now because a path
            // that is already dirty has no other record of its turn-start bytes.
            let pre_turn = capture_pre_turn_content(&self.workspace_root, &before_dirty).await;
            (before_dirty, pre_turn)
        };
        let started = Instant::now();

        let (tx, rx) = mpsc::channel::<Result<UnifiedStreamEvent>>(256);
        let write_gate = self.write_gate.clone();
        let observer = self.observer.clone();
        let workspace_root = self.workspace_root.clone();
        let additional_roots = self.additional_roots.clone();
        let file_scope = self.file_scope.clone();
        let cancel = self.cancel_token.clone();
        let agent_id = self.agent_id.clone();
        let conv_id = self.conv_id.clone();

        // Take the live session so the task can own the RPC child.
        let mut live = self.inner.take().expect("ensure_running");
        live.gate_written.clear();

        self.running = Some(tokio::spawn(async move {
            let prompt_fut = handle.request_indefinite(
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
                            &live.session_cwd,
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
                if prompt_result.is_some()
                    && live.rpc.incoming.is_empty()
                    && live.rpc.notifications.is_empty()
                {
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
                    &live.session_cwd,
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
                Some(Ok(result)) if host_capture => {
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Done(map::map_stop_reason(&result))))
                        .await;
                }
                Some(Ok(result)) => {
                    let dirty_roots_after = {
                        let mut roots = vec![workspace_root.clone()];
                        roots.extend(additional_roots.iter().cloned());
                        roots
                    };
                    let after =
                        tokio::task::spawn_blocking(move || git_dirty_roots(&dirty_roots_after))
                            .await
                            .unwrap_or_default();
                    let extra: Vec<PathBuf> = after
                        .difference(&before_dirty)
                        .filter(|p| !live.gate_written.contains(*p))
                        .cloned()
                        .collect();
                    let mut reported: Vec<PathBuf> = Vec::new();
                    if !extra.is_empty() {
                        tracing::warn!(
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            paths = ?extra,
                            "dsh wrote outside the client fs channel"
                        );
                        reported = reconcile_out_of_band_writes(
                            &write_gate,
                            &observer,
                            &workspace_root,
                            &agent_id,
                            conv_id.as_deref(),
                            &pre_turn,
                            &extra,
                        )
                        .await;
                        // What is actually on disk once the gate has had its
                        // say: `AutoAccept` re-materializes the change, every
                        // other mode leaves the pre-turn content. Downstream
                        // consumers (swarm validation, merge, loop judges) read
                        // this set, so it must not name a path the gate just
                        // declined to write.
                        let dirty_roots_now = {
                            let mut roots = vec![workspace_root.clone()];
                            roots.extend(additional_roots.iter().cloned());
                            roots
                        };
                        let now =
                            tokio::task::spawn_blocking(move || git_dirty_roots(&dirty_roots_now))
                                .await
                                .unwrap_or_default();
                        for path in now.difference(&before_dirty) {
                            if !live.gate_written.contains(path) && !reported.contains(path) {
                                reported.push(path.clone());
                            }
                        }
                    }
                    reported.sort();
                    reported.dedup();
                    if !reported.is_empty() {
                        let _ = tx
                            .send(Ok(UnifiedStreamEvent::PathsModified(reported)))
                            .await;
                    }
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Done(map::map_stop_reason(&result))))
                        .await;
                }
                Some(Err(e)) => {
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Error(format!("{e:#}"))))
                        .await;
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                        .await;
                }
                None => {
                    let _ = tx
                        .send(Ok(UnifiedStreamEvent::Done(StopReason::Timeout)))
                        .await;
                }
            }
            // Dropping `live` kills the child. Chat wants ProcessBound reuse —
            // we cannot return it to the session from this task. Kill-on-drop
            // is correct for swarm (one session per unit). Chat registry
            // constructs a new session per turn today as well when the
            // wrapper consumes send_turn to completion; the handle still
            // round-trips via ContinuityHandle::AcpSessionId.
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
        assert!(session_new_mcp_rejected(
            "MCP server declaration not allowed"
        ));
        assert!(!session_new_mcp_rejected("cwd must be absolute"));
        assert!(!session_new_mcp_rejected(
            "ACP RPC session/new error -32602: Invalid params: additionalDirectories is not supported"
        ));
    }

    #[test]
    fn session_new_additional_dirs_rejected_detects_live_dsh_error() {
        assert!(session_new_additional_dirs_rejected(
            "ACP RPC session/new error -32602: Invalid params: additionalDirectories is not supported"
        ));
        assert!(session_new_additional_dirs_rejected(
            "additional directories are not supported"
        ));
        assert!(!session_new_additional_dirs_rejected(
            "cwd must be absolute"
        ));
        assert!(!session_new_additional_dirs_rejected(
            "ACP RPC session/new error -32602: non-empty mcpServers rejected"
        ));
    }

    fn grouped_dsh_model_option() -> Value {
        json!({
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": "[\"deepseek-official\",\"deepseek-v4-pro\"]",
            "options": [{
                "group": "deepseek-official",
                "name": "DeepSeek",
                "options": [
                    {
                        "value": "[\"deepseek-official\",\"deepseek-v4-flash\"]",
                        "name": "DeepSeek-V4-Flash"
                    },
                    {
                        "value": "[\"deepseek-official\",\"deepseek-v4-pro\"]",
                        "name": "DeepSeek-V4-Pro"
                    },
                    {
                        "value": "[\"deepseek-official\",\"deepseek-flash\"]",
                        "name": "DeepSeek-V4.1-Flash"
                    }
                ]
            }]
        })
    }

    #[test]
    fn match_advertised_model_prefers_exact_flash_id() {
        let option = grouped_dsh_model_option();
        assert_eq!(
            match_advertised_model(&option, "deepseek-flash").as_deref(),
            Some(r#"["deepseek-official","deepseek-flash"]"#)
        );
    }

    #[test]
    fn match_advertised_model_maps_flash_alias_when_v41_absent() {
        let mut option = grouped_dsh_model_option();
        option["options"][0]["options"] = json!([
            {
                "value": "[\"deepseek-official\",\"deepseek-v4-flash\"]",
                "name": "DeepSeek-V4-Flash"
            },
            {
                "value": "[\"deepseek-official\",\"deepseek-v4-pro\"]",
                "name": "DeepSeek-V4-Pro"
            }
        ]);
        assert_eq!(
            match_advertised_model(&option, "deepseek-flash").as_deref(),
            Some(r#"["deepseek-official","deepseek-v4-flash"]"#)
        );
        assert!(option_already_selects_model(&option, "deepseek-v4-pro"));
        assert!(!option_already_selects_model(&option, "deepseek-flash"));
    }

    #[test]
    fn find_model_config_option_accepts_config_id() {
        let new = json!({
            "sessionId": "s",
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [{"value": "m1"}]
            }]
        });
        let option = find_model_config_option(&new).expect("model option");
        assert_eq!(config_option_id(option), Some("model"));
        assert_eq!(match_advertised_model(option, "m1").as_deref(), Some("m1"));
    }

    fn sample_turn_with_replay() -> Turn {
        Turn {
            user_message: "now".into(),
            memory_selections: vec![],
            graph_selections: vec![],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: Some(crate::context_planner::ReplayPayload {
                entries: vec![
                    (crate::context_planner::ledger::Role::User, "old q".into()),
                    (
                        crate::context_planner::ledger::Role::Assistant,
                        "old a".into(),
                    ),
                ],
            }),
            effort: None,
            auto_approve: false,
            metadata: Default::default(),
        }
    }

    fn prompt_texts(blocks: &[Value]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|v| v.get("text").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn fresh_session_prompt_includes_host_replay() {
        let turn = sample_turn_with_replay();
        let blocks = build_prompt_blocks(&turn, None, true, Path::new("."), &[], Path::new("."));
        let texts = prompt_texts(&blocks);
        assert!(
            texts.iter().any(|t| t.contains("<user>\nold q\n</user>")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("<assistant>\nold a\n</assistant>")),
            "{texts:?}"
        );
        assert_eq!(*texts.last().unwrap(), "now");
    }

    #[test]
    fn reused_session_prompt_skips_host_replay() {
        let turn = sample_turn_with_replay();
        let blocks = build_prompt_blocks(&turn, None, false, Path::new("."), &[], Path::new("."));
        let texts = prompt_texts(&blocks);
        assert!(texts.iter().all(|t| !t.contains("old q")), "{texts:?}");
        assert_eq!(*texts.last().unwrap(), "now");
    }

    #[test]
    fn enclosing_cwd_none_without_siblings() {
        assert!(enclosing_workspace_cwd(Path::new("/work/proj"), &[]).is_none());
    }

    #[test]
    fn enclosing_cwd_is_common_parent_of_siblings() {
        let parent = tempfile::tempdir().unwrap();
        let a = parent.path().join("gaviero");
        let b = parent.path().join("gaviero-flutter");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let cwd = enclosing_workspace_cwd(&a, std::slice::from_ref(&b)).expect("enclosing cwd");
        assert!(path_is_under(&cwd, &a), "cwd={cwd:?} a={a:?}");
        assert!(path_is_under(&cwd, &b), "cwd={cwd:?} b={b:?}");
        assert!(!is_filesystem_root(&cwd));
        assert!(paths_eq(&cwd, parent.path()));
    }

    #[test]
    fn enclosing_cwd_nested_extra_stays_primary() {
        let dir = tempfile::tempdir().unwrap();
        let extra = dir.path().join("additional");
        std::fs::create_dir_all(&extra).unwrap();
        let cwd = enclosing_workspace_cwd(dir.path(), &[extra]).expect("cwd");
        assert!(paths_eq(&cwd, &PathBuf::from(absolute_cwd(dir.path()))));
    }

    #[test]
    fn enclosing_cwd_rejects_filesystem_root() {
        #[cfg(windows)]
        {
            let a = PathBuf::from(r"C:\alpha");
            let b = PathBuf::from(r"C:\beta");
            assert!(enclosing_workspace_cwd(&a, &[b]).is_none());
        }
        #[cfg(not(windows))]
        {
            let a = PathBuf::from("/alpha");
            let b = PathBuf::from("/beta");
            assert!(enclosing_workspace_cwd(&a, &[b]).is_none());
        }
    }

    #[test]
    fn workspace_folders_hint_lists_siblings_and_enclosing_cwd() {
        let parent = tempfile::tempdir().unwrap();
        let primary = parent.path().join("gaviero");
        let sibling = parent.path().join("gaviero-flutter");
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let turn = sample_turn_with_replay();
        let blocks = build_prompt_blocks(&turn, None, false, &primary, &[sibling], parent.path());
        let texts = prompt_texts(&blocks);
        let hint = texts
            .iter()
            .copied()
            .find(|t| t.contains("<workspace_folders>"))
            .expect("workspace_folders block");
        assert!(hint.contains("gaviero-flutter"), "{hint}");
        assert!(hint.contains("primary_rel:"), "{hint}");
        assert!(
            hint.contains("Do not assume the primary folder is cwd"),
            "{hint}"
        );
        assert_eq!(*texts.last().unwrap(), "now");
    }
}
