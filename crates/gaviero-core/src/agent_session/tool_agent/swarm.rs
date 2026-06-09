//! Swarm work-unit entry point for the in-process tool-agent harness (Unit 17).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

use crate::observer::AcpObserver;
use crate::types::FileScope;

use super::client::DeepseekClient;
use super::policy::ToolPolicy;
use super::replay::build_messages;
use super::snapshot::TurnSnapshot;
use super::tools::{ToolCtx, ToolRegistry};
use super::resolve_api_config;

/// Inputs for one swarm work-unit turn through the tool-agent harness.
pub struct SwarmTurnRequest {
    pub model: String,
    pub workspace_root: PathBuf,
    pub additional_roots: Vec<PathBuf>,
    pub scope: FileScope,
    pub system_prompt: String,
    pub user_prompt: String,
    pub allowed_tools: Vec<String>,
    pub auto_approve: bool,
}

/// Outcome of a swarm harness turn.
pub struct SwarmTurnOutcome {
    pub visible: String,
    pub error: Option<String>,
    pub total_cost_usd: f64,
    pub modified_paths: Vec<PathBuf>,
}

/// Run one tool-agent turn for a swarm work unit (Option-B writes, scoped).
pub async fn run_turn(
    req: SwarmTurnRequest,
    observer: &dyn AcpObserver,
    cancel: &CancellationToken,
) -> SwarmTurnOutcome {
    let client = match resolve_api_config(&req.workspace_root) {
        Ok(cfg) => DeepseekClient::new(Ok(cfg)),
        Err(e) => {
            return SwarmTurnOutcome {
                visible: String::new(),
                error: Some(format!("{e:#}")),
                total_cost_usd: 0.0,
                modified_paths: vec![],
            };
        }
    };

    let tools = if req.allowed_tools.is_empty() {
        ToolRegistry::with_writes()
    } else {
        ToolRegistry::from_names(&req.allowed_tools)
    };

    let messages = build_messages(&req.system_prompt, None, &req.user_prompt);
    let snapshot = Arc::new(TokioMutex::new(TurnSnapshot::new()));
    let ctx = ToolCtx {
        workspace_root: req.workspace_root.clone(),
        additional_roots: req.additional_roots,
        scope: req.scope,
        snapshot: Some(snapshot.clone()),
        policy: ToolPolicy::resolve(&req.workspace_root),
        auto_approve: req.auto_approve,
        observer: None,
    };

    let outcome = super::agent_loop::run_agent_loop(
        &client,
        &tools,
        &ctx,
        observer,
        &req.model,
        messages,
        &super::agent_loop::LoopLimits::default(),
        cancel,
    )
    .await;

    let modified_paths = snapshot.lock().await.touched_paths();
    let failed = outcome.error.is_some() || cancel.is_cancelled();
    if failed && !modified_paths.is_empty() {
        if let Err(e) = snapshot.lock().await.revert_all().await {
            tracing::warn!("swarm tool-agent revert on error/cancel failed: {e:#}");
        }
    }

    SwarmTurnOutcome {
        visible: outcome.visible,
        error: outcome.error,
        total_cost_usd: outcome.total_cost_usd,
        modified_paths: if failed { vec![] } else { modified_paths },
    }
}
