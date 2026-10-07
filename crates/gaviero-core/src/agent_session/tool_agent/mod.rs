//! In-process API tool-agent harness (DeepSeek V4 Pro plan).
//!
//! This is gaviero's first in-process agent loop: the host calls an
//! OpenAI-compatible chat API, executes the tools the model requests
//! *in-process*, feeds results back, and repeats. The Claude/Codex/Cursor CLIs
//! bring their own loop as subprocesses; here the host owns it. Built as a
//! reusable harness over the [`ApiClient`] trait so the next API provider only
//! implements the trait.
//!
//! **PR-7 (this milestone):** the gaviero MCP retrieval tools (`memory_search`,
//! `blast_radius`, `node_doc`, `repo_outline`, `symbol_search`, `symbol_doc`, …)
//! are callable from the in-process loop — see [`tools::mcp`]. They were
//! previously a follow-up for API providers, which left `deepseek:` and
//! `ollama:` reading the filesystem by hand while the system prompt instructed
//! them to call `blast_radius(path)` they could not reach.

mod agent_loop;
mod attachments;
pub mod client;
pub mod config;
pub mod policy;
mod replay;
mod snapshot;
pub mod swarm;
pub mod tools;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::context_planner::compaction::CompactionPolicy;
use crate::context_planner::{ContinuityHandle, ContinuityMode, ProviderProfile};
use crate::observer::AcpObserver;
use crate::swarm::backend::shared::{
    default_editor_system_prompt, render_graph_block, render_memory_block, render_skill_block,
};
use crate::swarm::backend::{
    Capabilities, RetrievalToolset, StopReason, TokenUsage, UnifiedStreamEvent,
};
use crate::types::FileScope;
use crate::write_gate::WriteGatePipeline;

use super::registry::SessionConstruction;
use super::{AgentSession, Turn};

use self::client::DeepseekClient;
use self::config::ApiClientConfig;
use self::policy::ToolPolicy;
use self::replay::{apply_replay_compaction, build_messages};
use self::snapshot::TurnSnapshot;
use self::tools::{ToolCtx, ToolRegistry};

/// Provider-agnostic request to an [`ApiClient`].
///
/// `messages` are raw OpenAI-compatible message objects — the loop builds
/// `assistant` tool-call turns and `tool` result messages directly — and
/// `tools` is the function-schema array from the [`tools::ToolRegistry`].
#[derive(Clone, Debug)]
pub struct ApiRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub max_tokens: Option<u32>,
    /// Reasoning effort, already mapped onto the provider's vocabulary
    /// (`low | high | max` for DeepSeek's chat API). Thinking mode is always on
    /// for gaviero, so the harness resolves a level for every request and never
    /// sends `off`.
    pub reasoning_effort: Option<String>,
}

/// Normalized event from an [`ApiClient`]. Mirrors the subset of
/// [`UnifiedStreamEvent`] an OpenAI-compatible chat stream produces.
#[derive(Clone, Debug)]
pub enum ApiEvent {
    /// Incremental visible reply text (`delta.content`).
    Text(String),
    /// Incremental reasoning / chain-of-thought text (`delta.reasoning_content`).
    Reasoning(String),
    /// A fully-assembled tool call — its `function.arguments` were reassembled
    /// across SSE fragments and parsed into [`ToolCall::args`].
    ToolCall(ToolCall),
    Usage(TokenUsage),
    Done(StopReason),
    Error(String),
}

/// A model tool call. `args` is the parsed JSON object reassembled from the
/// streamed `function.arguments` fragments.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// A chat-completions client for an OpenAI-compatible API. The harness is
/// generic over this so the next API provider (OpenAI, Gemini, Qwen, …) only
/// implements the trait — the loop, tools, and write integration are shared.
#[async_trait::async_trait]
pub trait ApiClient: Send + Sync {
    async fn complete(
        &self,
        request: ApiRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ApiEvent>> + Send>>>;
}

/// In-process tool-agent session: the multi-round loop in [`agent_loop`] over
/// the registry built from `agent.availableTools`, gaviero's MCP tools,
/// context7, the ask tool, and — connected lazily on the first turn — the
/// tools of every permitted `mcp.extraServers` entry.
///
/// Writes go straight to disk: TUI chat reviews the turn through host capture,
/// and outside it the per-turn snapshot reverts a failed turn. `write_gate` is
/// held only because every session is constructed with one.
pub struct ToolAgentSession {
    client: Box<dyn ApiClient>,
    observer: Arc<dyn AcpObserver>,
    model: String,
    workspace_root: PathBuf,
    additional_roots: Vec<PathBuf>,
    scope: FileScope,
    tools: ToolRegistry,
    /// Names of the gaviero MCP retrieval tools this session holds, in `tools`
    /// order. Empty when no server was threaded in or the allow-list omitted
    /// them; drives [`RetrievalToolset`] so the pull stanza matches reality.
    retrieval_tools: Vec<String>,
    limits: agent_loop::LoopLimits,
    profile: ProviderProfile,
    compaction: CompactionPolicy,
    policy: ToolPolicy,
    /// `AgentOptions::effort`, the host-resolved default for this session.
    /// `Turn::effort` overrides it per turn; thinking mode is never disabled.
    effort: String,
    #[allow(dead_code)] // Option-B writes are direct-to-disk; see the struct doc
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    cancel_token: CancellationToken,
    /// `AgentOptions::host_capture`: the host records and reviews the turn's
    /// changes, so writes need no turn snapshot and nothing is reverted here
    /// on error or cancel (the review shows those turns too).
    host_capture: bool,
    /// Whether [`Self::connect_extra_servers`] has run. Connections live in
    /// the registry's `RemoteMcpTool`s and close when the session drops.
    extra_servers_connected: bool,
}

impl ToolAgentSession {
    /// Construct from the registry's [`SessionConstruction`]. Resolves the
    /// DeepSeek [`ApiClientConfig`] (env/secrets key + default base_url) and
    /// builds a [`DeepseekClient`]. Key-resolution failure is surfaced lazily on
    /// the first turn so construction stays infallible (matches the other arms).
    pub(super) fn new(args: SessionConstruction) -> Self {
        let SessionConstruction {
            write_gate,
            observer,
            workspace_root,
            additional_roots,
            profile,
            cancel_token,
            options,
            mcp_server,
            ..
        } = args;
        let config = resolve_api_config(&workspace_root);
        // Registry membership is *derived* from the tool surface rather than
        // answering "is Bash available?" a second time (Phase 3). Two
        // behaviour notes, both deliberate:
        //
        // * an unpopulated `availableTools` now yields the documented default
        //   (no `Bash`) instead of `full_chat()`;
        // * an explicit `"availableTools": []` yields an empty registry — it
        //   previously fell through to `full_chat()`, handing the most
        //   permissive tool set to the most restrictive setting.
        let surface =
            super::tool_surface::AgentToolSurface::from_agent_options(&options, &workspace_root);
        let mut tools = ToolRegistry::from_names(surface.available());
        // Gaviero's MCP retrieval tools, adapted to the in-process loop. Appended
        // after the fs/exec tools so those keep a stable position in the `tools`
        // array (prompt-cache friendliness). `extend_mcp` returns exactly the
        // names it added, which is what the pull stanza is then built from — so
        // the prompt can only ever name tools this session really holds.
        //
        // MCP visibility is the *server's* to decide: `mcp.permissions` and
        // `mcp.gavieroServer.exposedTools` were applied where the server was
        // built, and `in_process_tool_specs` returns only what survives them.
        // It must **not** be filtered by `options.available_tools` — that is the
        // Claude-shaped fs surface, no MCP tool is a member of it, and passing it
        // as an allow-list silently removed every retrieval tool. See
        // `ToolRegistry::extend_mcp`.
        let retrieval_tools = match &mcp_server {
            Some(server) => tools.extend_mcp(server),
            None => Vec::new(),
        };
        // context7 for the in-process loop (Phase 2d). A *native* tool over
        // context7's REST API rather than an MCP client — `tools/context7.rs`
        // explains why that leg needs no new dependency.
        //
        // The gate is deliberately identical to dsh's `session/new` entry: the
        // provider's table row (`context7_allowed`), the workspace's
        // `mcp.context7.enabled`, *and* `mcp.permissions`. Deriving all three
        // from the same sources is what makes "the same MCPs" a property rather
        // than three parallel implementations that can drift.
        //
        // Names are appended to `tools` but *not* to `retrieval_tools`: the pull
        // stanza describes gaviero's own memory/kb tools, and context7 is
        // external documentation. Its tools are discoverable through their own
        // schema descriptions, as they are for every other provider.
        let mut context7_tools = Vec::new();
        if profile.mcp_capabilities().context7 {
            let workspace = crate::workspace::Workspace::single_folder(workspace_root.clone());
            let ctx7 = crate::mcp::resolve_context7_config(&workspace, Some(&workspace_root));
            let permissions =
                crate::mcp::resolve_mcp_permissions(&workspace, Some(&workspace_root));
            if ctx7.enabled && permissions.server_allowed("context7") {
                context7_tools = tools.extend_context7(ctx7.rest_base());
            }
        }
        if !context7_tools.is_empty() {
            tracing::debug!(
                tools = ?context7_tools,
                "in-process session holds context7 native tools"
            );
        }
        // `AskUserQuestion` for the in-process loop (Phase 4). Registered off
        // the provider's *prompt channel*, not off `agent.availableTools`: the
        // name is not a member of that list for any provider, it is what a
        // provider gains by having a multi-choice channel. Mirrors Claude's
        // `ensure_ask_user_question` injection (`acp/session.rs`), and keeps the
        // table the single answer to "can this provider ask a question?".
        //
        // The tool's answer channel is the observer, which the loop always
        // passes (`send_turn` builds `ctx.observer` from `self.observer`); a
        // session constructed without one reports that as a tool error rather
        // than silently succeeding with no answer.
        let mut ask_tools = Vec::new();
        if profile.prompt_kind.has_multi_choice() {
            ask_tools = tools.extend_ask();
        }
        if !ask_tools.is_empty() {
            tracing::debug!(
                tools = ?ask_tools,
                "in-process session holds the ask tool"
            );
        }
        // Host-resolved shell policy (workspace cascade). Read back off the
        // surface rather than re-resolving: `from_agent_options` already
        // applied the host's policy, and a second resolution is how the two
        // views drift apart.
        let policy = surface.policy().clone();
        let host_capture = options.host_capture;
        let effort = options.effort.clone();
        Self {
            client: Box::new(DeepseekClient::new(config)),
            observer: Arc::from(observer),
            // API model id is unprefixed (`deepseek-v4-pro`); `args.model` is
            // the user-facing `deepseek:…` spec.
            model: profile.model.clone(),
            workspace_root: workspace_root.clone(),
            additional_roots,
            // Chat has no scope restriction; the swarm passes the work unit's
            // owned_paths in Phase 6.
            scope: FileScope::default(),
            tools,
            retrieval_tools,
            limits: resolve_loop_limits(&workspace_root),
            compaction: CompactionPolicy::for_context_window(profile.max_context_tokens),
            profile,
            policy,
            effort,
            write_gate,
            cancel_token,
            host_capture,
            extra_servers_connected: false,
        }
    }

    /// Connect every permitted `mcp.extraServers` entry and register its
    /// tools, once per session.
    ///
    /// Runs at the top of the first `send_turn`: construction is synchronous
    /// while MCP `initialize` + `tools/list` are async, and a session that
    /// never runs a turn never spawns a server. The gate is the one dsh's
    /// `session/new` applies — the provider row, `mcp.extraServers`, and
    /// `mcp.permissions` (server *and* per-tool) — so "the same MCPs" holds by
    /// construction. A server that fails to connect is reported in the stream
    /// and skipped; it never fails the turn.
    async fn connect_extra_servers(&mut self) {
        if self.extra_servers_connected {
            return;
        }
        self.extra_servers_connected = true;
        if !self.profile.extra_servers_allowed {
            return;
        }
        let ws = crate::workspace::Workspace::single_folder(self.workspace_root.clone());
        let root = Some(self.workspace_root.as_path());
        let permissions = crate::mcp::resolve_mcp_permissions(&ws, root);
        let servers: Vec<crate::mcp::ExtraMcpServer> =
            crate::mcp::extra_servers_from_workspace(&ws, root)
                .into_iter()
                .filter(|s| permissions.server_allowed(&s.name))
                .collect();
        if servers.is_empty() {
            return;
        }

        let cwd = self.workspace_root.clone();
        let results = futures::future::join_all(
            servers
                .iter()
                .map(|s| crate::mcp::client::RemoteMcpServer::connect(s, &cwd)),
        )
        .await;
        let mut taken: std::collections::HashSet<String> =
            self.tools.names().into_iter().map(str::to_string).collect();
        for (server, result) in servers.iter().zip(results) {
            match result {
                Ok(remote) => {
                    let tools = tools::remote_mcp::tools_for(
                        Arc::new(remote),
                        |tool| permissions.tool_allowed(&server.name, tool),
                        &mut taken,
                    );
                    let added = self.tools.extend_remote(tools);
                    tracing::info!(server = %server.name, tools = ?added, "mcp.extraServers entry connected");
                }
                Err(e) => {
                    tracing::warn!(server = %server.name, "mcp.extraServers entry unavailable: {e:#}");
                    self.observer.on_stream_chunk(&format!(
                        "[mcp: extra server '{}' unavailable: {e:#}]\n\n",
                        server.name
                    ));
                }
            }
        }
    }

    /// Capabilities advertised to the system-prompt builder. `tool_use=true` +
    /// `supports_file_blocks=false` means the model is taught to edit via
    /// Write/Edit/MultiEdit tool calls, never the in-band `<file>` marker.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            // Model-derived: V4.1-Flash takes images, V4-Pro does not.
            vision: self.vision(),
            extended_thinking: true,
            max_context_tokens: self.profile.max_context_tokens.unwrap_or(0),
            supports_system_prompt: true,
            supports_file_blocks: false,
            // Names of the gaviero MCP tools this session actually holds (empty
            // when no server was wired, or when the allow-list excluded them).
            // Deriving the stanza from live state is what keeps the prompt from
            // instructing the model to call a tool it cannot reach.
            retrieval: RetrievalToolset::from_exposed(&self.retrieval_tools),
        }
    }

    fn vision(&self) -> bool {
        crate::swarm::backend::deepseek::deepseek_supports_vision(&self.model)
    }

    /// Assemble the user-facing prompt from the turn's planner selections.
    /// User message first (mirrors `LegacyAgentSession`), then graph / memory /
    /// skill blocks.
    fn build_prompt(turn: &Turn) -> String {
        let mut parts = vec![turn.user_message.clone()];
        if let Some(b) = render_graph_block(&turn.graph_selections) {
            parts.push(b);
        }
        if let Some(b) = render_memory_block(&turn.memory_selections) {
            parts.push(b);
        }
        if let Some(b) = render_skill_block(&turn.skill_selections) {
            parts.push(b);
        }
        parts.join("\n\n")
    }
}

#[async_trait::async_trait]
impl AgentSession for ToolAgentSession {
    async fn send_turn(
        &mut self,
        mut turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        apply_replay_compaction(&mut turn, &self.compaction, self.profile.max_context_tokens);
        self.connect_extra_servers().await;

        let system = default_editor_system_prompt(&self.capabilities());
        let prompt = Self::build_prompt(&turn);
        let user_content =
            attachments::user_content(prompt, &turn.file_refs, self.vision()).await;
        let messages = build_messages(&system, turn.replay_history.as_ref(), user_content);
        // Per-turn effort, resolved the same way the ACP path resolves it:
        // the turn's value wins over the session default. Thinking mode is
        // always on, so an unset/`off`/`auto` effort becomes DeepSeek's `high`
        // rather than nothing.
        let effort = turn.effort.as_deref().unwrap_or(self.effort.as_str());
        let reasoning_effort =
            crate::swarm::backend::deepseek::deepseek_reasoning_effort(Some(effort));
        let snapshot = Arc::new(TokioMutex::new(TurnSnapshot::new()));
        let ctx = ToolCtx {
            workspace_root: self.workspace_root.clone(),
            additional_roots: self.additional_roots.clone(),
            scope: self.scope.clone(),
            snapshot: Some(snapshot.clone()),
            policy: self.policy.clone(),
            auto_approve: turn.auto_approve,
            observer: Some(self.observer.clone()),
            sensitive: crate::scope_enforcer::SensitivePolicy::resolve(&self.workspace_root),
        };

        let outcome = agent_loop::run_agent_loop(
            self.client.as_ref(),
            &self.tools,
            &ctx,
            self.observer.as_ref(),
            &self.model,
            Some(reasoning_effort),
            messages,
            &self.limits,
            &self.cancel_token,
        )
        .await;

        if outcome.total_cost_usd > 0.0 {
            self.observer
                .as_ref()
                .on_turn_cost_usd(outcome.total_cost_usd);
        }

        // Host capture: the host diffs the tree after the turn and reviews
        // every change (cancelled and failed turns included), so the snapshot
        // here is not used to revert or report anything.
        let had_edits = !self.host_capture && !snapshot.lock().await.is_empty();
        if outcome.error.is_some() || self.cancel_token.is_cancelled() {
            if had_edits && let Err(e) = snapshot.lock().await.revert_all().await {
                tracing::warn!("tool-agent revert on error/cancel failed: {e:#}");
            }
        } else if had_edits {
            // Only the paths matter: the host syncs its open buffers to disk and
            // never undoes an individual file of the set (see the TUI's
            // `agent_writes.rs`).
            let paths = snapshot.lock().await.touched_paths();
            self.observer.as_ref().on_tool_agent_edits(&paths);
        }

        // Fire on_message_complete even on error (parity with
        // ObservedStreamSession) so the post-turn memory pass still runs.
        self.observer
            .as_ref()
            .on_message_complete("assistant", &outcome.visible);

        if let Some(msg) = outcome.error {
            anyhow::bail!(msg);
        }

        Ok(Box::pin(futures::stream::empty()))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.profile.continuity_mode
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        // StatelessReplay: no server-side thread. The ledger owns replay.
        None
    }

    async fn close(self: Box<Self>) {}
}

/// The notice a UI shows when `deepseek:` cannot honour part of the
/// configured tool surface, or `None` when it honours all of it.
///
/// `agent.availableTools` is the single source of truth for every provider,
/// and Claude/Cursor honour names the in-process loop has no implementation
/// for (`WebSearch`, `WebFetch`, `Agent`, `Task`, `TodoWrite`, …). Skipping
/// them silently is how a settings divergence goes unnoticed, so they are
/// *declared* — the same posture [`crate::context_planner::ToolEnforcement::ui_disclosure`]
/// takes for unenforced providers. Names the loop does serve through another
/// channel are not reported: the registry tools, `AskUserQuestion` on a
/// multi-choice prompt channel, `mcp__gaviero*`, `mcp__context7*` when context7
/// is enabled and permitted, and `mcp__<server>*` for a permitted
/// `mcp.extraServers` entry once the provider row allows extra servers. While
/// the row does not, configured extra servers are reported as unreachable.
///
/// Deterministic for a given configuration, because the TUI announces it
/// once per conversation by content.
pub fn tool_agent_disclosure(
    available_tools: &[String],
    profile: &ProviderProfile,
    workspace: &crate::workspace::Workspace,
    root: Option<&Path>,
) -> Option<String> {
    if profile.provider != "deepseek" {
        return None;
    }
    let permissions = crate::mcp::resolve_mcp_permissions(workspace, root);
    let context7_on = profile.context7_allowed
        && crate::mcp::resolve_context7_config(workspace, root).enabled
        && permissions.server_allowed("context7");
    let extras: Vec<String> = crate::mcp::extra_servers_from_workspace(workspace, root)
        .into_iter()
        .filter(|s| permissions.server_allowed(&s.name))
        .map(|s| s.name)
        .collect();

    let mut unsupported: Vec<&str> = Vec::new();
    for name in available_tools {
        let served = if tools::REGISTRY_TOOLS.contains(&name.as_str()) {
            true
        } else if name == crate::acp::session::ASK_USER_QUESTION_TOOL {
            profile.prompt_kind.has_multi_choice()
        } else if let Some(rest) = name.strip_prefix("mcp__") {
            let server = rest.split("__").next().unwrap_or(rest);
            match server {
                "gaviero" => true,
                "context7" => context7_on,
                other => profile.extra_servers_allowed && extras.iter().any(|e| e == other),
            }
        } else {
            false
        };
        if !served && !unsupported.contains(&name.as_str()) {
            unsupported.push(name);
        }
    }

    let mut lines: Vec<String> = Vec::new();
    if !unsupported.is_empty() {
        lines.push(format!(
            "deepseek runs gaviero's in-process tool loop, which has no equivalent for {} \
             (named in `agent.availableTools`); this agent cannot use them.",
            unsupported.join(", ")
        ));
    }
    if !profile.extra_servers_allowed && !extras.is_empty() {
        lines.push(format!(
            "`mcp.extraServers` {} cannot be reached by deepseek.",
            extras.join(", ")
        ));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// Resolve the per-turn tool-round cap for this workspace.
///
/// Path-based fallback, mirroring [`ToolPolicy::resolve`]: a host holding a
/// [`crate::workspace::Workspace`] should hand the resolved value down, but no
/// [`SessionConstruction`] field carries it — `AgentOptions` has no workspace.
/// Inside a swarm worktree there is no `.gaviero/settings.json` of its own
/// (`.gaviero/**` is gitignored), so the cascade is read from
/// `workspace_root`; falling back to the default is the safe direction, since
/// an under-tight bound costs one hand-off round while an unbounded loop spins.
fn resolve_loop_limits(workspace_root: &Path) -> agent_loop::LoopLimits {
    let ws = crate::workspace::Workspace::single_folder(workspace_root.to_path_buf());
    agent_loop::LoopLimits::from_workspace(&ws, Some(workspace_root))
}

/// Resolve DeepSeek API config from the settings cascade + env/secrets.
///
/// `providers.deepseek.baseUrl` / `.pricing` resolve like every other key
/// (folder → workspace → user `~/.gaviero/settings.json`), root-scoped the same
/// way as [`resolve_loop_limits`]. The legacy snake_case `base_url` is still
/// honoured when the camelCase key is absent at every level.
pub(crate) fn resolve_api_config(workspace_root: &std::path::Path) -> Result<ApiClientConfig> {
    use crate::workspace::settings;
    let ws = crate::workspace::Workspace::single_folder(workspace_root.to_path_buf());
    let root = Some(workspace_root);
    let base_url = ws
        .resolve_setting_opt(settings::PROVIDERS_DEEPSEEK_BASE_URL, root)
        .or_else(|| ws.resolve_setting_opt("providers.deepseek.base_url", root))
        .and_then(|v| v.as_str().map(str::trim).map(str::to_string))
        .filter(|s| !s.is_empty());
    let pricing = ws
        .resolve_setting_opt(settings::PROVIDERS_DEEPSEEK_PRICING, root)
        .and_then(|v| match serde_json::from_value(v) {
            Ok(table) => Some(table),
            Err(e) => {
                tracing::warn!(
                    "ignoring invalid {}: {e}",
                    settings::PROVIDERS_DEEPSEEK_PRICING
                );
                None
            }
        });
    ApiClientConfig::resolve_deepseek(workspace_root, base_url, pricing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(settings: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let gaviero = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gaviero).unwrap();
        std::fs::write(gaviero.join("settings.json"), settings).unwrap();
        std::fs::write(
            gaviero.join("secrets.toml"),
            "[deepseek]\napi_key = \"test-key\"\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn camel_case_provider_keys_resolve_through_the_cascade() {
        let dir = workspace_with(
            r#"{ "providers": { "deepseek": {
                "baseUrl": "https://gateway.example/v1/",
                "base_url": "https://legacy.example",
                "pricing": { "cache_hit_in": 1.0, "cache_miss_in": 2.0, "out": 3.0 }
            } } }"#,
        );
        let cfg = resolve_api_config(dir.path()).unwrap();
        assert_eq!(cfg.base_url, "https://gateway.example/v1");
        let pricing = cfg.pricing.expect("pricing override");
        assert_eq!(pricing.out, 3.0);
    }

    #[test]
    fn legacy_snake_case_base_url_is_still_read() {
        let dir = workspace_with(
            r#"{ "providers": { "deepseek": { "base_url": "https://legacy.example" } } }"#,
        );
        let cfg = resolve_api_config(dir.path()).unwrap();
        assert_eq!(cfg.base_url, "https://legacy.example");
        assert!(cfg.pricing.is_none());
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn profile(spec: &str) -> ProviderProfile {
        crate::context_planner::build_provider_profile(
            &crate::context_planner::ModelSpec::parse(spec),
            &crate::context_planner::RuntimeConfig::default(),
        )
    }

    /// The operator's real `agent.availableTools`: four names are served
    /// outside the registry and must not be reported; four have no in-process
    /// equivalent and must be.
    #[test]
    fn disclosure_reports_only_unserved_tool_names() {
        let dir = workspace_with(r#"{ "mcp": { "context7": { "enabled": true } } }"#);
        let ws = crate::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let available = names(&[
            "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash", "AskUserQuestion",
            "WebSearch", "WebFetch", "Agent", "mcp__gaviero", "mcp__context7", "mcp__arxiv",
        ]);
        let notice = tool_agent_disclosure(
            &available,
            &profile("deepseek:deepseek-flash"),
            &ws,
            Some(dir.path()),
        )
        .expect("unserved names must be disclosed");
        assert!(
            notice.contains("WebSearch, WebFetch, Agent, mcp__arxiv"),
            "{notice}"
        );
        for served in ["Bash", "AskUserQuestion", "mcp__gaviero", "mcp__context7"] {
            assert!(!notice.contains(served), "{served} is served: {notice}");
        }
    }

    /// `deepseek:` reaches `mcp.extraServers` through its own client, so a
    /// configured extra server is served, not disclosed — unless
    /// `mcp.permissions` denies it.
    #[test]
    fn configured_extra_servers_are_served_unless_denied() {
        let dir = workspace_with(
            r#"{ "mcp": { "extraServers": [ { "name": "arxiv", "command": "arxiv-mcp" } ] } }"#,
        );
        let ws = crate::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let available = names(&["Read", "mcp__arxiv"]);
        let deepseek = profile("deepseek:deepseek-v4-pro");
        assert!(deepseek.extra_servers_allowed);
        assert_eq!(
            tool_agent_disclosure(&available, &deepseek, &ws, Some(dir.path())),
            None
        );

        let dir = workspace_with(
            r#"{ "mcp": {
                "extraServers": [ { "name": "arxiv", "command": "arxiv-mcp" } ],
                "permissions": { "deny": ["arxiv:*"] }
            } }"#,
        );
        let ws = crate::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let notice = tool_agent_disclosure(&available, &deepseek, &ws, Some(dir.path()))
            .expect("a denied server is not served");
        assert!(notice.contains("mcp__arxiv"), "{notice}");
    }

    #[test]
    fn disclosure_is_silent_when_everything_is_served_or_not_deepseek() {
        let dir = workspace_with("{}");
        let ws = crate::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let fs_only = names(&["Read", "Grep", "Bash", "mcp__gaviero"]);
        assert_eq!(
            tool_agent_disclosure(&fs_only, &profile("deepseek:deepseek-flash"), &ws, Some(dir.path())),
            None
        );
        let web = names(&["WebSearch"]);
        assert_eq!(
            tool_agent_disclosure(&web, &profile("claude:sonnet"), &ws, Some(dir.path())),
            None
        );
    }

    #[test]
    fn absent_provider_keys_fall_back_to_defaults() {
        let dir = workspace_with("{}");
        let cfg = resolve_api_config(dir.path()).unwrap();
        assert_eq!(cfg.base_url, config::DEFAULT_DEEPSEEK_BASE_URL);
        assert!(cfg.pricing.is_none());
    }
}
