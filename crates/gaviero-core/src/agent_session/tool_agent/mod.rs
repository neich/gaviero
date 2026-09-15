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

/// In-process tool-agent session.
///
/// PR-1 scaffold: a single non-streaming round, no tools. `write_gate` and
/// `cancel_token` are held now so later PRs (the loop + Option-B writes) wire
/// them without changing construction.
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
    #[allow(dead_code)] // Option-B writes are direct-to-disk; gate kept for parity
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    cancel_token: CancellationToken,
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
        let surface = super::tool_surface::AgentToolSurface::from_agent_options(
            &options,
            &workspace_root,
        );
        let mut tools = ToolRegistry::from_names(surface.available());
        // Gaviero's MCP retrieval tools, adapted to the in-process loop. Appended
        // after the fs/exec tools so those keep a stable position in the `tools`
        // array (prompt-cache friendliness). `extend_mcp` returns exactly the
        // names it added, which is what the pull stanza is then built from — so
        // the prompt can only ever name tools this session really holds.
        let retrieval_tools = match &mcp_server {
            Some(server) => tools.extend_mcp(server, options.available_tools.as_deref()),
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
            profile,
            compaction: CompactionPolicy::default(),
            policy,
            write_gate,
            cancel_token,
        }
    }

    /// Capabilities advertised to the system-prompt builder. `tool_use=true` +
    /// `supports_file_blocks=false` means the model is taught to edit via
    /// Write/Edit/MultiEdit tool calls, never the in-band `<file>` marker.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
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

        let system = default_editor_system_prompt(&self.capabilities());
        let prompt = Self::build_prompt(&turn);
        let messages = build_messages(&system, turn.replay_history.as_ref(), &prompt);
        let snapshot = Arc::new(TokioMutex::new(TurnSnapshot::new()));
        let ctx = ToolCtx {
            workspace_root: self.workspace_root.clone(),
            additional_roots: self.additional_roots.clone(),
            scope: self.scope.clone(),
            snapshot: Some(snapshot.clone()),
            policy: self.policy.clone(),
            auto_approve: turn.auto_approve,
            observer: Some(self.observer.clone()),
        };

        let outcome = agent_loop::run_agent_loop(
            self.client.as_ref(),
            &self.tools,
            &ctx,
            self.observer.as_ref(),
            &self.model,
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

        let had_edits = !snapshot.lock().await.is_empty();
        if outcome.error.is_some() || self.cancel_token.is_cancelled() {
            if had_edits {
                if let Err(e) = snapshot.lock().await.revert_all().await {
                    tracing::warn!("tool-agent revert on error/cancel failed: {e:#}");
                }
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

/// Resolve DeepSeek API config from workspace settings + env/secrets.
pub(crate) fn resolve_api_config(workspace_root: &PathBuf) -> Result<ApiClientConfig> {
    let settings_path = workspace_root.join(".gaviero").join("settings.json");
    let (base_url, pricing) = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .map(|doc| {
            let base = doc
                .pointer("/providers/deepseek/base_url")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let pricing = doc
                .pointer("/providers/deepseek/pricing")
                .and_then(|p| serde_json::from_value(p.clone()).ok());
            (base, pricing)
        })
        .unwrap_or((None, None));
    ApiClientConfig::resolve_deepseek(workspace_root, base_url, pricing)
}
