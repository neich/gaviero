//! In-process API tool-agent harness (DeepSeek V4 Pro plan).
//!
//! This is gaviero's first in-process agent loop: the host calls an
//! OpenAI-compatible chat API, executes the tools the model requests
//! *in-process*, feeds results back, and repeats. The Claude/Codex/Cursor CLIs
//! bring their own loop as subprocesses; here the host owns it. Built as a
//! reusable harness over the [`ApiClient`] trait so the next API provider only
//! implements the trait.
//!
//! **PR-6 (this milestone):** cross-turn replay + compaction, API retry/backoff,
//! cancel-aware streaming, per-turn cost telemetry, and docs. MCP graph tools
//! remain a follow-up — see `docs/plans/deepseek_v4_pro_provider.md`.

mod agent_loop;
pub mod client;
pub mod config;
pub mod policy;
mod replay;
mod snapshot;
pub mod swarm;
pub mod tools;

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::context_planner::compaction::CompactionPolicy;
use crate::context_planner::{ContinuityHandle, ContinuityMode, ProviderProfile};
use crate::observer::{AcpObserver, ToolAgentEdit};
use crate::swarm::backend::shared::{
    default_editor_system_prompt, render_graph_block, render_memory_block, render_skill_block,
};
use crate::swarm::backend::{Capabilities, StopReason, TokenUsage, UnifiedStreamEvent};
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
            model,
            workspace_root,
            additional_roots,
            profile,
            cancel_token,
            ..
        } = args;
        let config = resolve_api_config(&workspace_root);
        Self {
            client: Box::new(DeepseekClient::new(config)),
            observer: Arc::from(observer),
            model,
            workspace_root: workspace_root.clone(),
            additional_roots,
            // Chat has no scope restriction; the swarm passes the work unit's
            // owned_paths in Phase 6.
            scope: FileScope::default(),
            tools: ToolRegistry::full_chat(),
            limits: agent_loop::LoopLimits::default(),
            profile,
            compaction: CompactionPolicy::default(),
            policy: ToolPolicy::resolve(&workspace_root),
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
        apply_replay_compaction(
            &mut turn,
            &self.compaction,
            self.profile.max_context_tokens,
        );

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
            let edits: Vec<ToolAgentEdit> = snapshot
                .lock()
                .await
                .edits()
                .into_iter()
                .map(|(path, pre_turn_content)| ToolAgentEdit {
                    path,
                    pre_turn_content,
                })
                .collect();
            self.observer.as_ref().on_tool_agent_edits(&edits);
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
