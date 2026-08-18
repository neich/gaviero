//! Swarm pipeline: validates → tiers → parallel execution → merge.
//!
//! Orchestrates multi-agent execution with git worktree isolation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::sync::{Mutex, Semaphore};

use super::backend::shared;
use super::board::SharedBoard;
use super::bus::AgentBus;
use super::context_bundle::build_bundle;
use super::coordinator::{Coordinator, CoordinatorConfig};
use super::execution_state::{ExecutionState, NodeStatus};
use super::loop_gate::{
    GATE_FEEDBACK_PLACEHOLDER, GATE_OUTPUT_CAP, GateFailure, ProbeDedup, VERIFY_DEDUP_KEY,
    evaluate_deterministic_gate, merge_command_output, run_command_probe, truncate_gate_output,
};
use super::loop_resume;
use super::merge;
use super::models::{AgentManifest, AgentStatus, MergeResult, SwarmResult, WorkUnit};
use super::plan::{CompiledPlan, ExecutionMode};
use super::router::{TierConfig, TierRouter};
use super::validation;
use crate::git::{GitCoordinator, WorktreeManager};
use crate::memory::store::file_hash;
use crate::memory::{MemoryStores, StoreOptions, WriterConfig, WriterHandle, spawn_writer_task};
use crate::observer::{AcpObserver, SwarmObserver};
use crate::types::{EntryMetadata, PrivacyLevel};
use crate::write_gate::{WriteGatePipeline, WriteMode};

/// Build the tier router used for dispatch from the swarm fallback model.
pub fn tier_router_for_model(fallback_model: &str, ollama_base_url: Option<&str>) -> TierRouter {
    let mut tier_config = TierConfig::default();
    let selected_local_model = fallback_model
        .strip_prefix("ollama:")
        .or_else(|| fallback_model.strip_prefix("local:"))
        .map(str::to_string);
    if let Some(base_url) = ollama_base_url {
        tier_config.local.base_url = base_url.to_string();
    }
    if let Some(local_model) = selected_local_model.as_ref() {
        tier_config.local.enabled = true;
        tier_config.local.model = local_model.clone();
        tier_config.cheap_model = local_model.clone();
        tier_config.expensive_model = local_model.clone();
    } else if shared::is_codex_model(fallback_model) {
        // Codex is API-backed like Claude. Propagate to both tier defaults so
        // work units without an explicit `model` override stay on Codex.
        tier_config.cheap_model = fallback_model.to_string();
        tier_config.expensive_model = fallback_model.to_string();
    }
    TierRouter::new(tier_config, selected_local_model.is_some())
}

/// Fail fast if any planned agent (or fan-out default) cannot be dispatched.
///
/// Call before launching agents so invalid `provider:model` specs surface
/// immediately instead of after earlier agents / loop iterations finish.
pub fn preflight_plan_models(
    plan: &CompiledPlan,
    fallback_model: &str,
    ollama_base_url: Option<&str>,
) -> Result<()> {
    shared::validate_model_spec(fallback_model).context("invalid swarm fallback model")?;

    for op in &plan.fanout_ops {
        if let Some(ref model) = op.default_model {
            shared::validate_model_spec(model).with_context(|| {
                format!("invalid fan-out default_model after '{}'", op.after_unit)
            })?;
        }
    }

    let router = tier_router_for_model(fallback_model, ollama_base_url);
    let ordered = plan
        .work_units_ordered()
        .map_err(|e| anyhow::anyhow!("plan graph error: {}", e))?;
    let units: Vec<&WorkUnit> = ordered.iter().chain(plan.loop_judge_units.iter()).collect();
    let errors = validation::validate_backends(&units, &router);
    if !errors.is_empty() {
        let msg = errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!("model / backend preflight failed: {}", msg);
    }
    Ok(())
}

/// Wall-clock budget for a whole swarm run.
///
/// Checked cooperatively rather than by wrapping `execute` in a timeout: a
/// hard cancel would throw away every manifest collected so far, and the
/// artefacts on disk are the point of a document workflow. Because every
/// dispatch is bounded by `WorkUnit::timeout_secs`, the gap between two checks
/// is bounded too, so cooperative checking cannot be starved.
struct RunDeadline {
    start: std::time::Instant,
    budget: Option<std::time::Duration>,
}

impl RunDeadline {
    fn new(secs: u64) -> Self {
        Self {
            start: std::time::Instant::now(),
            budget: (secs > 0).then_some(std::time::Duration::from_secs(secs)),
        }
    }

    /// `Some(elapsed_secs)` once the budget is spent.
    fn expired(&self) -> Option<u64> {
        let budget = self.budget?;
        let elapsed = self.start.elapsed();
        (elapsed >= budget).then_some(elapsed.as_secs())
    }
}

/// Configuration for a swarm execution.
pub struct SwarmConfig {
    /// `Repo` (default): git worktrees, merge, repo-map / code graph.
    /// `Document`: shared workspace, no git lifecycle or code context assembly.
    pub execution_mode: ExecutionMode,
    pub max_parallel: usize,
    pub workspace_root: PathBuf,
    pub model: String,
    pub ollama_base_url: Option<String>,
    pub use_worktrees: bool,
    pub read_namespaces: Vec<String>,
    pub write_namespace: String,
    /// Extra files to inject into each agent's worktree after provisioning.
    /// Populated from `@file` references in the user prompt that are not git-tracked
    /// (e.g. `tmp/` plan documents). Each entry is `(rel_path, content)`.
    pub context_files: Vec<(String, String)>,
    /// Repo-relative or absolute file paths to copy into worktrees when they are
    /// missing from `HEAD` (CLI: `--var` file values and `--prompt-file`).
    /// Agent `read_only` scopes are not scanned — only this list is injected.
    pub worktree_context_paths: Vec<String>,
    /// Folder names or glob patterns to skip when scanning the workspace for
    /// repo-map / code-graph building. Bare names (no `/`) match any directory
    /// basename; entries with `/` are glob-matched against workspace-relative
    /// paths (see [`crate::path_pattern::matches`]).
    pub excludes: Vec<String>,
    /// Optional memory writer supplied by the embedding application. When
    /// absent, `execute` creates a local writer for best-effort memory writes.
    pub memory_writer: Option<WriterHandle>,
    /// Optional MCP config template. The pipeline fills in each agent's
    /// actual worktree before spawning its subprocess backend.
    pub mcp_config: Option<crate::mcp::McpConfigSynth>,
    /// C3: per-build specificity configuration. Embedding applications
    /// (TUI / CLI) should populate this from `workspace.resolve_specificity_config`
    /// so `repoMap.specificity.enabled` and the stop-symbol threshold
    /// take effect. Defaults to enabled with a 0.5 stop-symbol cutoff.
    pub specificity: crate::repo_map::SpecificityConfig,
    /// Workspace-level fallback for swarm tool grants. Populate from
    /// `agent.availableTools` (see `Workspace::resolve_agent_tools`).
    /// Names beyond the swarm base set
    /// (`Read,Glob,Grep,Write,Edit,MultiEdit`) act as implicit
    /// `extra_allowed_tools` for any work unit whose DSL leaves
    /// `tools [...]` unset. DSL declarations always take precedence:
    /// when a unit declares any `tools`, the workspace fallback is
    /// ignored entirely for that unit so the DSL remains the audit
    /// record. Empty = no fallback (legacy behaviour).
    pub swarm_extra_tools: Vec<String>,
    /// When true, each completed agent's findings (task + full text
    /// output) are run through the per-turn memory extractor — the same
    /// `enqueue_post_turn` path as a TUI chat turn — so durable facts are
    /// captured with `source=agent`, low trust, and subject to
    /// dedup/consolidation/decay. This is the curated route; it does
    /// **not** add an MCP write tool. Requires the supplied
    /// `memory_writer` to carry an extraction LLM (the TUI's does; the
    /// headless CLI's fallback writer does not, so leave this `false`
    /// there until a CLI extractor LLM is wired). Default `false`.
    pub extract_agent_findings: bool,
    /// When true (the default for both front ends), a `loop { }` block whose
    /// `OUT_DIR` already holds versioned artefacts resumes after the newest
    /// iteration the whole reviewer panel completed, instead of overwriting
    /// it from `iter_start`. See [`crate::swarm::loop_resume`]. Set `false`
    /// (CLI `--fresh`) to force a clean run.
    pub resume_from_artifacts: bool,
    /// Optional callback to invalidate MCP / repo-map caches after a
    /// file-mutating fan-out wave (TUI wires graph cache drop).
    pub knowledge_invalidation: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Wall-clock budget for the entire swarm run, in seconds. `0` disables it.
    ///
    /// Termination is already guaranteed by `WorkUnit::timeout_secs` bounding
    /// every dispatch; this is the outer belt — a single number an operator can
    /// set to cap a run's cost and latency without reasoning about
    /// `max_iterations × roster × per-agent budget`. On expiry the run stops
    /// where it is and returns the manifests collected so far, so partial
    /// artefacts stay usable and `--resume` can pick up from them.
    pub run_timeout_secs: u64,
}

/// Execute a swarm of work units from a compiled plan.
///
/// 1. Extract work units from plan graph (topological order)
/// 2. Validate scopes (no overlaps) and model / backend preflight
/// 3. Compute dependency tiers
/// 4. For each tier: provision worktrees, run agents in parallel, collect manifests
/// 5. Merge agent branches into main
/// 6. Return SwarmResult
///
/// `initial_state` supports `--resume`: completed nodes are skipped.
pub async fn execute(
    plan: &CompiledPlan,
    config: &SwarmConfig,
    initial_state: Option<ExecutionState>,
    memory: Option<Arc<MemoryStores>>,
    observer: &dyn SwarmObserver,
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver> + Send + Sync,
) -> Result<SwarmResult> {
    tracing::info!(
        agents = plan.graph.node_count(),
        max_parallel = config.max_parallel,
        execution_mode = ?config.execution_mode,
        "swarm.execute starting"
    );

    // Surface workspace-level Bash grant so the security-sensitive
    // weakening of "DSL is the sole place Bash can be granted" is
    // visible in the log every swarm run rather than buried in
    // settings. DSL grants are already part of the unit's checked-in
    // declaration so they don't need a runtime warning.
    if config
        .swarm_extra_tools
        .iter()
        .any(|t| t.eq_ignore_ascii_case("Bash"))
    {
        tracing::warn!(
            target: "swarm",
            extras = ?config.swarm_extra_tools,
            "agent.availableTools grants Bash to swarm units (workspace-level fallback). \
             Per-unit DSL `tools [...]` overrides this. Bash bypasses Write Gate \
             scope validation; remove from settings if unintended."
        );
    }

    // Extract work units in topological order from the plan graph
    let work_units = plan
        .work_units_ordered()
        .map_err(|e| anyhow::anyhow!("plan graph error: {}", e))?;

    // Override max_parallel from plan if declared
    let mut effective_max_parallel = plan.max_parallel.unwrap_or(config.max_parallel);
    let repo_execution = config.execution_mode == ExecutionMode::Repo;
    if !repo_execution
        && let Some(ref mcp) = config.mcp_config
        && crate::mcp::config_synth::synth_has_remote_url_servers(mcp)
        && effective_max_parallel > 1
    {
        tracing::warn!(
            "document mode with remote MCP: reducing max_parallel from {effective_max_parallel} \
             to 1 (shared workspace — concurrent Cursor agents race on .cursor/mcp.json)",
        );
        effective_max_parallel = 1;
    }

    // CLI-referenced context files (outside --repo or uncommitted) are absent
    // from git worktrees checked out at HEAD — inject copies and rewrite paths.
    let read_only_prep = super::worktree_context::prepare_worktree_read_only_context(
        &config.workspace_root,
        &config.worktree_context_paths,
    )?;
    let mut context_files = config.context_files.clone();
    context_files.extend(read_only_prep.injections);
    let work_units = super::worktree_context::apply_worktree_path_rewrites(
        work_units,
        &read_only_prep.path_rewrites,
    );
    let want_worktrees = repo_execution && config.use_worktrees && effective_max_parallel > 1;
    let use_worktrees = if want_worktrees {
        let mgr = WorktreeManager::new(config.workspace_root.clone());
        if !mgr.can_use_worktrees() {
            tracing::warn!(
                "git worktrees unavailable (not a git repo or no commits); \
                 running agents in the shared workspace"
            );
            false
        } else {
            true
        }
    } else {
        false
    };
    let tier_router = tier_router_for_model(&config.model, config.ollama_base_url.as_deref());
    let git_coordinator = Arc::new(GitCoordinator::new());
    let memory_writer = config.memory_writer.clone().or_else(|| {
        memory.as_ref().map(|stores| {
            spawn_writer_task(WriterConfig {
                stores: stores.clone(),
                llm: None,
                observer: None,
                manifest_observer: None,
            })
        })
    });

    // Execution state tracks per-node progress (populated as nodes complete)
    let mut exec_state = initial_state.unwrap_or_else(|| ExecutionState::new_from_plan(plan));
    let plan_hash = plan.hash();

    // Filter out already-completed nodes if resuming
    let work_units: Vec<WorkUnit> = work_units
        .into_iter()
        .filter(|u| {
            let status = exec_state.status(&u.id);
            if status == NodeStatus::Completed {
                tracing::info!("Resuming: skipping already-completed node '{}'", u.id);
                true // Keep in the list but execution will be skipped via exec_state check
            } else {
                true
            }
        })
        .collect();

    // Generate a unique run ID for this execution
    let run_id = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    // Capture HEAD SHA before any merges (for revert support)
    let pre_swarm_sha = if use_worktrees {
        crate::git::current_head_sha(&config.workspace_root).unwrap_or_default()
    } else {
        String::new()
    };

    observer.on_phase_changed("validating");

    // 1. Validate scopes
    let loop_groups = validation::expand_loop_groups_with_roster_init(
        plan.loop_configs
            .iter()
            .map(|lc| lc.agent_ids.clone())
            .collect(),
        &work_units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
    );
    let scope_errors = validation::validate_scopes(&work_units, &loop_groups);
    if !scope_errors.is_empty() {
        let msg = scope_errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("scope validation failed: {}", msg);
    }

    // 2. Validate every agent model / backend can be resolved before launch.
    // Dynamic fan-out workers are checked later when their manifest lands;
    // static units, loop judges, and fan-out default_model are checked here.
    preflight_plan_models(plan, &config.model, config.ollama_base_url.as_deref())?;

    // ── Single-agent fast path ────────────────────────────────────────────────
    // One work unit → bypass worktrees, bus, and merge; run directly through
    // the IterationEngine so strategy / retry / model-escalation all apply.
    if work_units.len() == 1 && plan.loop_configs.is_empty() {
        let unit = work_units.into_iter().next().unwrap();

        // Resume support: skip if already completed.
        if exec_state.status(&unit.id) == NodeStatus::Completed {
            tracing::info!("Single-agent resume: '{}' already complete", unit.id);
            let manifest = AgentManifest {
                work_unit_id: unit.id.clone(),
                status: AgentStatus::Completed,
                modified_files: vec![],
                branch: None,
                summary: Some("already completed (resume)".into()),
                output: None,
                cost_usd: 0.0,
            };
            let swarm_result = SwarmResult {
                manifests: vec![manifest],
                merge_results: vec![],
                success: true,
                pre_swarm_sha,
            };
            observer.on_phase_changed("completed");
            observer.on_completed(&swarm_result);
            return Ok(swarm_result);
        }

        observer.on_phase_changed("running");
        observer.on_agent_state_changed(&unit.id, &AgentStatus::Running, "starting");

        let single_validation: Option<Arc<crate::validation_gate::ValidationPipeline>> =
            if config.workspace_root.join("Cargo.toml").exists() {
                Some(Arc::new(
                    crate::validation_gate::ValidationPipeline::default_for_rust(),
                ))
            } else {
                Some(Arc::new(
                    crate::validation_gate::ValidationPipeline::fast_only(),
                ))
            };
        let analysis = WorkspaceAnalysis::build(config, std::slice::from_ref(&unit)).await;

        let effective_read_ns = effective_read_namespaces(&unit, config, &memory);
        // Single-agent fast path: no shared board, no bundle pre-fetch
        // (coordinator + runner = 2 queries, already within M7 ≤2 gate).
        let agent_ctx = AgentRunContext::for_run(
            config,
            &context_files,
            &effective_read_ns,
            observer,
            memory.clone(),
            git_coordinator.clone(),
            single_validation.clone(),
            None,
            &analysis,
            Arc::new(None),
        );

        invalidate_stale_sources(&memory, &unit, &config.workspace_root).await;

        let manifest = run_single_agent(
            &unit,
            None,
            &agent_ctx,
            &tier_router,
            &plan.iteration_config,
            make_observer(&unit.id),
        )
        .await?;
        let agent_completed = matches!(manifest.status, AgentStatus::Completed);
        observer.on_agent_state_changed(
            &manifest.work_unit_id,
            &manifest.status,
            manifest.summary.as_deref().unwrap_or(""),
        );

        if agent_completed {
            let effective_write_ns = effective_write_namespace(&unit, config);
            store_agent_result(
                &memory,
                &memory_writer,
                effective_write_ns,
                &manifest,
                &unit,
                &run_id,
                &config.workspace_root,
                config.extract_agent_findings,
            )
            .await;
        }
        exec_state.record_result(&unit.id, manifest.clone());
        let _ = exec_state.save(&config.workspace_root, &plan_hash);

        let verification_passed = run_post_execution_verification(
            &plan.verification_config,
            std::slice::from_ref(&manifest),
            &config.workspace_root,
            &config.excludes,
            observer,
        )
        .await?;

        let swarm_result = SwarmResult {
            manifests: vec![manifest],
            merge_results: vec![],
            success: agent_completed && verification_passed,
            pre_swarm_sha,
        };
        observer.on_phase_changed("completed");
        observer.on_completed(&swarm_result);
        return Ok(swarm_result);
    }

    // 2. Compute dependency tiers
    let tiers = validation::dependency_tiers(&work_units)
        .map_err(|e| anyhow::anyhow!("dependency cycle: {}", e))?;

    // Build lookup map
    let unit_map: std::collections::HashMap<&str, &WorkUnit> =
        work_units.iter().map(|u| (u.id.as_str(), u)).collect();

    let mut all_manifests: Vec<AgentManifest> = Vec::new();
    let mut all_merges: Vec<MergeResult> = Vec::new();
    let semaphore = Arc::new(Semaphore::new(effective_max_parallel));

    // Build validation pipeline based on workspace type (shared across all agents via Arc)
    let validation_pipeline: Option<Arc<crate::validation_gate::ValidationPipeline>> =
        if config.execution_mode == ExecutionMode::Document {
            None
        } else if config.workspace_root.join("Cargo.toml").exists() {
            Some(Arc::new(
                crate::validation_gate::ValidationPipeline::default_for_rust(),
            ))
        } else {
            Some(Arc::new(
                crate::validation_gate::ValidationPipeline::fast_only(),
            ))
        };

    // Build repo map + per-unit impact texts once for the whole swarm (repo mode only).
    let units_for_graph: Vec<WorkUnit> = work_units
        .iter()
        .chain(plan.loop_judge_units.iter())
        .cloned()
        .collect();
    let analysis = if repo_execution {
        tracing::info!("repo_map: scanning workspace");
        tracing::info!("code graph: indexing workspace");
        WorkspaceAnalysis::build(config, &units_for_graph).await
    } else {
        tracing::info!("execution document: skipping repo-map and code graph");
        WorkspaceAnalysis::empty()
    };
    let repo_map = analysis.repo_map.clone();
    let impact_texts = analysis.impact_texts.clone();

    tracing::info!("memory bundle: querying");
    // M7: Build SwarmContextBundle — one shared memory query for all work units.
    //
    // The coordinator already issues one DB query (coordinator.plan).  This
    // second query covers all units' topics so each runner receives
    // pre-fetched candidates and issues zero additional DB ops.
    // Total for N-unit swarm: coordinator(1) + bundle(1) = 2 ≤ M7 gate.
    //
    // Architectural intent: concatenate all work-unit descriptions so the
    // query captures the full swarm scope.
    let swarm_intent: String = work_units
        .iter()
        .chain(plan.loop_judge_units.iter())
        .map(|u| u.description.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let bundle = build_bundle(
        &swarm_intent,
        memory.as_ref(),
        &config.workspace_root,
        &config.read_namespaces,
        10,
    )
    .await;
    let pre_fetched_memory: Arc<Option<String>> = Arc::new(bundle.memory_text_for_prompt());

    // Inter-agent communication bus (available for future coordination)
    let bus = Arc::new(tokio::sync::Mutex::new(AgentBus::new()));
    // Register all agents upfront so they can send messages to each other
    {
        let mut b = bus.lock().await;
        for unit in &work_units {
            b.register(&unit.id);
        }
        for unit in &plan.loop_judge_units {
            b.register(&unit.id);
        }
    }

    // Shared discovery / artifact blackboard
    let shared_board = Arc::new(SharedBoard::new());
    shared_board
        .bind_run(&config.workspace_root, &plan.hash())
        .await;

    // Optional worktree manager (see `use_worktrees` computed above).
    let mut worktree_mgr = if use_worktrees {
        Some(WorktreeManager::new(config.workspace_root.clone()))
    } else {
        if want_worktrees {
            tracing::warn!(
                "parallel swarm requested worktree isolation but it is unavailable; \
                 running agents in the shared workspace"
            );
        }
        None
    };

    // ── Artefact-based loop resume ────────────────────────────────────────────
    // A refine loop's only durable record is the versioned files it writes
    // under OUT_DIR — the checkpoint in `exec_state` tracks node completion,
    // not iteration progress. When those artefacts show a fully completed
    // round, advance the loop's `iter_start` past it so the panel continues
    // instead of overwriting its own earlier work. `max_iterations` keeps its
    // meaning: a budget of rounds for *this* run, counted from the new start.
    let mut effective_loop_configs: Vec<crate::swarm::plan::LoopConfig> = plan.loop_configs.clone();
    if config.resume_from_artifacts {
        for lc in effective_loop_configs.iter_mut() {
            let Some(resume) = loop_resume::detect(&config.workspace_root, lc, &unit_map) else {
                continue;
            };
            tracing::info!("loop resume: {}", resume.summary());
            for note in &resume.notes {
                tracing::debug!("loop resume: {}", note);
            }
            // Baseline (`<id>-init`) units already have their output on disk;
            // marking them Completed keeps the tier dispatch from rewriting it.
            for init_id in &resume.satisfied_init_units {
                exec_state.set_status(init_id, NodeStatus::Completed);
            }
            lc.iter_start = resume.resume_iter_start;
            observer.on_loop_resumed(&resume);
        }
    }

    observer.on_phase_changed("running");

    // Build a map from loop-agent id → iter_start for first-pass {{ITER}} substitution.
    // Agents that appear in a loop block get {{ITER}}/{{PREV_ITER}} substituted before
    // every dispatch (first pass uses iter_start; subsequent passes increment).
    let loop_agent_first_iter: std::collections::HashMap<String, u32> = effective_loop_configs
        .iter()
        .flat_map(|lc| {
            lc.agent_ids
                .iter()
                .map(move |id| (id.clone(), lc.iter_start))
        })
        .collect();

    // Agents that participate in a stacked-mode loop are dispatched
    // entirely inside the loop block (so iteration 1 also runs with the
    // per-iteration branch + chain anchor). The tier dispatch below
    // skips them.
    let stacked_loop_agents: std::collections::HashSet<String> = effective_loop_configs
        .iter()
        .filter(|lc| {
            matches!(
                lc.branch_chain,
                crate::swarm::plan::BranchChainMode::Stacked
            )
        })
        .flat_map(|lc| lc.agent_ids.iter().cloned())
        .collect();

    // Loop body units (across all loop configs, regardless of branch_chain
    // mode) plus their transitive descendants in the depends_on graph.
    // These post-loop units are deferred from the first tier-dispatch pass
    // and re-dispatched in tier order AFTER the explicit-loops block has
    // run. Without this, a unit like `test_audit depends_on [execute_module]`
    // dispatches as soon as its tier is reached — which is BEFORE
    // execute_module's loop iterations 2..N actually run, so test_audit sees
    // an empty workspace state. The deferral makes "depends on a loop body
    // agent" mean "depends on the loop body having fully iterated".
    let loop_body_agents: std::collections::HashSet<String> = effective_loop_configs
        .iter()
        .flat_map(|lc| lc.agent_ids.iter().cloned())
        .collect();
    let post_loop_units: std::collections::HashSet<String> = {
        let mut deps_of: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for u in &work_units {
            for d in &u.depends_on {
                deps_of.entry(d.as_str()).or_default().push(u.id.as_str());
            }
        }
        let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut queue: Vec<&str> = loop_body_agents.iter().map(String::as_str).collect();
        while let Some(id) = queue.pop() {
            if let Some(children) = deps_of.get(id) {
                for &c in children {
                    if out.insert(c.to_string()) {
                        queue.push(c);
                    }
                }
            }
        }
        out
    };

    let loop_judge_map: std::collections::HashMap<&str, &WorkUnit> = plan
        .loop_judge_units
        .iter()
        .map(|u| (u.id.as_str(), u))
        .collect();

    // Baseline for the "every reviewer delivered" gate (see
    // `assert_loop_agents_produced_output`). The loop body's first pass runs
    // in the tier dispatch below rather than in the explicit-loops block, so
    // its `before` snapshot has to be taken here, ahead of any agent.
    let mut loop_output_baseline: std::collections::HashMap<String, OwnedSnapshot> =
        std::collections::HashMap::new();
    for lc in &effective_loop_configs {
        loop_output_baseline.extend(snapshot_loop_agents(
            &lc.agent_ids,
            &unit_map,
            &config.workspace_root,
        ));
    }

    let deadline = RunDeadline::new(config.run_timeout_secs);

    // 3. Execute tiers
    for (tier_idx, tier) in tiers.iter().enumerate() {
        if let Some(elapsed) = deadline.expired() {
            tracing::error!(
                "Run budget of {}s exhausted after {}s — stopping before tier {}/{}",
                config.run_timeout_secs,
                elapsed,
                tier_idx + 1,
                tiers.len()
            );
            observer.on_phase_changed("run budget exhausted");
            break;
        }
        observer.on_tier_started(tier_idx + 1, tiers.len());

        if effective_max_parallel <= 1 || tier.len() <= 1 {
            // Sequential execution
            for unit_id in tier {
                // Skip if already completed (resume support)
                if exec_state.status(unit_id) == NodeStatus::Completed {
                    tracing::info!("Skipping completed node '{}' (resume)", unit_id);
                    continue;
                }
                // Stacked-mode loop body agents: tier dispatch hands them
                // off to the loop block so all iterations (including #1)
                // run with the per-iteration branch + chain anchor.
                // Post-loop units (transitively depend on a loop body agent)
                // are deferred to the post-loop dispatch phase that runs
                // AFTER the explicit-loops block.
                if stacked_loop_agents.contains(unit_id) || post_loop_units.contains(unit_id) {
                    continue;
                }

                let unit = unit_map
                    .get(unit_id.as_str())
                    .with_context(|| format!("work unit '{}' not found", unit_id))?;

                // Apply {{ITER}}/{{PREV_ITER}} for first pass of loop agents
                let _iter_unit_seq: Option<WorkUnit>;
                let unit: &WorkUnit = if let Some(&is) = loop_agent_first_iter.get(unit_id.as_str())
                {
                    _iter_unit_seq = Some(apply_iter_vars(unit, is));
                    _iter_unit_seq.as_ref().unwrap()
                } else {
                    _iter_unit_seq = None;
                    unit
                };

                exec_state.set_status(unit_id, NodeStatus::Running);
                observer.on_agent_state_changed(unit_id, &AgentStatus::Running, &unit.description);

                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns = effective_read_namespaces(unit, config, &memory);

                let agent_ctx = AgentRunContext::for_run(
                    config,
                    &context_files,
                    &effective_read_ns,
                    observer,
                    memory.clone(),
                    git_coordinator.clone(),
                    validation_pipeline.clone(),
                    Some(shared_board.clone()),
                    &analysis,
                    pre_fetched_memory.clone(),
                );
                let manifest = run_single_agent(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    &tier_router,
                    &plan.iteration_config,
                    make_observer(unit_id),
                )
                .await?;

                let failed = matches!(manifest.status, AgentStatus::Failed(_));
                // Broadcast completion to bus so later tiers can see results
                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    // Store result to memory
                    let effective_write_ns = effective_write_namespace(&unit, config);
                    store_agent_result(
                        &memory,
                        &memory_writer,
                        effective_write_ns,
                        &manifest,
                        unit,
                        &run_id,
                        &config.workspace_root,
                        config.extract_agent_findings,
                    )
                    .await;
                }
                // Record result in execution state and checkpoint
                exec_state.record_result(unit_id, manifest.clone());
                if let Err(e) = exec_state.save(&config.workspace_root, &plan_hash) {
                    tracing::warn!("Failed to save execution state checkpoint: {}", e);
                }
                all_manifests.push(manifest);
                if failed {
                    break;
                }

                // Runtime fan-out: materialize + run SpawnManifest workers
                if !failed {
                    if let Err(e) = run_fanout_wave_if_needed(
                        unit_id,
                        plan,
                        config,
                        &context_files,
                        &memory,
                        &memory_writer,
                        observer,
                        git_coordinator.clone(),
                        validation_pipeline.clone(),
                        shared_board.clone(),
                        &analysis,
                        pre_fetched_memory.clone(),
                        &tier_router,
                        &plan.iteration_config,
                        &bus,
                        &mut all_manifests,
                        &mut exec_state,
                        &run_id,
                        &plan_hash,
                        worktree_mgr.as_mut(),
                        &make_observer,
                    )
                    .await
                    {
                        tracing::warn!("fan-out after '{unit_id}' failed: {e:#}");
                    }
                }
            }
        } else {
            // Parallel execution within tier
            let mut handles = Vec::new();

            // Register all agents as Pending before spawning. Skip stacked
            // loop body agents (handled by the loop block) and post-loop
            // units (deferred to the post-loop dispatch phase).
            for unit_id in tier {
                if stacked_loop_agents.contains(unit_id) || post_loop_units.contains(unit_id) {
                    continue;
                }
                observer.on_agent_state_changed(unit_id, &AgentStatus::Pending, "queued");
            }

            for unit_id in tier {
                if stacked_loop_agents.contains(unit_id) || post_loop_units.contains(unit_id) {
                    continue;
                }
                let unit = (*unit_map
                    .get(unit_id.as_str())
                    .with_context(|| format!("work unit '{}' not found", unit_id))?)
                .clone();

                // Apply {{ITER}}/{{PREV_ITER}} for first pass of loop agents
                let unit = if let Some(&is) = loop_agent_first_iter.get(unit_id.as_str()) {
                    apply_iter_vars(&unit, is)
                } else {
                    unit
                };

                let sem = semaphore.clone();
                let root = config.workspace_root.clone();
                let mem = memory.clone();
                let ns: Vec<String> = unit
                    .read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();
                let obs = make_observer(unit_id);
                let git_coord = git_coordinator.clone();
                let val_pipeline = validation_pipeline.clone();
                let board_ref = Some(shared_board.clone());
                let rm = repo_map.clone();
                let agent_impact = impact_texts.get(unit_id).cloned();
                let router = tier_router.clone();
                let iteration_config = plan.iteration_config.clone();
                let pfm = pre_fetched_memory.clone();
                let swarm_extras = config.swarm_extra_tools.clone();
                let skip_repo_context = config.execution_mode == ExecutionMode::Document;
                if let Ok(backend) = resolve_backend_for_unit(&router, &unit) {
                    observer.on_tier_dispatch(unit_id, unit.tier, backend.name());
                }

                // Provision worktree if enabled
                let in_worktree = worktree_mgr.is_some();
                let agent_root = if let Some(ref mut mgr) = worktree_mgr {
                    let handle = mgr.provision(&unit.id)?;
                    handle.path.clone()
                } else {
                    root.clone()
                };

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    invalidate_stale_sources(&mem, &unit, &root).await;

                    let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
                        WriteMode::AutoAccept,
                        Box::new(NoopWriteGateObserver),
                    )));
                    let engine = crate::iteration::IterationEngine::new(iteration_config.clone());
                    let mut manifest = engine
                        .run_with_backend_factory(
                            unit.clone(),
                            write_gate,
                            &agent_root,
                            mem.as_ref(),
                            &ns,
                            obs.as_ref(),
                            val_pipeline.as_deref(),
                            board_ref.as_deref(),
                            (*rm).as_ref(),
                            agent_impact.as_deref(),
                            (*pfm).as_deref(),
                            &swarm_extras,
                            skip_repo_context,
                            |candidate| resolve_backend_for_unit(&router, candidate),
                        )
                        .await
                        .manifest;

                    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
                        let summary = manifest
                            .summary
                            .as_deref()
                            .unwrap_or("task complete")
                            .to_string();
                        let agent_root_c = agent_root.clone();
                        let unit_id_c = unit.id.clone();
                        let owned_c = unit.scope.owned_paths.clone();
                        let changed = git_coord
                            .lock_git(move || {
                                commit_agent_changes(&agent_root_c, &unit_id_c, &summary, &owned_c)
                            })
                            .await
                            .unwrap_or_else(|e| {
                                tracing::warn!(
                                    "Failed to commit worktree changes for {}: {}",
                                    unit.id,
                                    e
                                );
                                vec![]
                            });
                        manifest.modified_files = changed;
                        manifest.branch = Some(format!("gaviero/{}", unit.id));
                    }

                    Ok::<_, anyhow::Error>(manifest)
                }));
            }

            // Collect results
            for (handle_idx, handle) in handles.into_iter().enumerate() {
                match handle.await {
                    Ok(Ok(manifest)) => {
                        observer.on_agent_state_changed(
                            &manifest.work_unit_id,
                            &manifest.status,
                            manifest.summary.as_deref().unwrap_or(""),
                        );
                        if matches!(manifest.status, AgentStatus::Completed) {
                            let b = bus.lock().await;
                            b.broadcast(
                                &manifest.work_unit_id,
                                &format!(
                                    "completed: {}",
                                    manifest.summary.as_deref().unwrap_or("")
                                ),
                            );
                            // Store result to memory
                            if let Some(unit) = unit_map.get(manifest.work_unit_id.as_str()) {
                                let effective_write_ns = effective_write_namespace(&unit, config);
                                store_agent_result(
                                    &memory,
                                    &memory_writer,
                                    effective_write_ns,
                                    &manifest,
                                    unit,
                                    &run_id,
                                    &config.workspace_root,
                                    config.extract_agent_findings,
                                )
                                .await;
                            }
                        }
                        all_manifests.push(manifest);
                    }
                    Ok(Err(e)) => {
                        let err_msg = format!("{:#}", e);
                        tracing::error!("Agent task error: {}", err_msg);
                        if let Some(unit_id) = tier.get(handle_idx) {
                            observer.on_agent_state_changed(
                                unit_id,
                                &AgentStatus::Failed(err_msg.clone()),
                                &err_msg,
                            );
                            all_manifests.push(AgentManifest {
                                work_unit_id: unit_id.clone(),
                                status: AgentStatus::Failed(err_msg),
                                modified_files: vec![],
                                branch: None,
                                summary: Some("Agent task error".into()),
                                output: None,
                                cost_usd: 0.0,
                            });
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("task panicked: {}", e);
                        tracing::error!("{}", err_msg);
                        if let Some(unit_id) = tier.get(handle_idx) {
                            observer.on_agent_state_changed(
                                unit_id,
                                &AgentStatus::Failed(err_msg.clone()),
                                &err_msg,
                            );
                            all_manifests.push(AgentManifest {
                                work_unit_id: unit_id.clone(),
                                status: AgentStatus::Failed(err_msg),
                                modified_files: vec![],
                                branch: None,
                                summary: Some("Agent task panicked".into()),
                                output: None,
                                cost_usd: 0.0,
                            });
                        }
                    }
                }
            }
        }
    }

    // 3b. Execute explicit loops (re-run loop agents until condition met)
    for loop_config in &effective_loop_configs {
        // First iteration was already executed in the tier loop above.
        // Now check the condition and re-iterate if needed.
        //
        // `stability` requires K consecutive PASS verdicts before exiting.
        // The counter resets on FAIL; it is only incremented for Agent judges
        // (verify/command conditions are boolean-per-iteration so stability
        // still composes correctly — a true result counts as a PASS).
        let mut loop_terminated = false;
        let stability_target = loop_config.stability.max(1);
        let mut consecutive_pass: u32 = 0;
        // Repeat detection for `irreconcilable_after`: the fingerprint of the
        // last FAIL's blocking disagreement and how many times running it has
        // been reported.
        let mut last_blocker_fingerprint: Option<String> = None;
        let mut repeat_streak: u32 = 0;
        // Skips a deterministic gate whose answer cannot have changed.
        // Lives across iterations; only the loop owns it.
        let mut probe_dedup = ProbeDedup::default();
        // Consecutive passes in which no loop agent delivered anything.
        let mut silent_streak: u32 = 0;

        // Stacked-mode chain anchor: the SHA each next agent's worktree
        // is based on. Starts at the pre-swarm HEAD; advances to each
        // committed branch's tip as iterations progress. Only consulted
        // when `loop_config.branch_chain == Stacked`.
        let stacked = matches!(
            loop_config.branch_chain,
            crate::swarm::plan::BranchChainMode::Stacked
        );
        // Iteration 1's chain anchor must include any commits produced by
        // the loop body's transitive non-stacked predecessors (e.g. a
        // pre-loop `inventory` agent whose output the body's prompt expects
        // to read). Those agents commit to their own `gaviero/{id}` branches
        // and the merge phase doesn't run until workflow exit, so workspace
        // HEAD does NOT contain their outputs. Anchoring iter 1 at workspace
        // HEAD therefore made every body iteration start blind to its own
        // prerequisites — the silent failure mode that produced the all-
        // HALTED 24-iteration run.
        let mut chain_anchor: Option<String> = if stacked {
            let body_set: std::collections::HashSet<&str> =
                loop_config.agent_ids.iter().map(String::as_str).collect();
            // Walk depends_on transitively from each body agent, collecting
            // dep ids that are NOT themselves loop body agents.
            let mut visited: std::collections::HashSet<String> = Default::default();
            let mut queue: Vec<String> = loop_config.agent_ids.iter().cloned().collect();
            while let Some(id) = queue.pop() {
                if let Some(unit) = unit_map.get(id.as_str()) {
                    for d in &unit.depends_on {
                        if body_set.contains(d.as_str()) {
                            continue;
                        }
                        if visited.insert(d.clone()) {
                            queue.push(d.clone());
                        }
                    }
                }
            }
            // Pull each predecessor's committed branch from its manifest.
            let dep_branches: Vec<String> = all_manifests
                .iter()
                .filter(|m| visited.contains(&m.work_unit_id))
                .filter(|m| matches!(m.status, AgentStatus::Completed))
                .filter_map(|m| m.branch.clone())
                .collect();
            match dep_branches.len() {
                0 => {
                    // No pre-loop deps — anchor at workspace HEAD.
                    worktree_mgr.as_ref().and_then(|mgr| mgr.head_commit().ok())
                }
                1 => {
                    // Single predecessor — chain off its committed tip so
                    // iter 1 sees its outputs.
                    let branch = &dep_branches[0];
                    worktree_mgr.as_ref().and_then(|mgr| mgr.branch_tip(branch))
                }
                n => {
                    // Multi-dep case: composing N parallel branches into a
                    // single base requires merge-conflict semantics that
                    // belong in the merge phase, not here. Bail with a
                    // pointer to the consolidation pattern.
                    anyhow::bail!(
                        "loop with branch_chain=stacked has {} non-stacked predecessor agents \
                         ({}); the runtime currently supports 0 or 1 pre-loop dependencies. \
                         Either consolidate the predecessors into one agent, or add a \
                         synthesizing dep agent that depends_on all of them and combines \
                         their outputs into a single branch.",
                        n,
                        dep_branches.join(", ")
                    );
                }
            }
        } else {
            None
        };
        if stacked && chain_anchor.is_none() {
            // Hard-fail rather than silently degrading: the previous
            // warn-and-fall-through path produced runs where iteration #1
            // was silently skipped and iterations 2..N ran via the
            // non-stacked legacy path, generating an off-by-one in
            // {{ITER}} (Module 1 unprocessed) and no per-iteration branches
            // in the repo's refs. Stacked is load-bearing for any workflow
            // whose iterations must see prior iterations' edits — failing
            // loudly forces the user to resolve the underlying issue
            // (worktrees disabled, repo has no commits, etc.) instead of
            // shipping a malformed deliverable.
            anyhow::bail!(
                "loop with branch_chain=stacked requires a resolvable workspace HEAD and \
                 usable git worktrees, but neither was available (worktrees enabled = {}). \
                 Ensure the workspace is a git repo with at least one commit and that \
                 worktrees are enabled in SwarmConfig.",
                use_worktrees
            );
        }

        // Stacked mode: iteration #1 runs HERE (the tier dispatch above
        // skipped these agents). Each body agent gets `gaviero/{id}-iter{N}`
        // chained off the running anchor; the anchor advances after every
        // committed agent so the next agent in this iteration (and the
        // first agent of iteration 2) chains off the latest tip.
        //
        // The `chain_anchor.is_some()` guard the previous version had here
        // is gone — the bail above guarantees Some when we reach this point.
        if stacked {
            tracing::info!(
                "Loop iteration 1/{} (stacked) for agents {:?}",
                loop_config.max_iterations,
                loop_config.agent_ids
            );
            observer.on_loop_iteration_started(
                1,
                loop_config.max_iterations,
                &loop_config.agent_ids,
            );
            observer.on_phase_changed("loop iteration 1");
            let iter_abs = loop_config.iter_start;
            for agent_id in &loop_config.agent_ids {
                let unit_template = match unit_map.get(agent_id.as_str()) {
                    Some(u) => u,
                    None => continue,
                };
                let iter_unit = apply_iter_vars(unit_template, iter_abs);
                let unit = &iter_unit;

                observer.on_agent_state_changed(agent_id, &AgentStatus::Running, &unit.description);
                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;
                let effective_read_ns = effective_read_namespaces(unit, config, &memory);
                let agent_ctx = AgentRunContext::for_run(
                    config,
                    &context_files,
                    &effective_read_ns,
                    observer,
                    memory.clone(),
                    git_coordinator.clone(),
                    validation_pipeline.clone(),
                    Some(shared_board.clone()),
                    &analysis,
                    pre_fetched_memory.clone(),
                );
                let branch = format!("gaviero/{}-iter{}", agent_id, iter_abs);
                let base_sha = chain_anchor.clone().unwrap();
                let manifest = run_single_agent_with_branch(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    &tier_router,
                    &plan.iteration_config,
                    make_observer(agent_id),
                    BranchOverride { branch, base_sha },
                )
                .await?;
                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    drop(b);
                    let effective_write_ns = effective_write_namespace(&unit, config);
                    store_agent_result(
                        &memory,
                        &memory_writer,
                        effective_write_ns,
                        &manifest,
                        unit,
                        &run_id,
                        &config.workspace_root,
                        config.extract_agent_findings,
                    )
                    .await;
                    if let Some(ref branch_name) = manifest.branch {
                        if let Some(ref mgr) = worktree_mgr {
                            if let Some(tip) = mgr.branch_tip(branch_name) {
                                chain_anchor = Some(tip);
                            }
                        }
                    }
                }
                exec_state.record_result(agent_id, manifest.clone());
                all_manifests.push(manifest);
            }
        }

        for iteration in 1..loop_config.max_iterations {
            let current_iter_abs = loop_config.iter_start + iteration - 1;

            if let Some(elapsed) = deadline.expired() {
                tracing::error!(
                    "Run budget of {}s exhausted after {}s — stopping the loop at iteration {}",
                    config.run_timeout_secs,
                    elapsed,
                    current_iter_abs
                );
                observer.on_phase_changed("run budget exhausted");
                loop_terminated = true;
                break;
            }

            // Every body agent has finished the pass being judged (the tier
            // dispatch for iteration 1, the barrier at the end of this loop
            // afterwards). A panel where every agent errored cannot be fixed
            // by another round, whatever the `until` condition is.
            assert_loop_pass_was_not_a_total_failure(
                &loop_config.agent_ids,
                &all_manifests,
                current_iter_abs,
            )?;

            // Then confirm each one actually delivered, before paying for a
            // judge turn.
            // Snapshot the pass now; the delivery gate itself runs only
            // if a judge is about to be dispatched (D-7).
            let delivery_after =
                snapshot_loop_agents(&loop_config.agent_ids, &unit_map, &config.workspace_root);

            // A panel that delivers nothing at all, pass after pass, is
            // stuck. Deterministic conditions never reach the delivery
            // gate, so without this they would iterate to the budget.
            {
                let silent = loop_agents_without_delivery(DeliveryCheck {
                    agent_ids: &loop_config.agent_ids,
                    unit_map: &unit_map,
                    all_manifests: &all_manifests,
                    before: &loop_output_baseline,
                    after: &delivery_after,
                    workspace_root: &config.workspace_root,
                    iter_abs: current_iter_abs,
                });
                let whole_panel_silent = !loop_config.agent_ids.is_empty()
                    && silent.len() == loop_config.agent_ids.len();
                silent_streak = if whole_panel_silent {
                    silent_streak.saturating_add(1)
                } else {
                    0
                };
                if silent_streak >= MAX_SILENT_LOOP_PASSES {
                    tracing::warn!(
                        "Loop stopped at iteration {}: no agent produced anything for {} consecutive passes ({})",
                        current_iter_abs,
                        silent_streak,
                        silent.join("; ")
                    );
                    observer.on_phase_changed("loop made no progress");
                    loop_terminated = true;
                    break;
                }
            }

            let outcome = {
                let mut loop_ctx = LoopConditionContext {
                    config,
                    context_files: &context_files,
                    memory: &memory,
                    memory_writer: &memory_writer,
                    observer,
                    git_coordinator: git_coordinator.clone(),
                    validation: validation_pipeline.clone(),
                    shared_board: shared_board.clone(),
                    repo_map: repo_map.clone(),
                    impact_texts: impact_texts.clone(),
                    pre_fetched_memory: pre_fetched_memory.clone(),
                    tier_router: &tier_router,
                    iteration_config: &plan.iteration_config,
                    loop_judge_map: &loop_judge_map,
                    bus: &bus,
                    all_manifests: &mut all_manifests,
                    run_id: &run_id,
                    make_observer: &make_observer,
                    strict_judge: loop_config.strict_judge,
                    judge_timeout_secs: loop_config.judge_timeout_secs,
                    loop_agent_ids: &loop_config.agent_ids,
                    consensus_mode: loop_config.consensus_mode,
                    verdict_output_dir: loop_config.verdict_output_dir.as_deref(),
                    probe_dedup: &mut probe_dedup,
                    delivery: DeliveryInputs {
                        unit_map: &unit_map,
                        before: &loop_output_baseline,
                        after: &delivery_after,
                    },
                };
                evaluate_loop_condition(&loop_config.until, current_iter_abs, &mut loop_ctx).await?
            };
            // Scoped to this pass: the agents dispatched below are the
            // ones that must react to it, so stale feedback can never
            // outlive the failure that produced it.
            let (outcome, last_gate_failure) = outcome;
            // Advance the baseline whether or not a judge ran: the
            // question each pass asks is "did this agent write during
            // *this* pass", so the window always moves.
            loop_output_baseline.extend(delivery_after);

            match outcome {
                LoopConditionOutcome::Partial => {
                    tracing::info!(
                        "Loop exited with PARTIAL consensus at iteration {} for agents {:?}",
                        iteration,
                        loop_config.agent_ids
                    );
                    loop_terminated = true;
                    break;
                }
                LoopConditionOutcome::Pass => {
                    consecutive_pass = consecutive_pass.saturating_add(1);
                    observer.on_loop_verdict(true, consecutive_pass, stability_target);
                    if consecutive_pass >= stability_target {
                        tracing::info!(
                            "Loop converged after {} iteration(s) with {}/{} consecutive PASS for agents {:?}",
                            iteration,
                            consecutive_pass,
                            stability_target,
                            loop_config.agent_ids
                        );
                        loop_terminated = true;
                        break;
                    }
                    tracing::info!(
                        "Loop got PASS {} / {} for agents {:?}; continuing for stability",
                        consecutive_pass,
                        stability_target,
                        loop_config.agent_ids
                    );
                }
                LoopConditionOutcome::Irreconcilable(report) => {
                    tracing::warn!(
                        "Loop stopped at iteration {}: judge ruled the disagreement irreconcilable for agents {:?}",
                        current_iter_abs,
                        loop_config.agent_ids
                    );
                    write_irreconcilable_report(
                        &config.workspace_root,
                        loop_config.verdict_output_dir.as_deref(),
                        current_iter_abs,
                        "judge verdict",
                        &report,
                        &loop_config.agent_ids,
                    );
                    loop_terminated = true;
                    break;
                }
                LoopConditionOutcome::Continue(report) => {
                    if consecutive_pass > 0 {
                        tracing::debug!(
                            "Loop PASS streak broken by FAIL at iteration {}, resetting counter",
                            iteration
                        );
                    }
                    consecutive_pass = 0;
                    observer.on_loop_verdict(false, 0, stability_target);

                    // Deterministic counterpart to `stability`: the same
                    // blocking disagreement, reported N times running, is a
                    // structural deadlock rather than a round away from
                    // resolution. Detected in the runtime so it holds even
                    // when the judge never reaches the `irreconcilable`
                    // verdict on its own.
                    //
                    // Only judge verdicts count. Under `until … and …` a
                    // deterministic condition can fail the iteration before
                    // the judge is dispatched; that pass carries no verdict,
                    // so it must neither extend a disagreement streak nor
                    // reset one that a later judge round would continue.
                    if loop_config.irreconcilable_after > 0 && last_gate_failure.is_none() {
                        let fingerprint = report.fingerprint();
                        if fingerprint.is_empty() {
                            repeat_streak = 0;
                        } else if Some(&fingerprint) == last_blocker_fingerprint.as_ref() {
                            repeat_streak = repeat_streak.saturating_add(1);
                        } else {
                            repeat_streak = 1;
                            last_blocker_fingerprint = Some(fingerprint);
                        }
                        if repeat_streak >= loop_config.irreconcilable_after {
                            tracing::warn!(
                                "Loop stopped at iteration {}: the same blocking disagreement was reported {} times running for agents {:?}",
                                current_iter_abs,
                                repeat_streak,
                                loop_config.agent_ids
                            );
                            write_irreconcilable_report(
                                &config.workspace_root,
                                loop_config.verdict_output_dir.as_deref(),
                                current_iter_abs,
                                &format!(
                                    "same disagreement reported {repeat_streak} times running"
                                ),
                                &report,
                                &loop_config.agent_ids,
                            );
                            loop_terminated = true;
                            break;
                        }
                    }
                }
            }

            tracing::info!(
                "Loop iteration {}/{} for agents {:?}",
                iteration + 1,
                loop_config.max_iterations,
                loop_config.agent_ids
            );
            observer.on_loop_iteration_started(
                iteration + 1,
                loop_config.max_iterations,
                &loop_config.agent_ids,
            );
            observer.on_phase_changed(&format!("loop iteration {}", iteration + 1));

            // Substitute {{ITER}} / {{PREV_ITER}} for this loop pass.
            // iteration is 1-indexed here (1..max_iterations); iter_abs = iter_start + iteration.
            let iter_abs = loop_config.iter_start + iteration as u32;
            let run_loop_parallel = effective_max_parallel > 1 && loop_config.agent_ids.len() > 1;

            if run_loop_parallel {
                // Barrier: fan out all loop-body agents concurrently (up to
                // max_parallel), then wait for every agent before judge / next iter.
                for agent_id in &loop_config.agent_ids {
                    observer.on_agent_state_changed(agent_id, &AgentStatus::Pending, "queued");
                }

                let mut prepared: Vec<(String, WorkUnit, PathBuf, Option<BranchOverride>)> =
                    Vec::new();
                for agent_id in &loop_config.agent_ids {
                    let unit_template = match unit_map.get(agent_id.as_str()) {
                        Some(u) => u,
                        None => continue,
                    };
                    let unit = apply_iter_vars_with_gate_feedback(
                        unit_template,
                        iter_abs,
                        last_gate_failure.as_ref(),
                    );
                    invalidate_stale_sources(&memory, &unit, &config.workspace_root).await;

                    let branch_override = if stacked && chain_anchor.is_some() {
                        Some(BranchOverride {
                            branch: format!("gaviero/{}-iter{}", agent_id, iter_abs),
                            base_sha: chain_anchor.clone().unwrap(),
                        })
                    } else {
                        None
                    };

                    let agent_root = if let Some(ref mut mgr) = worktree_mgr {
                        let handle = if let Some(ref ov) = branch_override {
                            mgr.provision_with_base(&unit.id, &ov.branch, &ov.base_sha)?
                        } else {
                            mgr.provision(&unit.id)?
                        };
                        if !context_files.is_empty() {
                            if let Err(e) = mgr.inject_context_files(&unit.id, &context_files) {
                                tracing::warn!(
                                    "Failed to inject context files for {}: {}",
                                    unit.id,
                                    e
                                );
                            }
                        }
                        handle.path.clone()
                    } else {
                        config.workspace_root.clone()
                    };
                    prepared.push((agent_id.clone(), unit, agent_root, branch_override));
                }

                let mut handles: Vec<(
                    String,
                    WorkUnit,
                    tokio::task::JoinHandle<anyhow::Result<AgentManifest>>,
                )> = Vec::new();
                for (agent_id, unit, agent_root, branch_override) in prepared {
                    let sem = semaphore.clone();
                    let mem = memory.clone();
                    let ns: Vec<String> = unit
                        .read_namespaces
                        .as_deref()
                        .unwrap_or(config.read_namespaces.as_slice())
                        .to_vec();
                    let obs = make_observer(&agent_id);
                    let git_coord = git_coordinator.clone();
                    let val_pipeline = validation_pipeline.clone();
                    let board_ref = Some(shared_board.clone());
                    let rm = repo_map.clone();
                    let agent_impact = impact_texts.get(&agent_id).cloned();
                    let router = tier_router.clone();
                    let iteration_config = plan.iteration_config.clone();
                    let pfm = pre_fetched_memory.clone();
                    let swarm_extras = config.swarm_extra_tools.clone();
                    let skip_repo_context = config.execution_mode == ExecutionMode::Document;
                    let in_worktree = worktree_mgr.is_some();
                    let override_branch_name = branch_override.as_ref().map(|ov| ov.branch.clone());

                    handles.push((
                        agent_id.clone(),
                        unit.clone(),
                        tokio::spawn(async move {
                            let _permit = sem.acquire().await.unwrap();
                            let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
                                WriteMode::AutoAccept,
                                Box::new(NoopWriteGateObserver),
                            )));
                            let engine =
                                crate::iteration::IterationEngine::new(iteration_config.clone());
                            let mut manifest = engine
                                .run_with_backend_factory(
                                    unit.clone(),
                                    write_gate,
                                    &agent_root,
                                    mem.as_ref(),
                                    &ns,
                                    obs.as_ref(),
                                    val_pipeline.as_deref(),
                                    board_ref.as_deref(),
                                    (*rm).as_ref(),
                                    agent_impact.as_deref(),
                                    (*pfm).as_deref(),
                                    &swarm_extras,
                                    skip_repo_context,
                                    |candidate| resolve_backend_for_unit(&router, candidate),
                                )
                                .await
                                .manifest;

                            if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
                                let summary = manifest
                                    .summary
                                    .as_deref()
                                    .unwrap_or("task complete")
                                    .to_string();
                                let agent_root_c = agent_root.clone();
                                let unit_id_c = unit.id.clone();
                                let owned_c = unit.scope.owned_paths.clone();
                                let changed = git_coord
                                    .lock_git(move || {
                                        commit_agent_changes(
                                            &agent_root_c,
                                            &unit_id_c,
                                            &summary,
                                            &owned_c,
                                        )
                                    })
                                    .await
                                    .unwrap_or_else(|e| {
                                        tracing::warn!(
                                            "Failed to commit worktree changes for {}: {}",
                                            unit.id,
                                            e
                                        );
                                        vec![]
                                    });
                                manifest.modified_files = changed;
                                manifest.branch = Some(
                                    override_branch_name
                                        .unwrap_or_else(|| format!("gaviero/{}", unit.id)),
                                );
                            }

                            Ok(manifest)
                        }),
                    ));
                }

                for (agent_id, unit, handle) in handles {
                    let manifest = match handle.await {
                        Ok(Ok(m)) => m,
                        Ok(Err(e)) => {
                            tracing::error!("Loop agent {} error: {:#}", agent_id, e);
                            AgentManifest {
                                work_unit_id: agent_id.clone(),
                                status: AgentStatus::Failed(format!("{e:#}")),
                                modified_files: vec![],
                                branch: None,
                                summary: Some("Agent task error".into()),
                                output: None,
                                cost_usd: 0.0,
                            }
                        }
                        Err(e) => {
                            tracing::error!("Loop agent {} panicked: {}", agent_id, e);
                            AgentManifest {
                                work_unit_id: agent_id.clone(),
                                status: AgentStatus::Failed(format!("task panicked: {e}")),
                                modified_files: vec![],
                                branch: None,
                                summary: Some("Agent task panicked".into()),
                                output: None,
                                cost_usd: 0.0,
                            }
                        }
                    };
                    observer.on_agent_state_changed(
                        &manifest.work_unit_id,
                        &manifest.status,
                        manifest.summary.as_deref().unwrap_or(""),
                    );
                    if matches!(manifest.status, AgentStatus::Completed) {
                        let b = bus.lock().await;
                        b.broadcast(
                            &manifest.work_unit_id,
                            &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                        );
                        drop(b);
                        let effective_write_ns = effective_write_namespace(&unit, config);
                        store_agent_result(
                            &memory,
                            &memory_writer,
                            effective_write_ns,
                            &manifest,
                            &unit,
                            &run_id,
                            &config.workspace_root,
                            config.extract_agent_findings,
                        )
                        .await;
                    }
                    exec_state.record_result(&agent_id, manifest.clone());
                    all_manifests.push(manifest);
                }
            } else {
                // Sequential fallback (max_parallel == 1 or a single loop agent).
                for agent_id in &loop_config.agent_ids {
                    let unit_template = match unit_map.get(agent_id.as_str()) {
                        Some(u) => u,
                        None => continue,
                    };

                    let iter_unit = apply_iter_vars_with_gate_feedback(
                        unit_template,
                        iter_abs,
                        last_gate_failure.as_ref(),
                    );
                    let unit = &iter_unit;

                    observer.on_agent_state_changed(
                        agent_id,
                        &AgentStatus::Running,
                        &unit.description,
                    );

                    invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                    let effective_read_ns = effective_read_namespaces(unit, config, &memory);

                    let agent_ctx = AgentRunContext::for_run(
                        config,
                        &context_files,
                        &effective_read_ns,
                        observer,
                        memory.clone(),
                        git_coordinator.clone(),
                        validation_pipeline.clone(),
                        Some(shared_board.clone()),
                        &analysis,
                        pre_fetched_memory.clone(),
                    );
                    let manifest = if stacked && chain_anchor.is_some() {
                        let branch = format!("gaviero/{}-iter{}", agent_id, iter_abs);
                        let base_sha = chain_anchor.clone().unwrap();
                        run_single_agent_with_branch(
                            unit,
                            worktree_mgr.as_mut(),
                            &agent_ctx,
                            &tier_router,
                            &plan.iteration_config,
                            make_observer(agent_id),
                            BranchOverride { branch, base_sha },
                        )
                        .await?
                    } else {
                        run_single_agent(
                            unit,
                            worktree_mgr.as_mut(),
                            &agent_ctx,
                            &tier_router,
                            &plan.iteration_config,
                            make_observer(agent_id),
                        )
                        .await?
                    };

                    if matches!(manifest.status, AgentStatus::Completed) {
                        let b = bus.lock().await;
                        b.broadcast(
                            &manifest.work_unit_id,
                            &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                        );
                        let effective_write_ns = effective_write_namespace(&unit, config);
                        store_agent_result(
                            &memory,
                            &memory_writer,
                            effective_write_ns,
                            &manifest,
                            unit,
                            &run_id,
                            &config.workspace_root,
                            config.extract_agent_findings,
                        )
                        .await;

                        if stacked {
                            if let Some(ref branch_name) = manifest.branch {
                                if let Some(ref mgr) = worktree_mgr {
                                    if let Some(tip) = mgr.branch_tip(branch_name) {
                                        chain_anchor = Some(tip);
                                    }
                                }
                            }
                        }
                    }
                    exec_state.record_result(agent_id, manifest.clone());
                    all_manifests.push(manifest);
                }
            }
        }

        // Final check after all iterations, but avoid re-running a judge after
        // the loop already terminated successfully.
        if !loop_terminated {
            let final_iter_abs =
                loop_config.iter_start + loop_config.max_iterations.saturating_sub(1);
            // Snapshot the pass now; the delivery gate itself runs only
            // if a judge is about to be dispatched (D-7).
            let delivery_after =
                snapshot_loop_agents(&loop_config.agent_ids, &unit_map, &config.workspace_root);
            let final_outcome = {
                let mut loop_ctx = LoopConditionContext {
                    config,
                    context_files: &context_files,
                    memory: &memory,
                    memory_writer: &memory_writer,
                    observer,
                    git_coordinator: git_coordinator.clone(),
                    validation: validation_pipeline.clone(),
                    shared_board: shared_board.clone(),
                    repo_map: repo_map.clone(),
                    impact_texts: impact_texts.clone(),
                    pre_fetched_memory: pre_fetched_memory.clone(),
                    tier_router: &tier_router,
                    iteration_config: &plan.iteration_config,
                    loop_judge_map: &loop_judge_map,
                    bus: &bus,
                    all_manifests: &mut all_manifests,
                    run_id: &run_id,
                    make_observer: &make_observer,
                    strict_judge: loop_config.strict_judge,
                    judge_timeout_secs: loop_config.judge_timeout_secs,
                    loop_agent_ids: &loop_config.agent_ids,
                    consensus_mode: loop_config.consensus_mode,
                    verdict_output_dir: loop_config.verdict_output_dir.as_deref(),
                    probe_dedup: &mut probe_dedup,
                    delivery: DeliveryInputs {
                        unit_map: &unit_map,
                        before: &loop_output_baseline,
                        after: &delivery_after,
                    },
                };
                evaluate_loop_condition(&loop_config.until, final_iter_abs, &mut loop_ctx).await?
            };
            // The run is over after this evaluation, so a gate failure
            // here has no next iteration to inform.
            let (final_outcome, _) = final_outcome;
            loop_output_baseline.extend(delivery_after);
            match final_outcome {
                LoopConditionOutcome::Pass => {
                    consecutive_pass = consecutive_pass.saturating_add(1);
                    if consecutive_pass < stability_target {
                        tracing::warn!(
                            "Loop exhausted max_iterations ({}) with final PASS but only {}/{} consecutive — convergence not confirmed for agents {:?}",
                            loop_config.max_iterations,
                            consecutive_pass,
                            stability_target,
                            loop_config.agent_ids
                        );
                    }
                }
                LoopConditionOutcome::Partial => {
                    tracing::info!(
                        "Loop exhausted max_iterations ({}) with final PARTIAL verdict for agents {:?}",
                        loop_config.max_iterations,
                        loop_config.agent_ids
                    );
                }
                LoopConditionOutcome::Irreconcilable(report) => {
                    tracing::warn!(
                        "Loop exhausted max_iterations ({}) and the final judge ruled the disagreement irreconcilable for agents {:?}",
                        loop_config.max_iterations,
                        loop_config.agent_ids
                    );
                    write_irreconcilable_report(
                        &config.workspace_root,
                        loop_config.verdict_output_dir.as_deref(),
                        final_iter_abs,
                        "judge verdict on the final check",
                        &report,
                        &loop_config.agent_ids,
                    );
                }
                LoopConditionOutcome::Continue(_) => {
                    tracing::warn!(
                        "Loop exhausted max_iterations ({}) without condition being met for agents {:?}",
                        loop_config.max_iterations,
                        loop_config.agent_ids
                    );
                }
            }
        }
    }

    // 3c. Post-loop tier dispatch.
    //
    // Run units that were deferred from the first tier-dispatch pass because
    // they transitively depend on a loop body agent. They walk the same tiers
    // in dependency order. Sequential-only — these units are always linked
    // by `depends_on` chains so parallel fan-out wouldn't help, and the
    // workflow-author intent (test_audit then final_verify) is sequential.
    if !post_loop_units.is_empty() {
        observer.on_phase_changed("post-loop");
        for tier in tiers.iter() {
            for unit_id in tier {
                if !post_loop_units.contains(unit_id) {
                    continue;
                }
                if exec_state.status(unit_id) == NodeStatus::Completed {
                    continue;
                }
                let unit = unit_map
                    .get(unit_id.as_str())
                    .with_context(|| format!("post-loop unit '{}' not found", unit_id))?;

                exec_state.set_status(unit_id, NodeStatus::Running);
                observer.on_agent_state_changed(unit_id, &AgentStatus::Running, &unit.description);
                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns = effective_read_namespaces(unit, config, &memory);
                let agent_ctx = AgentRunContext::for_run(
                    config,
                    &context_files,
                    &effective_read_ns,
                    observer,
                    memory.clone(),
                    git_coordinator.clone(),
                    validation_pipeline.clone(),
                    Some(shared_board.clone()),
                    &analysis,
                    pre_fetched_memory.clone(),
                );
                let manifest = run_single_agent(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    &tier_router,
                    &plan.iteration_config,
                    make_observer(unit_id),
                )
                .await?;

                let failed = matches!(manifest.status, AgentStatus::Failed(_));
                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    let effective_write_ns = effective_write_namespace(&unit, config);
                    store_agent_result(
                        &memory,
                        &memory_writer,
                        effective_write_ns,
                        &manifest,
                        unit,
                        &run_id,
                        &config.workspace_root,
                        config.extract_agent_findings,
                    )
                    .await;
                }
                exec_state.record_result(unit_id, manifest.clone());
                if let Err(e) = exec_state.save(&config.workspace_root, &plan_hash) {
                    tracing::warn!("Failed to save execution state checkpoint: {}", e);
                }
                all_manifests.push(manifest);
                if failed {
                    // A post-loop failure aborts further post-loop dispatch
                    // (e.g. final_verify shouldn't run if test_audit failed)
                    // but is otherwise non-fatal — merge/teardown still run.
                    break;
                }
            }
        }
    }

    // 4. Merge phase (only if using worktrees)
    if use_worktrees {
        observer.on_phase_changed("merging");

        // Stacked-loop iterations produce branches like `gaviero/{id}-iter{N}`
        // that form a deliberate chain in the repo's refs — they are the
        // deliverable, NOT something to merge back. Skip them here.
        let is_stacked_iter_branch = |branch: &str| -> bool {
            // Match `gaviero/<anything>-iter<digits>` shape.
            if let Some(rest) = branch.strip_prefix("gaviero/") {
                if let Some(idx) = rest.rfind("-iter") {
                    let suffix = &rest[idx + "-iter".len()..];
                    return !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit());
                }
            }
            false
        };
        for manifest in &all_manifests {
            if let Some(ref branch) = manifest.branch {
                if matches!(manifest.status, AgentStatus::Completed) {
                    if is_stacked_iter_branch(branch) {
                        tracing::debug!("skipping merge of stacked iteration branch '{}'", branch);
                        continue;
                    }
                    let mut result = merge::merge_branch(&config.workspace_root, branch)?;
                    if !result.success && !result.conflicts.is_empty() {
                        let files: Vec<String> = result
                            .conflicts
                            .iter()
                            .map(|c| c.file.to_string_lossy().to_string())
                            .collect();
                        observer.on_merge_conflict(branch, &files);

                        // Auto-resolve conflicts via Claude
                        observer.on_phase_changed("resolving conflicts");
                        let resolved = merge::auto_resolve_conflicts(
                            &config.workspace_root,
                            branch,
                            &result.conflicts,
                            &config.model,
                            config.ollama_base_url.as_deref(),
                        )
                        .await;

                        match resolved {
                            Ok(resolved_conflicts) => {
                                let all_ok = resolved_conflicts.iter().all(|c| c.resolved);
                                result.conflicts = resolved_conflicts;
                                result.success = all_ok;
                                if !all_ok {
                                    tracing::warn!(
                                        "some conflicts could not be auto-resolved for {}",
                                        branch
                                    );
                                    merge::abort_merge(&config.workspace_root)?;
                                }
                            }
                            Err(e) => {
                                tracing::error!("auto-resolve failed for {}: {}", branch, e);
                                merge::abort_merge(&config.workspace_root)?;
                            }
                        }
                    }
                    all_merges.push(result);
                }
            }
        }
    }

    // 5. Teardown worktrees
    if let Some(ref mut mgr) = worktree_mgr {
        mgr.teardown_all();
    }

    // 6. Post-execution memory consolidation (best-effort)
    if let Some(mem) = memory.as_ref() {
        // Reuse the pipeline's writer. A private consolidator writer
        // would run concurrently with the extractor writes this phase is
        // supposed to triage — racing it for the store connection and the
        // ONNX session — and the host's exit flush only covers this one.
        let consolidator = match memory_writer.as_ref() {
            Some(writer) => crate::memory::consolidation::Consolidator::with_stores_and_writer(
                Arc::clone(mem),
                writer.clone(),
            ),
            None => crate::memory::consolidation::Consolidator::with_stores(Arc::clone(mem)),
        };
        let repo_id = crate::memory::hash_path(&config.workspace_root);
        match consolidator.consolidate_run(&run_id, &repo_id).await {
            Ok(report) => {
                tracing::info!(
                    promoted = report.promoted,
                    reinforced = report.reinforced,
                    pruned = report.pruned,
                    "memory consolidation complete"
                );
            }
            Err(e) => {
                tracing::warn!("memory consolidation failed: {}", e);
            }
        }
    }

    let verification_passed = run_post_execution_verification(
        &plan.verification_config,
        &all_manifests,
        &config.workspace_root,
        &config.excludes,
        observer,
    )
    .await?;

    let success = all_manifests
        .iter()
        .all(|m| matches!(m.status, AgentStatus::Completed))
        && all_merges.iter().all(|m| m.success)
        && verification_passed;

    let result = SwarmResult {
        manifests: all_manifests,
        merge_results: all_merges,
        success,
        pre_swarm_sha,
    };

    observer.on_phase_changed("completed");
    observer.on_completed(&result);

    Ok(result)
}

/// Shared execution context for a single agent run.
///
/// Bundles the parameters that are constant across all agents in a swarm run,
/// reducing `run_single_agent` from 11 parameters to 4.
struct AgentRunContext<'a> {
    workspace_root: &'a PathBuf,
    context_files: &'a [(String, String)],
    memory: Option<Arc<MemoryStores>>,
    read_namespaces: &'a [String],
    swarm_observer: &'a dyn SwarmObserver,
    git_coordinator: Arc<GitCoordinator>,
    validation: Option<Arc<crate::validation_gate::ValidationPipeline>>,
    board: Option<Arc<SharedBoard>>,
    repo_map: Arc<Option<crate::repo_map::RepoMap>>,
    /// Pre-computed impact analysis text per agent (from code knowledge graph).
    impact_texts: Arc<std::collections::HashMap<String, String>>,
    /// Shared memory text pre-fetched for all runners (M7 bundle query, 1 DB op).
    ///
    /// `Some(text)` → planner skips per-runner DB query; `None` → fallback to
    /// per-runner query (single-agent fast path does not pre-fetch).
    pre_fetched_memory: Arc<Option<String>>,
    mcp_config: Option<crate::mcp::McpConfigSynth>,
    /// Workspace-resolved extras for swarm tool grants (see
    /// `SwarmConfig::swarm_extra_tools`). Borrowed from `SwarmConfig`
    /// for the duration of the swarm run.
    swarm_extras: &'a [String],
    /// When true, omit repo-map, topology, and code-graph context from prompts.
    skip_repo_context: bool,
}

impl<'a> AgentRunContext<'a> {
    /// Build an `AgentRunContext` from the swarm's shared state. Single
    /// construction site for both the single-agent fast path (with `board =
    /// None`, `pre_fetched_memory = Arc::new(None)`) and the multi-agent /
    /// loop / readonly paths (which pass `Some(shared_board)` and the
    /// pre-fetched bundle text).
    #[allow(clippy::too_many_arguments)]
    fn for_run(
        config: &'a SwarmConfig,
        context_files: &'a [(String, String)],
        read_namespaces: &'a [String],
        observer: &'a dyn SwarmObserver,
        memory: Option<Arc<MemoryStores>>,
        git_coordinator: Arc<GitCoordinator>,
        validation: Option<Arc<crate::validation_gate::ValidationPipeline>>,
        board: Option<Arc<SharedBoard>>,
        analysis: &WorkspaceAnalysis,
        pre_fetched_memory: Arc<Option<String>>,
    ) -> Self {
        Self {
            workspace_root: &config.workspace_root,
            context_files,
            memory,
            read_namespaces,
            swarm_observer: observer,
            git_coordinator,
            validation,
            board,
            repo_map: analysis.repo_map.clone(),
            impact_texts: analysis.impact_texts.clone(),
            pre_fetched_memory,
            mcp_config: config.mcp_config.clone(),
            swarm_extras: &config.swarm_extra_tools,
            skip_repo_context: config.execution_mode == ExecutionMode::Document,
        }
    }
}

/// Repo map + per-unit impact texts derived from the code knowledge graph.
/// Computed once per swarm run; cloned cheaply (`Arc`) into every
/// `AgentRunContext`.
struct WorkspaceAnalysis {
    repo_map: Arc<Option<crate::repo_map::RepoMap>>,
    impact_texts: Arc<std::collections::HashMap<String, String>>,
}

impl WorkspaceAnalysis {
    fn empty() -> Self {
        Self {
            repo_map: Arc::new(None),
            impact_texts: Arc::new(std::collections::HashMap::new()),
        }
    }

    /// Build the repo map and per-unit impact texts. Both phases run on
    /// blocking threads to avoid starving the async executor; failures are
    /// logged at debug level and yield empty results so single-agent and
    /// multi-agent runs degrade identically.
    async fn build(config: &SwarmConfig, units: &[WorkUnit]) -> Self {
        let repo_map: Arc<Option<crate::repo_map::RepoMap>> = {
            let workspace = config.workspace_root.clone();
            let excludes = config.excludes.clone();
            let specificity = config.specificity;
            Arc::new(
                tokio::task::spawn_blocking(move || {
                    crate::repo_map::RepoMap::build_with_config(&workspace, &excludes, specificity)
                        .map_err(|e| {
                            tracing::debug!("repo_map build skipped: {}", e);
                            e
                        })
                        .ok()
                        .inspect(|_| tracing::info!("repo_map: done"))
                })
                .await
                .unwrap_or(None),
            )
        };

        let units_for_graph: Vec<WorkUnit> = units.to_vec();
        let impact_texts: Arc<std::collections::HashMap<String, String>> = {
            let workspace = config.workspace_root.clone();
            let excludes = config.excludes.clone();
            Arc::new(
                tokio::task::spawn_blocking(move || {
                    let mut map = std::collections::HashMap::new();
                    match crate::repo_map::graph_builder::build_graph(&workspace, &excludes) {
                        Ok((store, result)) => {
                            tracing::info!(
                                "code graph: {} nodes, {} edges ({} files changed, {} unchanged)",
                                result.total_nodes,
                                result.total_edges,
                                result.files_changed,
                                result.files_unchanged,
                            );
                            for wu in &units_for_graph {
                                let mut sections: Vec<String> = Vec::new();

                                let owned: Vec<&str> =
                                    wu.scope.owned_paths.iter().map(|s| s.as_str()).collect();
                                if !owned.is_empty() {
                                    let depth = if wu.impact_scope {
                                        wu.context_depth.max(3) as usize
                                    } else {
                                        3
                                    };
                                    if let Ok(impact) = store.impact_radius(&owned, depth) {
                                        if !impact.affected_files.is_empty() {
                                            sections.push(
                                                crate::repo_map::store::GraphStore::format_impact_for_prompt(
                                                    &impact,
                                                ),
                                            );
                                        }
                                    }
                                }

                                if !wu.context_callers_of.is_empty() {
                                    let refs: Vec<&str> = wu
                                        .context_callers_of
                                        .iter()
                                        .map(|s| s.as_str())
                                        .collect();
                                    if let Ok(impact) =
                                        store.impact_radius(&refs, wu.context_depth as usize)
                                    {
                                        let callers: Vec<&str> = impact
                                            .affected_files
                                            .iter()
                                            .filter(|f| !wu.context_callers_of.contains(f))
                                            .map(|s| s.as_str())
                                            .collect();
                                        if !callers.is_empty() {
                                            sections.push(format!(
                                                "[Callers of {:?}]:\n{}",
                                                wu.context_callers_of,
                                                callers.join(", ")
                                            ));
                                        }
                                    }
                                }

                                if !wu.context_tests_for.is_empty() {
                                    let refs: Vec<&str> = wu
                                        .context_tests_for
                                        .iter()
                                        .map(|s| s.as_str())
                                        .collect();
                                    if let Ok(impact) =
                                        store.impact_radius(&refs, wu.context_depth as usize)
                                    {
                                        if !impact.affected_tests.is_empty() {
                                            sections.push(format!(
                                                "[Tests for {:?}]:\n{}",
                                                wu.context_tests_for,
                                                impact.affected_tests.join(", ")
                                            ));
                                        }
                                    }
                                }

                                if !sections.is_empty() {
                                    map.insert(wu.id.clone(), sections.join("\n\n"));
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("code graph build skipped: {}", e);
                        }
                    }
                    tracing::info!("code graph: done");
                    map
                })
                .await
                .unwrap_or_default(),
            )
        };

        Self {
            repo_map,
            impact_texts,
        }
    }
}

/// Resolve the effective read-namespace list for a work unit: the unit's
/// own list if set, otherwise the swarm-config default. When memory is open
/// and both are empty, fall back to `shared` (dual-surface contract).
fn effective_read_namespaces(
    unit: &WorkUnit,
    config: &SwarmConfig,
    memory: &Option<Arc<MemoryStores>>,
) -> Vec<String> {
    let mut ns = unit
        .read_namespaces
        .as_deref()
        .unwrap_or(config.read_namespaces.as_slice())
        .to_vec();
    if memory.is_some() && ns.is_empty() {
        ns.push("shared".into());
    }
    ns
}

fn effective_write_namespace<'a>(unit: &'a WorkUnit, config: &'a SwarmConfig) -> &'a str {
    unit.write_namespace
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if config.write_namespace.is_empty() {
                "swarm"
            } else {
                config.write_namespace.as_str()
            }
        })
}

struct LoopConditionContext<'a> {
    config: &'a SwarmConfig,
    context_files: &'a [(String, String)],
    memory: &'a Option<Arc<MemoryStores>>,
    memory_writer: &'a Option<WriterHandle>,
    observer: &'a dyn SwarmObserver,
    git_coordinator: Arc<GitCoordinator>,
    validation: Option<Arc<crate::validation_gate::ValidationPipeline>>,
    shared_board: Arc<SharedBoard>,
    repo_map: Arc<Option<crate::repo_map::RepoMap>>,
    impact_texts: Arc<std::collections::HashMap<String, String>>,
    pre_fetched_memory: Arc<Option<String>>,
    tier_router: &'a TierRouter,
    iteration_config: &'a crate::iteration::IterationConfig,
    loop_judge_map: &'a std::collections::HashMap<&'a str, &'a WorkUnit>,
    bus: &'a Arc<tokio::sync::Mutex<AgentBus>>,
    all_manifests: &'a mut Vec<AgentManifest>,
    run_id: &'a str,
    make_observer: &'a (dyn Fn(&str) -> Box<dyn AcpObserver> + Send + Sync),
    /// When true, unparseable judge output on a completed run is promoted to
    /// `AgentStatus::Failed`. Wired from `LoopConfig.strict_judge`.
    strict_judge: bool,
    /// Hard timeout for each judge invocation in seconds. 0 disables.
    /// Wired from `LoopConfig.judge_timeout_secs`.
    judge_timeout_secs: u32,
    /// Loop worker agent ids, used to build `{{ITER_EVIDENCE}}` digests.
    loop_agent_ids: &'a [String],
    consensus_mode: crate::swarm::plan::ConsensusMode,
    verdict_output_dir: Option<&'a str>,
    /// Dedup state for deterministic gates, owned by the loop so it
    /// survives across iterations. Judges are never deduplicated.
    probe_dedup: &'a mut ProbeDedup,
    /// Inputs for the delivery gate, consulted only if a judge is about
    /// to be dispatched.
    delivery: DeliveryInputs<'a>,
}

/// What the delivery gate needs to decide whether the panel is intact.
///
/// Carried on the condition context rather than checked in the loop head
/// because the check belongs immediately before a judge runs — see
/// [`evaluate_agent_condition`].
struct DeliveryInputs<'a> {
    unit_map: &'a std::collections::HashMap<&'a str, &'a WorkUnit>,
    /// Owned-file snapshots bracketing the pass being judged.
    before: &'a std::collections::HashMap<String, OwnedSnapshot>,
    after: &'a std::collections::HashMap<String, OwnedSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JudgeVerdict {
    Pass,
    Fail,
    /// Substantive agreement not reached, but the panel should stop: the
    /// disagreement is structural, not a round away from resolving.
    Irreconcilable,
    Partial,
}

/// One reviewer's position in a disagreement the judge ruled irreconcilable.
///
/// The judge supplies the prose (it is the only party that has read every
/// conclusion); the runtime decides what to do with it and renders the
/// hand-off document. Every field is optional because a judge that gets the
/// shape half-right should still produce a usable report rather than a parse
/// error that discards the whole verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct JudgeBlocker {
    #[serde(default)]
    agent: String,
    /// What this reviewer holds, in its own terms.
    #[serde(default)]
    position: String,
    /// Which reviewer(s) it conflicts with.
    #[serde(default)]
    conflicts_with: String,
    /// Which convergence criterion the conflict falls under (framing, avenues,
    /// risks, evidence bar, stopping rules).
    #[serde(default)]
    criterion: String,
    /// What evidence would settle it — the actionable part for a human.
    #[serde(default)]
    evidence_gap: String,
}

/// A judge verdict plus the structured detail the runtime keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct JudgeReport {
    reason: String,
    blockers: Vec<JudgeBlocker>,
}

impl JudgeReport {
    /// Stable identity of the blocking disagreement, for repeat detection.
    ///
    /// Keyed on the blockers when the judge supplied them (agent + criterion +
    /// who it conflicts with) and on the normalized reason otherwise. Prose
    /// wording drifts between iterations even when the substance does not, so
    /// matching on the reason alone would under-detect; matching on the
    /// criterion pairing is what actually stays constant while a panel
    /// restates the same deadlock.
    fn fingerprint(&self) -> String {
        if self.blockers.is_empty() {
            return normalize_for_fingerprint(&self.reason);
        }
        let mut parts: Vec<String> = self
            .blockers
            .iter()
            .map(|b| {
                format!(
                    "{}|{}|{}",
                    normalize_for_fingerprint(&b.agent),
                    normalize_for_fingerprint(&b.criterion),
                    normalize_for_fingerprint(&b.conflicts_with),
                )
            })
            .collect();
        parts.sort();
        parts.join("\n")
    }
}

/// Lowercase, collapse whitespace, drop punctuation — so "avenue A vs B."
/// and "avenue a vs b" fingerprint identically.
fn normalize_for_fingerprint(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LoopConditionOutcome {
    /// Keep iterating (FAIL, unparseable, or not yet converged). Carries the
    /// judge's report so the loop can detect a disagreement that repeats.
    Continue(JudgeReport),
    Pass,
    /// Substantive agreement not reached but the run should stop (partial_ok).
    Partial,
    /// The judge ruled the disagreement structural. Stop and hand off.
    Irreconcilable(JudgeReport),
}

/// Turn a deterministic gate result into a loop outcome.
fn report_deterministic_gate(
    failure: Option<GateFailure>,
    iter_abs: u32,
    observer: &dyn SwarmObserver,
) -> (LoopConditionOutcome, Option<GateFailure>) {
    match failure {
        None => (LoopConditionOutcome::Pass, None),
        Some(failure) => {
            tracing::info!(
                "Loop gate failed at iteration {}: `{}` ({})",
                iter_abs,
                failure.probe,
                failure.status
            );
            observer.on_loop_gate_failed(&failure.probe, &failure.status, &failure.output);
            (
                LoopConditionOutcome::Continue(JudgeReport::default()),
                Some(failure),
            )
        }
    }
}

/// Clone `unit` and substitute `{{ITER}}` / `{{PREV_ITER}}` with `iter_abs`
/// and `iter_abs - 1` respectively. Called for every loop-agent dispatch.
fn apply_iter_vars(unit: &WorkUnit, iter_abs: u32) -> WorkUnit {
    apply_iter_vars_full(unit, iter_abs, "", None)
}

/// `apply_iter_vars` plus the previous iteration's gate failure, for
/// loop-body agents dispatched after a deterministic condition failed.
fn apply_iter_vars_with_gate_feedback(
    unit: &WorkUnit,
    iter_abs: u32,
    gate: Option<&GateFailure>,
) -> WorkUnit {
    apply_iter_vars_full(unit, iter_abs, "", gate)
}

/// Clone `unit` and substitute `{{ITER}}`, `{{PREV_ITER}}`, and
/// `{{ITER_EVIDENCE}}` in `coordinator_instructions`. Evidence is intended
/// for judge agents — it summarises the previous iteration's manifests and
/// modified files so the judge can decide on facts instead of hallucinating.
fn apply_iter_vars_with_evidence(unit: &WorkUnit, iter_abs: u32, evidence: &str) -> WorkUnit {
    apply_iter_vars_full(unit, iter_abs, evidence, None)
}

/// Substitute every iteration variable in one pass.
///
/// `{{GATE_FEEDBACK}}` is substituted wherever the author placed it. When
/// a gate failed but no template mentions the placeholder, the detail is
/// appended as a `## Previous gate failure` section instead — the
/// feedback has to reach the agent whether or not the script was written
/// to expect it. With no failure to report the placeholder collapses to
/// nothing, so a template that always references it stays clean on a
/// passing iteration.
fn apply_iter_vars_full(
    unit: &WorkUnit,
    iter_abs: u32,
    evidence: &str,
    gate: Option<&GateFailure>,
) -> WorkUnit {
    let prev = iter_abs.saturating_sub(1);
    let iter_str = iter_abs.to_string();
    let prev_str = prev.to_string();
    let feedback = gate.map(|g| g.render()).unwrap_or_default();
    let sub = |s: &str| {
        s.replace("{{ITER}}", &iter_str)
            .replace("{{PREV_ITER}}", &prev_str)
            .replace(GATE_FEEDBACK_PLACEHOLDER, &feedback)
    };

    let author_placed_feedback = unit.description.contains(GATE_FEEDBACK_PLACEHOLDER)
        || unit
            .coordinator_instructions
            .contains(GATE_FEEDBACK_PLACEHOLDER);

    let mut coordinator_instructions = unit
        .coordinator_instructions
        .replace("{{ITER}}", &iter_str)
        .replace("{{PREV_ITER}}", &prev_str)
        .replace("{{ITER_EVIDENCE}}", evidence)
        .replace(GATE_FEEDBACK_PLACEHOLDER, &feedback);

    if let Some(gate) = gate
        && !author_placed_feedback
    {
        coordinator_instructions.push_str("\n\n## Previous gate failure\n\n");
        coordinator_instructions.push_str(&gate.render());
    }

    WorkUnit {
        description: sub(&unit.description),
        // The output contract is versioned per pass, so it has to be
        // substituted alongside the prompt that tells the agent which version
        // to write — otherwise the gate would check iteration 1's paths for
        // every iteration.
        produces: unit
            .produces
            .iter()
            .map(|p| {
                p.replace("{{ITER}}", &iter_str)
                    .replace("{{PREV_ITER}}", &prev_str)
            })
            .collect(),
        coordinator_instructions,
        ..unit.clone()
    }
}

/// Fingerprints of the files an agent owns: workspace-relative path → (len, mtime).
///
/// Two snapshots taken around a loop pass tell us whether the agent actually
/// delivered anything, independently of how it writes (native tool calls,
/// in-band file blocks, or worktree commits).
type OwnedSnapshot = std::collections::BTreeMap<String, (u64, Option<std::time::SystemTime>)>;

/// Widen an `owned` glob so it covers every version an agent may write.
///
/// A workflow that versions artefacts per pass (`{{OUT_DIR}}/x-v{{ITER}}.md`)
/// leaves `{{ITER}}` in the glob when the snapshot is taken outside a
/// specific iteration; collapsing any surviving `{{…}}` placeholder to `*`
/// keeps the snapshot iteration-agnostic, which is what a
/// "did this agent write anything at all" check wants.
fn widen_owned_glob(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open..].find("}}") else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push('*');
        rest = &rest[open + close + 2..];
    }
    out.push_str(rest);
    out
}

/// Snapshot the files under `root` owned by each of `agents`, in one walk.
///
/// Depth is bounded and the usual build/VCS directories are pruned: owned
/// globs address source and artefact directories, and an unbounded walk of a
/// large checkout — repeated per agent, per iteration — would cost more than
/// the check saves.
fn snapshot_owned_files(
    agents: &[(&str, &WorkUnit)],
    root: &std::path::Path,
) -> std::collections::HashMap<String, OwnedSnapshot> {
    let mut snaps: std::collections::HashMap<String, OwnedSnapshot> = agents
        .iter()
        .map(|(id, _)| ((*id).to_string(), OwnedSnapshot::new()))
        .collect();
    let widened: Vec<(&str, Vec<String>)> = agents
        .iter()
        .map(|(id, u)| {
            (
                *id,
                u.scope
                    .owned_paths
                    .iter()
                    .map(|p| widen_owned_glob(p))
                    .collect(),
            )
        })
        .filter(|(_, pats): &(&str, Vec<String>)| !pats.is_empty())
        .collect();
    if widened.is_empty() {
        return snaps;
    }

    for entry in crate::swarm::workspace_snapshot::pruned_walk(root).flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let mut meta = None;
        for (id, patterns) in &widened {
            if !patterns
                .iter()
                .any(|p| crate::path_pattern::matches(p, &rel))
            {
                continue;
            }
            let meta = meta.get_or_insert_with(|| entry.metadata().ok());
            let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta.as_ref().and_then(|m| m.modified().ok());
            if let Some(snap) = snaps.get_mut(*id) {
                snap.insert(rel.clone(), (len, mtime));
            }
        }
    }
    snaps
}

/// Snapshot every loop-body agent's owned files.
fn snapshot_loop_agents(
    agent_ids: &[String],
    unit_map: &std::collections::HashMap<&str, &WorkUnit>,
    root: &std::path::Path,
) -> std::collections::HashMap<String, OwnedSnapshot> {
    let agents: Vec<(&str, &WorkUnit)> = agent_ids
        .iter()
        .filter_map(|id| unit_map.get(id.as_str()).map(|u| (id.as_str(), *u)))
        .collect();
    snapshot_owned_files(&agents, root)
}

/// Inputs to [`assert_loop_agents_produced_output`].
struct DeliveryCheck<'a> {
    agent_ids: &'a [String],
    /// Templates, with `{{ITER}}` still unsubstituted — the check applies
    /// `iter_abs` itself so a versioned contract names the right pass.
    unit_map: &'a std::collections::HashMap<&'a str, &'a WorkUnit>,
    all_manifests: &'a [AgentManifest],
    /// Owned-file snapshots bracketing the pass, for agents that declare no
    /// `produces` contract.
    before: &'a std::collections::HashMap<String, OwnedSnapshot>,
    after: &'a std::collections::HashMap<String, OwnedSnapshot>,
    workspace_root: &'a std::path::Path,
    /// The absolute iteration being judged.
    iter_abs: u32,
}

/// Abort the run when *every* agent in a loop pass hard-failed.
///
/// The delivery gate below never runs for `command` / `verify`
/// conditions: those measure the workspace rather than the panel, so an
/// iteration that produced nothing is an unproductive but legitimate
/// outcome. "Every agent errored" has no such reading. It is a broken
/// panel — a bad model spec, a missing CLI, an unreachable provider —
/// and re-dispatching it cannot succeed, so the loop would spend every
/// remaining iteration on it and still report success at the end.
///
/// Deliberately narrow. A *partially* failing panel can still make
/// progress, and the delivery gate plus the judge already reason about
/// that; only a unanimous failure is unambiguous enough to stop on.
fn assert_loop_pass_was_not_a_total_failure(
    agent_ids: &[String],
    all_manifests: &[AgentManifest],
    iter_abs: u32,
) -> Result<()> {
    if agent_ids.is_empty() {
        return Ok(());
    }

    let latest: Vec<Option<&AgentManifest>> = agent_ids
        .iter()
        .map(|id| all_manifests.iter().rev().find(|m| m.work_unit_id == *id))
        .collect();

    // An agent that never reported at all is the delivery gate's
    // business, not ours — it can distinguish "never dispatched" from
    // "ran and wrote nothing" and says so in its diagnostic.
    if latest.iter().any(|m| m.is_none()) {
        return Ok(());
    }

    let failures: Vec<String> = agent_ids
        .iter()
        .zip(&latest)
        .filter_map(|(id, m)| match m.map(|m| &m.status) {
            Some(AgentStatus::Failed(msg)) => Some(format!("'{id}': {msg}")),
            _ => None,
        })
        .collect();

    if failures.len() < agent_ids.len() {
        return Ok(());
    }

    bail!(
        "loop iteration {} failed for all {} of its agent(s): {}. \
         Stopping rather than re-dispatching a panel that cannot run — the loop \
         would otherwise spend every remaining iteration on it and still report \
         success. Check the model specs, provider CLIs, and endpoints for these agents.",
        iter_abs,
        agent_ids.len(),
        failures.join("; ")
    );
}

/// How many consecutive passes a loop may produce nothing at all before
/// it is treated as stuck.
///
/// The delivery gate exempts deterministic conditions because *one*
/// unproductive iteration is a legitimate outcome for a probe that
/// measures the workspace. That reasoning does not survive repetition: a
/// panel delivering nothing every pass will keep doing so, and without a
/// bound it spends the whole iteration budget proving it.
///
/// Mirrors `irreconcilable_after`'s default and its logic — a repeated
/// identical non-result is structural, not transient. Kept a constant
/// rather than a `LoopConfig` field so PR-4 adds no DSL surface; promote
/// it if a workflow ever needs a different bound.
const MAX_SILENT_LOOP_PASSES: u32 = 2;

/// Abort the run when the pass about to be judged lost a reviewer.
///
/// Called immediately before a judge is dispatched, never from the loop
/// head: under `until … and …` a cheaper condition may fail first, in
/// which case no judge runs and there is nothing to protect (D-7).
///
///
/// The judge that follows compares reviewers against each other, so a pass in
/// which one reviewer wrote nothing produces a verdict about a panel that
/// silently shrank — the judge reads the surviving reviewer's "consensus not
/// reached" and fails the iteration forever, burning `max_iterations` worth of
/// frontier-model turns. Neither an empty manifest nor a completed status
/// catches this on its own: providers that write through their own tools
/// (Claude Code) report no `modified_files` even on success, and a proposal
/// dropped by scope or path resolution still completes the turn. So an agent
/// counts as having delivered when *either* its manifest lists modified files
/// *or* its owned files on disk changed across the pass.
fn assert_loop_agents_produced_output(check: DeliveryCheck<'_>) -> Result<()> {
    let iter_abs = check.iter_abs;
    let total = check.agent_ids.len();
    let silent = loop_agents_without_delivery(check);

    if silent.is_empty() {
        return Ok(());
    }
    bail!(
        "loop iteration {} did not deliver for {} of {} agent(s): {}. \
         Refusing to run the loop's judge against an incomplete panel — its verdict \
         would describe a panel that silently lost a member. Check the run log for \
         'Scope rejected' or 'failed to apply file proposal' lines, and verify that \
         the agents' declared paths resolve inside the workspace root.",
        iter_abs,
        silent.len(),
        total,
        silent.join("; ")
    );
}

/// Which agents produced nothing during the pass, and why we say so.
///
/// The judgement itself, without the verdict on what to do about it: the
/// delivery gate turns a non-empty result into an abort, while the
/// no-progress tracker only counts passes where *every* agent came back
/// empty.
fn loop_agents_without_delivery(check: DeliveryCheck<'_>) -> Vec<String> {
    let DeliveryCheck {
        agent_ids,
        unit_map,
        all_manifests,
        before,
        after,
        workspace_root,
        iter_abs,
    } = check;

    let mut silent: Vec<String> = Vec::new();
    for agent_id in agent_ids {
        let Some(template) = unit_map.get(agent_id.as_str()).copied() else {
            continue;
        };
        // Substitute the iteration being judged so a versioned contract
        // ("…-conclusion-v{{ITER}}.md") names this pass's artefacts.
        let unit = apply_iter_vars(template, iter_abs);
        let unit = &unit;
        let status = || match all_manifests
            .iter()
            .rev()
            .find(|m| m.work_unit_id == *agent_id)
            .map(|m| &m.status)
        {
            Some(AgentStatus::Completed) => "completed".to_string(),
            Some(AgentStatus::Failed(msg)) => format!("failed ({msg})"),
            Some(other) => format!("{other:?}"),
            None => "never dispatched".to_string(),
        };

        // Declared contract: check the exact artefacts for THIS iteration.
        // `unit` has already had {{ITER}} substituted for the pass being
        // judged, so a reviewer that wrote v3 when v4 was due fails here.
        if !unit.produces.is_empty() {
            let missing = unit.missing_declared_artifacts(workspace_root);
            if !missing.is_empty() {
                silent.push(format!(
                    "'{agent_id}' [{}] missing: {}",
                    status(),
                    missing.join(", ")
                ));
            }
            continue;
        }

        // No declared contract — fall back to "did anything this agent owns
        // change during the pass". An agent that owns nothing promises
        // nothing, so there is nothing to assert.
        if unit.scope.owned_paths.is_empty() {
            continue;
        }
        if all_manifests
            .iter()
            .rev()
            .find(|m| m.work_unit_id == *agent_id)
            .is_some_and(|m| !m.modified_files.is_empty())
        {
            continue;
        }
        let changed = match (before.get(agent_id), after.get(agent_id)) {
            (Some(b), Some(a)) => a != b,
            // No snapshot means no owned globs to watch; the manifest is the
            // only signal available and it said nothing, so don't guess.
            _ => true,
        };
        if changed {
            continue;
        }
        silent.push(format!(
            "'{agent_id}' [{}] wrote nothing under: {}",
            status(),
            unit.scope.owned_paths.join(", ")
        ));
    }

    silent
}

/// Build a compact, deterministic textual digest of the most recent loop
/// iteration for injection into a judge prompt via `{{ITER_EVIDENCE}}`.
///
/// Inputs are trimmed: we show up to the last `loop_agent_count`
/// completed worker manifests (one per worker in the loop body), summary +
/// first 20 modified files per worker. Long outputs are truncated to keep
/// context cheap; the judge is expected to inspect files directly via tools
/// if it needs more.
fn build_iter_evidence(
    all_manifests: &[AgentManifest],
    loop_agent_ids: &[String],
    iter_abs: u32,
) -> String {
    use std::collections::HashSet;
    let loop_set: HashSet<&str> = loop_agent_ids.iter().map(String::as_str).collect();

    // Walk backwards; collect the most recent manifest per loop-agent id.
    let mut by_agent: std::collections::HashMap<&str, &AgentManifest> = Default::default();
    for m in all_manifests.iter().rev() {
        if loop_set.contains(m.work_unit_id.as_str())
            && !by_agent.contains_key(m.work_unit_id.as_str())
        {
            by_agent.insert(m.work_unit_id.as_str(), m);
        }
        if by_agent.len() == loop_set.len() {
            break;
        }
    }

    let mut out = String::with_capacity(512);
    out.push_str("### Iteration ");
    out.push_str(&iter_abs.to_string());
    out.push_str(" evidence\n\n");

    if by_agent.is_empty() {
        out.push_str("_No completed worker manifests available yet._\n");
        return out;
    }

    // Emit in the user-declared loop order so output is deterministic.
    for agent_id in loop_agent_ids {
        let Some(m) = by_agent.get(agent_id.as_str()) else {
            continue;
        };
        out.push_str("- **agent `");
        out.push_str(agent_id);
        out.push_str("`** — status: ");
        match &m.status {
            AgentStatus::Completed => out.push_str("completed"),
            AgentStatus::Failed(msg) => {
                out.push_str("failed (");
                out.push_str(msg);
                out.push(')');
            }
            AgentStatus::Running => out.push_str("running"),
            AgentStatus::Pending => out.push_str("pending"),
        }
        out.push('\n');

        if let Some(summary) = m.summary.as_deref() {
            let trimmed = summary.trim();
            if !trimmed.is_empty() {
                out.push_str("  summary: ");
                // Cap summary at 400 chars to keep the prompt bounded.
                if trimmed.len() > 400 {
                    out.push_str(&trimmed[..400]);
                    out.push_str("…");
                } else {
                    out.push_str(trimmed);
                }
                out.push('\n');
            }
        }

        if !m.modified_files.is_empty() {
            out.push_str("  modified files (first 20):\n");
            for f in m.modified_files.iter().take(20) {
                out.push_str("    - ");
                out.push_str(&f.display().to_string());
                out.push('\n');
            }
            if m.modified_files.len() > 20 {
                out.push_str(&format!("    … and {} more\n", m.modified_files.len() - 20));
            }
        }
    }
    out
}

/// Override the worktree branch + base SHA for a single agent invocation.
/// Used by stacked-mode loop iterations to chain off the previous
/// iteration's tip instead of falling through to `WorktreeManager::provision`'s
/// default reset-from-HEAD behaviour.
#[derive(Debug, Clone)]
pub struct BranchOverride {
    pub branch: String,
    pub base_sha: String,
}

/// Run a single agent, optionally in a worktree.
async fn run_single_agent(
    unit: &WorkUnit,
    worktree_mgr: Option<&mut WorktreeManager>,
    ctx: &AgentRunContext<'_>,
    tier_router: &TierRouter,
    iteration_config: &crate::iteration::IterationConfig,
    acp_observer: Box<dyn AcpObserver>,
) -> Result<AgentManifest> {
    run_agent_inner(
        unit,
        worktree_mgr,
        ctx,
        tier_router,
        iteration_config,
        acp_observer,
        false,
        None,
    )
    .await
}

/// Like [`run_single_agent`] but provisions the worktree at a specific
/// branch + base SHA. Sets the resulting manifest's branch to the override.
async fn run_single_agent_with_branch(
    unit: &WorkUnit,
    worktree_mgr: Option<&mut WorktreeManager>,
    ctx: &AgentRunContext<'_>,
    tier_router: &TierRouter,
    iteration_config: &crate::iteration::IterationConfig,
    acp_observer: Box<dyn AcpObserver>,
    override_branch: BranchOverride,
) -> Result<AgentManifest> {
    run_agent_inner(
        unit,
        worktree_mgr,
        ctx,
        tier_router,
        iteration_config,
        acp_observer,
        false,
        Some(override_branch),
    )
    .await
}

/// Run a work unit in **read-only mode**: the write gate is configured to
/// `RejectAll`, silently discarding any write proposals the backend emits.
/// Use for judge / reviewer agents that must never mutate the workspace,
/// even if the underlying model attempts a Write/Edit tool call.
async fn run_readonly_agent(
    unit: &WorkUnit,
    ctx: &AgentRunContext<'_>,
    tier_router: &TierRouter,
    iteration_config: &crate::iteration::IterationConfig,
    acp_observer: Box<dyn AcpObserver>,
) -> Result<AgentManifest> {
    // No worktree: judge should not even see a private checkout — it inspects
    // the workspace as it stands after the iteration's workers have merged.
    run_agent_inner(
        unit,
        None,
        ctx,
        tier_router,
        iteration_config,
        acp_observer,
        true,
        None,
    )
    .await
}

async fn run_agent_inner(
    unit: &WorkUnit,
    worktree_mgr: Option<&mut WorktreeManager>,
    ctx: &AgentRunContext<'_>,
    tier_router: &TierRouter,
    iteration_config: &crate::iteration::IterationConfig,
    acp_observer: Box<dyn AcpObserver>,
    read_only: bool,
    branch_override: Option<BranchOverride>,
) -> Result<AgentManifest> {
    let workspace_root = ctx.workspace_root;
    let context_files = ctx.context_files;
    let memory = ctx.memory.clone();
    let read_namespaces = ctx.read_namespaces;
    let swarm_observer = ctx.swarm_observer;
    let git_coordinator = ctx.git_coordinator.clone();
    let validation = ctx.validation.clone();
    let board = ctx.board.clone();
    let repo_map = ctx.repo_map.clone();
    let impact_text = ctx.impact_texts.get(&unit.id).cloned();
    let pre_fetched_memory_text = (*ctx.pre_fetched_memory).clone();
    let in_worktree = worktree_mgr.is_some();
    let (agent_root, override_branch_name) = if let Some(mgr) = worktree_mgr {
        let handle = if let Some(ref ov) = branch_override {
            mgr.provision_with_base(&unit.id, &ov.branch, &ov.base_sha)?
        } else {
            mgr.provision(&unit.id)?
        };
        let path = handle.path.clone();
        if !context_files.is_empty() {
            if let Err(e) = mgr.inject_context_files(&unit.id, context_files) {
                tracing::warn!("Failed to inject context files for {}: {}", unit.id, e);
            }
        }
        (path, branch_override.as_ref().map(|ov| ov.branch.clone()))
    } else {
        (workspace_root.clone(), None)
    };

    // Per-agent git worktrees need their own `.mcp.json` / `.cursor/mcp.json`.
    // Document mode and single-threaded repo runs share `workspace_root` with
    // the root synth done in `prepare_mcp_for_swarm` — re-writing here races
    // when `max_parallel > 1` without worktrees.
    if let Some(base_mcp) = &ctx.mcp_config
        && in_worktree
        && agent_root != base_mcp.worktree
    {
        let mut synth = base_mcp.clone();
        synth.worktree = agent_root.clone();
        match crate::mcp::synthesize_for_worktree(&synth) {
            Ok(paths) if !paths.is_empty() => {
                tracing::debug!(
                    agent_id = %unit.id,
                    files = paths.len(),
                    "synthesized MCP config for agent worktree"
                );
            }
            Ok(_) => {
                // Zero files is legitimate intent (synthesis suppressed —
                // e.g. Codex trust not granted with nothing else to
                // write), but it must be visible: this agent runs without
                // per-worktree gaviero MCP wiring.
                tracing::warn!(
                    agent_id = %unit.id,
                    worktree = %agent_root.display(),
                    "MCP config synthesis wrote no files for this worktree \
                     (e.g. Codex trust not granted); agent runs without \
                     per-worktree MCP configs"
                );
            }
            Err(e) => {
                // F2: an agent silently launched without its MCP configs
                // violates the runtime-parity contract (no degraded
                // modes). Fail this work unit with the remedy instead.
                return Err(anyhow::anyhow!(
                    "failed to synthesize MCP config for agent `{}` in worktree {}: {e} — \
                     fix the worktree (permissions / conflicting configs) or disable MCP \
                     for this run",
                    unit.id,
                    agent_root.display(),
                ));
            }
        }
    }

    let write_mode = if read_only {
        WriteMode::RejectAll
    } else {
        WriteMode::AutoAccept
    };
    let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
        write_mode,
        Box::new(NoopWriteGateObserver),
    )));
    let engine = crate::iteration::IterationEngine::new(iteration_config.clone());

    swarm_observer.on_agent_state_changed(&unit.id, &AgentStatus::Running, "starting");

    let mut manifest = engine
        .run_with_backend_factory(
            unit.clone(),
            write_gate,
            &agent_root,
            memory.as_ref(),
            read_namespaces,
            acp_observer.as_ref(),
            validation.as_deref(),
            board.as_deref(),
            (*repo_map).as_ref(),
            impact_text.as_deref(),
            pre_fetched_memory_text.as_deref(),
            ctx.swarm_extras,
            ctx.skip_repo_context,
            |candidate| {
                let backend = resolve_backend_for_unit(tier_router, candidate)?;
                swarm_observer.on_tier_dispatch(&candidate.id, candidate.tier, backend.name());
                Ok(backend)
            },
        )
        .await
        .manifest;

    swarm_observer.on_agent_state_changed(
        &unit.id,
        &manifest.status,
        manifest.summary.as_deref().unwrap_or(""),
    );

    // Commit changes and record branch name if running in a worktree.
    // The GitCoordinator serializes concurrent commits to prevent .git/index.lock races.
    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
        let summary = manifest
            .summary
            .as_deref()
            .unwrap_or("task complete")
            .to_string();
        let agent_root_c = agent_root.clone();
        let unit_id_c = unit.id.clone();
        let owned_c = unit.scope.owned_paths.clone();
        let changed = git_coordinator
            .lock_git(move || commit_agent_changes(&agent_root_c, &unit_id_c, &summary, &owned_c))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e);
                vec![]
            });
        manifest.modified_files = changed;
        // When the caller provided a per-iteration branch override (stacked
        // mode), record THAT branch name on the manifest so the merge phase
        // can treat it correctly and downstream iterations can chain off it.
        manifest.branch =
            Some(override_branch_name.unwrap_or_else(|| format!("gaviero/{}", unit.id)));
    }

    Ok(manifest)
}

fn resolve_backend_for_unit(
    router: &TierRouter,
    unit: &WorkUnit,
) -> Result<Box<dyn super::backend::AgentBackend>> {
    router.resolve_backend(unit).map_err(|reason| {
        anyhow::anyhow!("backend resolution failed for '{}': {}", unit.id, reason)
    })
}

async fn run_post_execution_verification(
    config: &super::plan::VerificationConfig,
    manifests: &[AgentManifest],
    workspace_root: &std::path::Path,
    excludes: &[String],
    observer: &dyn SwarmObserver,
) -> Result<bool> {
    if !config.compile && !config.clippy && !config.test && !config.impact_tests {
        return Ok(true);
    }

    observer.on_phase_changed("verifying");
    observer.on_verification_started("workflow_config");

    let modified_files = collect_completed_modified_files(manifests);
    let outcome = run_verification_checks(
        config,
        workspace_root,
        excludes,
        Some(modified_files.as_slice()),
    )
    .await?;
    if !outcome.passed() {
        observer.on_verification_complete(false);
        return Ok(false);
    }

    observer.on_verification_complete(true);
    Ok(true)
}

fn collect_completed_modified_files(manifests: &[AgentManifest]) -> Vec<std::path::PathBuf> {
    manifests
        .iter()
        .filter(|m| matches!(m.status, AgentStatus::Completed))
        .flat_map(|m| m.modified_files.iter().cloned())
        .collect()
}

/// Outcome of a `verify {}` block.
///
/// Names the first check that failed rather than collapsing to `false`,
/// so a loop gate can tell the next iteration's agents *which* check
/// blocked them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum VerificationOutcome {
    Passed,
    /// The failing check and its bounded diagnostic output.
    Failed {
        check: String,
        output: String,
    },
}

impl VerificationOutcome {
    fn passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

async fn run_verification_checks(
    config: &super::plan::VerificationConfig,
    workspace_root: &std::path::Path,
    excludes: &[String],
    modified_files: Option<&[std::path::PathBuf]>,
) -> Result<VerificationOutcome> {
    if config.compile
        && let Some(output) = run_verification_command(workspace_root, "cargo", &["check"]).await?
    {
        return Ok(VerificationOutcome::Failed {
            check: "cargo check".into(),
            output,
        });
    }

    if config.test
        && let Some(output) = run_test_verification(workspace_root, &[], false).await?
    {
        return Ok(VerificationOutcome::Failed {
            check: "cargo test".into(),
            output,
        });
    }

    if config.impact_tests {
        let failure = if let Some(files) = modified_files {
            run_test_verification(workspace_root, files, true).await?
        } else {
            run_conservative_impact_tests(workspace_root, excludes).await?
        };
        if let Some(output) = failure {
            return Ok(VerificationOutcome::Failed {
                check: "impact tests".into(),
                output,
            });
        }
    }

    let clippy_args = ["clippy", "--", "-D", "warnings"];
    if config.clippy
        && let Some(output) =
            run_verification_command(workspace_root, "cargo", &clippy_args).await?
    {
        return Ok(VerificationOutcome::Failed {
            check: "cargo clippy -- -D warnings".into(),
            output,
        });
    }

    Ok(VerificationOutcome::Passed)
}

async fn run_verification_command(
    workspace_root: &std::path::Path,
    program: &str,
    args: &[&str],
) -> Result<Option<String>> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .with_context(|| {
            format!(
                "verification command could not be executed: `{} {}`",
                program,
                args.join(" ")
            )
        })?;
    if output.status.success() {
        return Ok(None);
    }
    let combined = merge_command_output(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    );
    Ok(Some(truncate_gate_output(&combined, GATE_OUTPUT_CAP)))
}

/// Run the test suite as a verification check.
///
/// Same tri-state as [`run_verification_command`]: `Err` when the suite
/// could not be run at all, `Ok(None)` on pass, `Ok(Some(output))` on
/// failure with bounded diagnostics.
///
/// A timeout stays an ordinary failure — a suite too slow to finish is a
/// verdict the agent can act on, unlike a harness that never started.
async fn run_test_verification(
    workspace_root: &std::path::Path,
    modified_files: &[std::path::PathBuf],
    targeted: bool,
) -> Result<Option<String>> {
    let report = super::verify::test_runner::run(
        &super::verify::test_runner::TestRunnerConfig {
            command: None,
            targeted,
            ..Default::default()
        },
        modified_files,
        workspace_root,
    )
    .await?;

    classify_test_report(report)
}

/// Map a [`TestReport`] onto the verification tri-state.
///
/// Split out from the runner call so the distinction that matters —
/// "could not run" versus "ran and failed" — is testable without a real
/// cargo project on disk.
fn classify_test_report(report: crate::swarm::verify::TestReport) -> Result<Option<String>> {
    if let Some(cause) = report.execution_error {
        bail!("test verification could not be run: {cause}");
    }

    if report.passed {
        return Ok(None);
    }

    Ok(Some(truncate_gate_output(
        &merge_command_output(&report.stdout, &report.stderr),
        GATE_OUTPUT_CAP,
    )))
}

/// Run only the tests in the blast radius of the workspace's sources.
///
/// Same tri-state as [`run_verification_command`]; the failing module's
/// own diagnostics are what comes back, since that is what the agent
/// needs to fix.
async fn run_conservative_impact_tests(
    workspace_root: &std::path::Path,
    excludes: &[String],
) -> Result<Option<String>> {
    match crate::repo_map::graph_builder::build_graph(workspace_root, excludes) {
        Ok((store, _)) => {
            let all_src: Vec<String> = store
                .all_file_hashes()
                .unwrap_or_default()
                .into_iter()
                .filter(|(f, _)| !f.contains("test"))
                .map(|(f, _)| f)
                .collect();
            let refs: Vec<&str> = all_src.iter().map(|s| s.as_str()).collect();
            if let Ok(impact) = store.impact_radius(&refs, 3) {
                let test_modules: Vec<String> = impact
                    .affected_tests
                    .iter()
                    .filter_map(|t| t.strip_suffix(".rs").map(|s| s.replace('/', "::")))
                    .collect();
                for test_mod in &test_modules {
                    if let Some(output) =
                        run_verification_command(workspace_root, "cargo", &["test", test_mod])
                            .await?
                    {
                        return Ok(Some(output));
                    }
                }
            }
            Ok(None)
        }
        Err(e) => {
            tracing::warn!(
                "impact_tests: graph build failed, falling back to full test: {}",
                e
            );
            run_verification_command(workspace_root, "cargo", &["test"]).await
        }
    }
}

/// Commit all changes in a worktree after an agent completes.
///
/// Stages everything with `git add -A` then commits. Returns the list of files
/// changed in the commit, or an empty vec if the working tree was already clean.
/// Commit all changes in an agent's worktree.
///
/// Stages with `git add -A`, then force-adds any **owned** paths that are
/// gitignored (e.g. `plans/` in this repo). Without `-f`, ignored artefact
/// directories look like an empty worktree: agents "succeed", merge has
/// nothing to bring back, and teardown deletes the only copies.
///
/// Returns the list of files in the resulting commit (empty if nothing to commit).
fn commit_agent_changes(
    worktree_path: &std::path::Path,
    agent_id: &str,
    summary: &str,
    owned_paths: &[String],
) -> Result<Vec<std::path::PathBuf>> {
    use std::process::Command;

    force_stage_owned_ignored(worktree_path, owned_paths)?;

    // Check for changes
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .context("git status in worktree")?;

    if status.stdout.is_empty() {
        return Ok(vec![]); // Nothing to commit
    }

    // Stage all changes
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git add in worktree")?;
    anyhow::ensure!(
        add.success(),
        "git add failed in worktree {}",
        worktree_path.display()
    );

    // Re-force owned ignored paths after `git add -A` (which won't include them).
    force_stage_owned_ignored(worktree_path, owned_paths)?;

    // Commit — silence stdout/stderr so git's progress output doesn't corrupt the TUI
    let msg = format!(
        "gaviero: agent {} — {}",
        agent_id,
        if summary.is_empty() {
            "task complete"
        } else {
            summary
        }
    );
    let commit = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(worktree_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git commit in worktree")?;
    anyhow::ensure!(
        commit.success(),
        "git commit failed in worktree {}",
        worktree_path.display()
    );

    let files = crate::git::files_changed_in_commit(worktree_path).unwrap_or_default();
    Ok(files)
}

/// `git add -f` every ignored path under the worktree that matches the agent's
/// owned scope. Skips empty / `.` owned entries so we never force-add the whole tree.
fn force_stage_owned_ignored(
    worktree_path: &std::path::Path,
    owned_paths: &[String],
) -> Result<()> {
    use std::process::Command;

    if owned_paths.is_empty() {
        return Ok(());
    }

    let scope = crate::types::FileScope {
        owned_paths: owned_paths.to_vec(),
        ..Default::default()
    };

    // Ask git which ignored paths exist; filter to owned scope only.
    let output = Command::new("git")
        .args(["status", "--porcelain", "--ignored=matching"])
        .current_dir(worktree_path)
        .output()
        .context("git status --ignored in worktree")?;
    if !output.status.success() {
        return Ok(());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut to_force: Vec<String> = Vec::new();
    for line in text.lines() {
        let path = line
            .strip_prefix("!! ")
            .or_else(|| line.strip_prefix("!!"))
            .map(str::trim)
            .unwrap_or("");
        if path.is_empty() {
            continue;
        }
        // Directories may appear as `plans/`; also try without trailing slash.
        let candidates = [path, path.trim_end_matches('/')];
        let owned = candidates.iter().any(|c| scope.is_owned(c))
            || scope.owned_paths.iter().any(|owned| {
                let prefix = owned
                    .split(['*', '?'])
                    .next()
                    .unwrap_or(owned)
                    .trim_end_matches(['/', '\\']);
                !prefix.is_empty()
                    && prefix != "."
                    && (path == prefix
                        || path.starts_with(&format!("{prefix}/"))
                        || path.starts_with(&format!("{prefix}\\")))
            });
        if owned {
            to_force.push(path.to_string());
        }
    }

    if to_force.is_empty() {
        return Ok(());
    }

    let mut args = vec!["add".to_string(), "-f".to_string(), "--".to_string()];
    args.extend(to_force);
    let add = Command::new("git")
        .args(&args)
        .current_dir(worktree_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git add -f owned ignored paths")?;
    anyhow::ensure!(
        add.success(),
        "git add -f failed in worktree {}",
        worktree_path.display()
    );
    Ok(())
}

/// Store an agent's execution result to memory (best-effort, never fails the pipeline).
///
/// Writes one aggregate entry for the agent's run, plus one sentinel entry per
/// `staleness_source` path recording the current file hash. On the next run,
/// `invalidate_stale_sources` checks these hashes and marks changed entries stale.
async fn run_fanout_wave_if_needed(
    completed_unit_id: &str,
    plan: &CompiledPlan,
    config: &SwarmConfig,
    context_files: &[(String, String)],
    memory: &Option<Arc<MemoryStores>>,
    memory_writer: &Option<WriterHandle>,
    observer: &dyn SwarmObserver,
    git_coordinator: Arc<GitCoordinator>,
    validation: Option<Arc<crate::validation_gate::ValidationPipeline>>,
    shared_board: Arc<SharedBoard>,
    analysis: &WorkspaceAnalysis,
    pre_fetched_memory: Arc<Option<String>>,
    tier_router: &TierRouter,
    iteration_config: &crate::iteration::IterationConfig,
    bus: &Arc<tokio::sync::Mutex<AgentBus>>,
    all_manifests: &mut Vec<AgentManifest>,
    exec_state: &mut ExecutionState,
    run_id: &str,
    plan_hash: &str,
    mut worktree_mgr: Option<&mut WorktreeManager>,
    make_observer: &impl Fn(&str) -> Box<dyn AcpObserver>,
) -> Result<()> {
    let Some(op) = plan
        .fanout_ops
        .iter()
        .find(|op| op.after_unit == completed_unit_id)
    else {
        return Ok(());
    };

    // Resume: skip if we already spawned for this after_unit
    if exec_state
        .spawned_ids
        .iter()
        .any(|id| id.starts_with(&format!("{}::", op.after_unit)))
    {
        tracing::info!(
            "fan-out after '{}' already materialized (resume); skipping",
            op.after_unit
        );
        return Ok(());
    }

    let manifest_path = shared_board
        .spawn_manifest_path(completed_unit_id)
        .await
        .or_else(|| {
            // Also accept workspace-root spawn_manifest.json / from-<id>.json
            let candidates = [
                config
                    .workspace_root
                    .join(format!("from-{completed_unit_id}.json")),
                config.workspace_root.join("spawn_manifest.json"),
                config
                    .workspace_root
                    .join(".gaviero")
                    .join("runs")
                    .join(plan_hash)
                    .join("artifacts")
                    .join("spawn_manifest")
                    .join(format!("from-{completed_unit_id}.json")),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

    let Some(path) = manifest_path else {
        tracing::warn!(
            "FanoutOp after '{}' but no spawn_manifest found on blackboard",
            completed_unit_id
        );
        return Ok(());
    };

    let parsed = crate::swarm::spawn::load_manifest_file(&path)?;
    let workers = crate::swarm::spawn::materialize_from_manifest(&parsed, op)?;

    let fanout_group: Vec<Vec<String>> = vec![workers.iter().map(|w| w.id.clone()).collect()];
    let scope_errors = validation::validate_scopes(&workers, &fanout_group);
    if !scope_errors.is_empty() {
        anyhow::bail!(
            "fan-out scope validation failed: {}",
            scope_errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    {
        let mut b = bus.lock().await;
        for w in &workers {
            b.register(&w.id);
        }
    }

    tracing::info!(
        "fan-out after '{}': materializing {} workers from {}",
        completed_unit_id,
        workers.len(),
        path.display()
    );

    for unit in &workers {
        if exec_state.status(&unit.id) == NodeStatus::Completed {
            continue;
        }
        exec_state.set_status(&unit.id, NodeStatus::Running);
        observer.on_agent_state_changed(&unit.id, &AgentStatus::Running, &unit.description);

        let effective_read_ns = effective_read_namespaces(unit, config, &memory);
        let agent_ctx = AgentRunContext::for_run(
            config,
            context_files,
            &effective_read_ns,
            observer,
            memory.clone(),
            git_coordinator.clone(),
            validation.clone(),
            Some(shared_board.clone()),
            analysis,
            pre_fetched_memory.clone(),
        );
        let manifest = run_single_agent(
            unit,
            worktree_mgr.as_deref_mut(),
            &agent_ctx,
            tier_router,
            iteration_config,
            make_observer(&unit.id),
        )
        .await?;

        if matches!(manifest.status, AgentStatus::Completed) {
            let effective_write_ns = effective_write_namespace(&unit, config);
            store_agent_result(
                memory,
                memory_writer,
                effective_write_ns,
                &manifest,
                unit,
                run_id,
                &config.workspace_root,
                config.extract_agent_findings,
            )
            .await;
        }
        exec_state
            .spawned_ids
            .push(format!("{}::{}", op.after_unit, unit.id));
        exec_state.record_result(&unit.id, manifest.clone());
        let _ = exec_state.save(&config.workspace_root, plan_hash);
        all_manifests.push(manifest);
    }

    // Refresh knowledge impact texts after a mutating fan-out wave
    if config.execution_mode == ExecutionMode::Repo {
        let remaining: Vec<WorkUnit> = plan
            .work_units_unordered()
            .into_iter()
            .filter(|u| !exec_state.status(&u.id).is_terminal())
            .cloned()
            .collect();
        if !remaining.is_empty() {
            let refreshed = WorkspaceAnalysis::build(config, &remaining).await;
            // Best-effort: callers hold analysis by ref; we only log refresh.
            // Full Arc swap would require refactoring execute()'s analysis binding.
            let _ = refreshed;
            tracing::info!("fan-out: rebuilt workspace analysis for remaining units");
            if let Some(cb) = &config.knowledge_invalidation {
                cb();
            }
        }
    }

    Ok(())
}

async fn store_agent_result(
    memory: &Option<Arc<MemoryStores>>,
    writer: &Option<WriterHandle>,
    write_ns: &str,
    manifest: &AgentManifest,
    unit: &WorkUnit,
    run_id: &str,
    workspace_root: &std::path::Path,
    extract_findings: bool,
) {
    if memory.is_none() {
        return;
    }
    let Some(writer) = writer else {
        return;
    };

    let privacy = match unit.privacy {
        PrivacyLevel::LocalOnly => "local_only",
        PrivacyLevel::Public => "public",
    };
    let importance = unit.memory_importance.unwrap_or(0.5);
    let metadata = EntryMetadata {
        privacy: unit.privacy,
        format_version: 1,
        source: "swarm_pipeline".into(),
    };
    let metadata_json = serde_json::to_string(&metadata).ok();

    // 1. Aggregate entry (summary of the whole agent run)
    let key = format!("agents:{}:{}", run_id, manifest.work_unit_id);
    let files: Vec<String> = manifest
        .modified_files
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    // {{SUMMARY}} resolves to the agent's full text output (preferred) or short summary.
    let summary_text = manifest
        .output
        .as_deref()
        .or(manifest.summary.as_deref())
        .unwrap_or("none");
    let content = if let Some(template) = &unit.memory_write_content {
        template
            .replace("{{SUMMARY}}", summary_text)
            .replace("{{FILES}}", &files.join(", "))
            .replace("{{AGENT}}", &manifest.work_unit_id)
            .replace("{{DESCRIPTION}}", &unit.description)
    } else {
        format!(
            "Task: {}\nTier: {:?}\nModified: {}\nOutput: {}",
            unit.description,
            unit.tier,
            files.join(", "),
            summary_text,
        )
    };
    let opts = StoreOptions {
        privacy: privacy.to_string(),
        importance,
        metadata: metadata_json.clone(),
        source_file: None,
        source_hash: None,
    };
    if let Err(e) = writer.store_with_options(write_ns, &key, &content, opts) {
        tracing::warn!("Failed to store agent result to memory: {}", e);
    }

    // 2. Per-staleness-source sentinel entries
    // Storing the current file hash lets `check_staleness` detect changes on the next run.
    for source_path in &unit.staleness_sources {
        let abs = workspace_root.join(source_path);
        let abs_str = abs.to_string_lossy().to_string();
        let hash = match file_hash(&abs) {
            Ok(h) => h,
            Err(_) => continue, // path may not exist yet; skip silently
        };
        let src_key = format!(
            "agents:{}:{}:src:{}",
            run_id, manifest.work_unit_id, source_path
        );
        let src_content = format!("Source snapshot: {} (hash: {})", source_path, hash);
        let src_opts = StoreOptions {
            privacy: privacy.to_string(),
            importance,
            metadata: metadata_json.clone(),
            source_file: Some(abs_str), // absolute path — matches check_staleness input
            source_hash: Some(hash),
        };
        if let Err(e) = writer.store_with_options(write_ns, &src_key, &src_content, src_opts) {
            tracing::warn!("Failed to store source snapshot for {}: {}", source_path, e);
        }
    }

    // 3. PR-6 replacement: route the agent's findings through the per-turn
    // memory extractor — the same `enqueue_post_turn` path a TUI chat turn
    // uses — so durable facts are curated with `source=agent`, low trust,
    // and subject to dedup/consolidation/decay. This is additive to the raw
    // aggregate above (which serves run bookkeeping + staleness). No-op
    // unless enabled; degrades to a History row when `writer` carries no
    // extraction LLM (e.g. the headless CLI fallback writer).
    if extract_findings
        && let Some((transcript, annotations)) = agent_findings_transcript(unit, manifest)
    {
        let turn_id = format!("swarm:{run_id}:{}", manifest.work_unit_id);
        let repo_id = crate::memory::hash_path(workspace_root);
        crate::context_planner::enqueue_post_turn(crate::context_planner::PostTurnRequest {
            writer,
            session_id: run_id,
            turn_id: &turn_id,
            repo_id: &repo_id,
            // `resolve_scope` falls back Module → Repo when there is no
            // module context, so leaving this `None` is safe.
            module_path: None,
            run_id,
            transcript,
            annotations,
            // Telemetry is off for swarm findings, so `response_text` is unused.
            response_text: String::new(),
            extractor_enabled: true,
            telemetry_enabled: false,
        });
    }
}

/// D2: the widest scope a swarm-originated annotation flag may claim.
///
/// Chat turns keep the full range (`extractor::resolve_scope` is
/// untouched) because they are human-supervised, one turn at a time. The
/// swarm firehose — many agents × many units × every run — is what would
/// actually accumulate global-scope rows, so it is clamped here.
const SWARM_MAX_ANNOTATION_SCOPE: &str = "repo";

/// Build the per-turn-extractor transcript for a completed swarm agent's
/// findings: the unit's task plus the agent's full text output (falling
/// back to its short summary). Returns `None` when there is no usable
/// output, so the caller skips the extractor enqueue entirely.
///
/// The agent's output still carries its `<turn_annotations>` sidecar at
/// this point. It is parsed out here so the flags reach the writer as
/// `LlmAnnotated` memories (trust 0.70) instead of being buried in the
/// transcript for the extractor to re-derive at `LlmExtracted` (0.60).
/// The stripped text is what goes into the transcript.
fn agent_findings_transcript(
    unit: &WorkUnit,
    manifest: &AgentManifest,
) -> Option<(String, Option<serde_json::Value>)> {
    let raw = manifest
        .output
        .as_deref()
        .or(manifest.summary.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    let parsed = crate::memory::parse_and_strip(raw);
    if let Some(err) = &parsed.parse_error {
        // Delimiters were present but the JSON did not parse. Same
        // treatment as the chat path in `memory/writer.rs`: warn loudly
        // so prompt drift is visible, keep the transcript.
        tracing::warn!(
            target: "memory_annotations",
            work_unit_id = %manifest.work_unit_id,
            error = %err,
            "swarm agent emitted a malformed <turn_annotations> block — \
             dropping annotations, keeping the transcript"
        );
    }

    let annotations = parsed.annotations.and_then(|mut ann| {
        for f in &mut ann.flags {
            if matches!(
                f.scope.to_ascii_lowercase().as_str(),
                "workspace" | "global"
            ) {
                tracing::debug!(
                    target: "memory_annotations",
                    work_unit_id = %manifest.work_unit_id,
                    from = %f.scope,
                    to = SWARM_MAX_ANNOTATION_SCOPE,
                    "D2: clamping swarm annotation scope"
                );
                f.scope = SWARM_MAX_ANNOTATION_SCOPE.to_string();
            }
        }
        serde_json::to_value(&ann).ok()
    });

    let body = parsed.stripped.trim();
    Some((
        format!(
            "TASK: {}\n\nAGENT {} OUTPUT:\n{}",
            unit.description, manifest.work_unit_id, body
        ),
        annotations,
    ))
}

/// Plan a coordinated swarm: Opus produces a `.gaviero` DSL file for user review.
///
/// This is the preferred entry point for coordinated runs. Unlike
/// `execute_coordinated()`, this function does NOT execute any agents.
/// It returns the raw DSL text that the caller should:
/// 1. Write to `tmp/gaviero_plan_<timestamp>.gaviero`
/// 2. Present to the user for review/editing
/// 3. Compile with `gaviero_dsl::compile()` and pass to `execute()`
///
/// This design eliminates the fragile JSON → WorkUnit parsing path and makes
/// the coordinator's plan visible and auditable before any agent runs.
pub async fn plan_coordinated(
    prompt: &str,
    config: &SwarmConfig,
    coordinator_config: CoordinatorConfig,
    memory: Option<Arc<MemoryStores>>,
    observer: &dyn SwarmObserver,
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver> + Send + Sync,
) -> Result<String> {
    observer.on_coordination_started(prompt);
    observer.on_agent_state_changed(
        "coordinator",
        &AgentStatus::Running,
        "Coordinator planning (DSL)...",
    );
    observer.on_tier_dispatch(
        "coordinator",
        crate::types::ModelTier::Expensive,
        &coordinator_config.model,
    );

    let file_list = collect_file_list(&config.workspace_root)?;
    let coordinator = Coordinator::new(memory, coordinator_config);
    let coord_observer = make_observer("coordinator");

    match coordinator
        .plan_as_dsl(
            prompt,
            &config.workspace_root,
            &file_list,
            &config.read_namespaces,
            Some(coord_observer),
        )
        .await
    {
        Ok(dsl) => {
            observer.on_agent_state_changed(
                "coordinator",
                &AgentStatus::Completed,
                "DSL plan ready — review before executing",
            );
            Ok(dsl)
        }
        Err(e) => {
            observer.on_agent_state_changed(
                "coordinator",
                &AgentStatus::Failed(e.to_string()),
                &e.to_string(),
            );
            Err(e)
        }
    }
}

/// Undo a swarm run by hard-resetting the repo to its pre-swarm state.
///
/// Deletes all agent branches that were part of `result`, then runs
/// `git reset --hard <pre_swarm_sha>`. This is destructive but recoverable
/// via `git reflog`.
pub fn revert_swarm(
    workspace_root: &std::path::Path,
    result: &super::models::SwarmResult,
) -> Result<()> {
    if result.pre_swarm_sha.is_empty() {
        anyhow::bail!("no pre-swarm SHA recorded — cannot revert (was this a non-worktree run?)");
    }

    // Delete agent branches first so they don't linger after the reset
    for manifest in &result.manifests {
        if let Some(ref branch) = manifest.branch {
            if let Err(e) = crate::git::delete_branch(workspace_root, branch) {
                tracing::warn!("Could not delete branch {}: {}", branch, e);
            }
        }
    }

    crate::git::reset_hard(workspace_root, &result.pre_swarm_sha)?;
    Ok(())
}

/// Outcome of [`cleanup_gaviero_branches`].
#[derive(Debug, Clone, Default)]
pub struct BranchCleanupReport {
    /// Branches matched the `gaviero/` prefix and were eligible for deletion.
    pub matched: Vec<String>,
    /// Branches actually deleted (empty when `dry_run` is true).
    pub deleted: Vec<String>,
    /// Branches skipped because they are currently checked out.
    pub skipped_current: Vec<String>,
}

/// Delete local branches whose name starts with `gaviero/`. These are the
/// per-agent / per-iteration branches produced by swarm runs (see
/// `WorktreeManager::provision`). Stacked-loop runs leave them behind by
/// design — the merge phase intentionally skips per-iteration branches.
///
/// - `dry_run = true`: enumerate matching branches without deleting.
/// - The currently checked-out branch (if it happens to match) is always
///   skipped — `git branch -D` would refuse it anyway.
/// - `git worktree prune` is invoked first so dead worktree refs don't
///   block branch deletion.
pub fn cleanup_gaviero_branches(
    workspace_root: &std::path::Path,
    dry_run: bool,
) -> Result<BranchCleanupReport> {
    let _ = crate::git::worktree_prune(workspace_root);

    let matched = crate::git::list_local_branches_with_prefix(workspace_root, "gaviero/")?;
    let current = crate::git::GitRepo::open(workspace_root)
        .ok()
        .and_then(|r| r.current_branch().ok());

    let mut report = BranchCleanupReport {
        matched: matched.clone(),
        ..Default::default()
    };

    for branch in matched {
        if Some(&branch) == current.as_ref() {
            report.skipped_current.push(branch);
            continue;
        }
        if dry_run {
            continue;
        }
        match crate::git::delete_branch(workspace_root, &branch) {
            Ok(()) => report.deleted.push(branch),
            Err(e) => tracing::warn!("Could not delete branch {}: {}", branch, e),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileScope, ModelTier, PrivacyLevel};
    use std::collections::HashMap;

    fn test_unit(tier: ModelTier, privacy: PrivacyLevel, model: Option<&str>) -> WorkUnit {
        WorkUnit {
            id: "unit".into(),
            description: "test task".into(),
            scope: FileScope {
                owned_paths: vec!["src/".into()],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            produces: vec![],
            depends_on: vec![],
            #[allow(deprecated)]
            backend: Default::default(),
            model: model.map(|m| m.to_string()),
            effort: None,
            extra: Vec::new(),
            tier,
            privacy,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            timeout_secs: 3600,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
            extra_allowed_tools: vec![],
        }
    }

    fn manifest_with(output: Option<&str>, summary: Option<&str>) -> AgentManifest {
        AgentManifest {
            work_unit_id: "unit-a".into(),
            status: AgentStatus::Completed,
            modified_files: vec![],
            branch: None,
            summary: summary.map(String::from),
            output: output.map(String::from),
            cost_usd: 0.0,
        }
    }

    #[test]
    fn agent_findings_transcript_prefers_output_and_tags_task() {
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        let manifest = manifest_with(Some("Refactored the parser; added 3 tests."), Some("done"));
        let (t, annotations) = agent_findings_transcript(&unit, &manifest).expect("output present");
        assert!(t.contains("TASK: test task"));
        assert!(t.contains("AGENT unit-a OUTPUT:"));
        assert!(t.contains("Refactored the parser"));
        // The full output wins over the short summary.
        assert!(!t.contains("done"));
        // No sidecar in the output → nothing to hand the writer.
        assert!(annotations.is_none());
    }

    #[test]
    fn agent_findings_transcript_falls_back_to_summary() {
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        let manifest = manifest_with(None, Some("Summary only."));
        let (t, _) = agent_findings_transcript(&unit, &manifest).expect("summary present");
        assert!(t.contains("Summary only."));
    }

    #[test]
    fn agent_findings_transcript_none_when_empty() {
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        assert!(agent_findings_transcript(&unit, &manifest_with(None, None)).is_none());
        // Whitespace-only output is treated as empty → skipped.
        assert!(agent_findings_transcript(&unit, &manifest_with(Some("  \n "), None)).is_none());
    }

    /// Build an agent output that ends with a `<turn_annotations>` block
    /// carrying a single flag at `scope`.
    fn output_with_annotation(scope: &str) -> String {
        format!(
            "Refactored the parser.\n\n<turn_annotations>\n{{\
             \"v\": 1, \"flags\": [{{\"type\": \"decision\", \"importance\": 0.8, \
             \"scope\": \"{scope}\", \"text\": \"parser owns tokenisation\", \"refs\": []}}], \
             \"session_thread\": \"parser work\", \"open_questions\": []}}\n\
             </turn_annotations>"
        )
    }

    #[test]
    fn agent_findings_transcript_extracts_and_strips_annotations() {
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        let manifest = manifest_with(Some(&output_with_annotation("repo")), None);
        let (t, annotations) = agent_findings_transcript(&unit, &manifest).expect("output present");

        assert!(
            !t.contains("turn_annotations"),
            "the sidecar must be stripped from the transcript: {t}"
        );
        assert!(t.contains("Refactored the parser."));

        let json = annotations.expect("a valid block must yield annotations");
        assert_eq!(json["flags"][0]["scope"], "repo");
        assert_eq!(json["flags"][0]["text"], "parser owns tokenisation");
    }

    #[test]
    fn agent_findings_transcript_clamps_global_scope_to_repo() {
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);

        for declared in ["global", "workspace"] {
            let manifest = manifest_with(Some(&output_with_annotation(declared)), None);
            let (_, annotations) =
                agent_findings_transcript(&unit, &manifest).expect("output present");
            let json = annotations.expect("annotations present");
            assert_eq!(
                json["flags"][0]["scope"], "repo",
                "D2: a swarm flag declaring `{declared}` must be clamped to repo"
            );
        }

        // Narrower scopes pass through untouched.
        let manifest = manifest_with(Some(&output_with_annotation("run")), None);
        let (_, annotations) = agent_findings_transcript(&unit, &manifest).expect("output present");
        assert_eq!(
            annotations.expect("annotations present")["flags"][0]["scope"],
            "run"
        );
    }

    #[test]
    fn backend_resolution_uses_router_models() {
        let router = TierRouter::new(TierConfig::default(), false);
        let backend = resolve_backend_for_unit(
            &router,
            &test_unit(ModelTier::Cheap, PrivacyLevel::Public, None),
        )
        .expect("cheap unit should resolve");

        assert!(backend.name().contains("haiku"));
    }

    #[test]
    fn backend_resolution_rejects_blocked_units() {
        let router = TierRouter::new(TierConfig::default(), false);
        let err = resolve_backend_for_unit(
            &router,
            &test_unit(ModelTier::Cheap, PrivacyLevel::LocalOnly, None),
        )
        .err()
        .expect("local-only unit should be blocked without local backend");

        assert!(err.to_string().contains("backend resolution failed"));
    }

    #[test]
    fn judge_verdict_parser_accepts_line_protocols() {
        assert_eq!(parse_judge_verdict("PASS"), Some(JudgeVerdict::Pass));
        assert_eq!(
            parse_judge_verdict("Verdict: FAIL\nReason: conflict remains"),
            Some(JudgeVerdict::Fail)
        );
        assert_eq!(
            parse_judge_verdict("Reasoning...\nFINAL VERDICT: PASS"),
            Some(JudgeVerdict::Pass)
        );
    }

    #[test]
    fn judge_verdict_parser_accepts_json_protocols() {
        assert_eq!(
            parse_judge_verdict(r#"{"pass":true,"reason":"stable"}"#),
            Some(JudgeVerdict::Pass)
        );
        assert_eq!(
            parse_judge_verdict(r#"{"verdict":"fail","reason":"conflicts remain"}"#),
            Some(JudgeVerdict::Fail)
        );
    }

    #[test]
    fn judge_verdict_parser_rejects_ambiguous_text() {
        assert_eq!(
            parse_judge_verdict("The plans mostly pass muster, but I need more analysis."),
            None
        );
    }

    #[test]
    fn judge_verdict_parser_accepts_extended_vocabulary() {
        assert_eq!(
            parse_judge_verdict("VERDICT: APPROVED"),
            Some(JudgeVerdict::Pass)
        );
        assert_eq!(parse_judge_verdict("LGTM"), Some(JudgeVerdict::Pass));
        assert_eq!(parse_judge_verdict("CONVERGED"), Some(JudgeVerdict::Pass));
        assert_eq!(parse_judge_verdict("REJECTED"), Some(JudgeVerdict::Fail));
        assert_eq!(
            parse_judge_verdict(r#"{"verdict":"partial","reason":"gaps remain"}"#),
            Some(JudgeVerdict::Partial)
        );
    }

    #[test]
    fn judge_verdict_parser_tolerates_trailing_punctuation_and_markdown() {
        assert_eq!(parse_judge_verdict("PASS."), Some(JudgeVerdict::Pass));
        assert_eq!(parse_judge_verdict("**FAIL**"), Some(JudgeVerdict::Fail));
        assert_eq!(
            parse_judge_verdict("VERDICT: PASS — tests green"),
            Some(JudgeVerdict::Pass)
        );
    }

    #[test]
    fn iter_evidence_digest_includes_loop_agents_and_respects_order() {
        use std::path::PathBuf;
        let manifests = vec![
            AgentManifest {
                work_unit_id: "unrelated".into(),
                status: AgentStatus::Completed,
                modified_files: vec![PathBuf::from("x.rs")],
                branch: None,
                summary: Some("should not appear".into()),
                output: None,
                cost_usd: 0.0,
            },
            AgentManifest {
                work_unit_id: "alpha".into(),
                status: AgentStatus::Completed,
                modified_files: vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")],
                branch: None,
                summary: Some("alpha did things".into()),
                output: None,
                cost_usd: 0.0,
            },
            AgentManifest {
                work_unit_id: "beta".into(),
                status: AgentStatus::Failed("boom".into()),
                modified_files: vec![],
                branch: None,
                summary: Some("beta failed".into()),
                output: None,
                cost_usd: 0.0,
            },
        ];
        let ids = vec!["beta".to_string(), "alpha".to_string()];
        let ev = build_iter_evidence(&manifests, &ids, 3);
        assert!(ev.contains("Iteration 3 evidence"));
        // Order must follow ids, not manifest order.
        let pos_beta = ev.find("agent `beta`").expect("beta present");
        let pos_alpha = ev.find("agent `alpha`").expect("alpha present");
        assert!(pos_beta < pos_alpha, "beta should appear before alpha");
        assert!(ev.contains("failed (boom)"));
        assert!(ev.contains("alpha did things"));
        assert!(ev.contains("a.rs"));
        // Unrelated manifest is filtered out.
        assert!(!ev.contains("unrelated"));
        assert!(!ev.contains("should not appear"));
    }

    #[test]
    fn iter_evidence_empty_when_no_matching_manifests() {
        let ev = build_iter_evidence(&[], &["a".into()], 1);
        assert!(ev.contains("No completed worker manifests"));
    }

    #[test]
    fn apply_iter_vars_with_evidence_substitutes_placeholder() {
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        unit.coordinator_instructions =
            "iter {{ITER}} prev {{PREV_ITER}} ev:\n{{ITER_EVIDENCE}}".into();
        let out = apply_iter_vars_with_evidence(&unit, 5, "EVIDENCE_HERE");
        assert!(out.coordinator_instructions.contains("iter 5 prev 4"));
        assert!(out.coordinator_instructions.contains("EVIDENCE_HERE"));
        assert!(!out.coordinator_instructions.contains("{{ITER_EVIDENCE}}"));
    }

    // ── H0 / PR-2: gate feedback in iteration prompts ────────────────
    //
    // The gate machinery itself is tested in `swarm::loop_gate`; these
    // cover only how a failure reaches the next iteration's agents.

    fn gate_failure(probe: &str, output: &str) -> GateFailure {
        GateFailure {
            probe: probe.to_string(),
            status: "exit status: 1".to_string(),
            output: output.to_string(),
        }
    }

    #[test]
    fn gate_feedback_substitutes_the_author_placeholder() {
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        unit.coordinator_instructions =
            "Fix iteration {{ITER}}.\n\nGate said:\n{{GATE_FEEDBACK}}".into();
        let failure = gate_failure("cargo test", "assertion failed at line 12");

        let out = apply_iter_vars_with_gate_feedback(&unit, 3, Some(&failure));

        assert!(out.coordinator_instructions.contains("Fix iteration 3."));
        assert!(
            out.coordinator_instructions
                .contains("assertion failed at line 12")
        );
        assert!(
            !out.coordinator_instructions
                .contains(GATE_FEEDBACK_PLACEHOLDER)
        );
        // The author chose the position, so no section is appended.
        assert!(
            !out.coordinator_instructions
                .contains("## Previous gate failure")
        );
    }

    #[test]
    fn gate_feedback_appends_a_section_when_the_script_has_no_placeholder() {
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        unit.coordinator_instructions = "Keep going.".into();
        let failure = gate_failure("cargo test", "assertion failed at line 12");

        let out = apply_iter_vars_with_gate_feedback(&unit, 2, Some(&failure));

        assert!(out.coordinator_instructions.starts_with("Keep going."));
        assert!(
            out.coordinator_instructions
                .contains("## Previous gate failure")
        );
        assert!(
            out.coordinator_instructions
                .contains("assertion failed at line 12")
        );
    }

    #[test]
    fn gate_feedback_placeholder_collapses_when_nothing_failed() {
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        unit.coordinator_instructions = "Body.\n{{GATE_FEEDBACK}}".into();

        let out = apply_iter_vars_with_gate_feedback(&unit, 2, None);

        assert!(
            !out.coordinator_instructions
                .contains(GATE_FEEDBACK_PLACEHOLDER)
        );
        assert!(
            !out.coordinator_instructions
                .contains("## Previous gate failure")
        );
        assert_eq!(out.coordinator_instructions.trim(), "Body.");
    }

    #[test]
    fn gate_feedback_never_touches_the_produces_contract() {
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        unit.produces = vec!["out/report-v{{ITER}}.md".into()];
        let failure = gate_failure("cargo test", "boom");

        let out = apply_iter_vars_with_gate_feedback(&unit, 4, Some(&failure));

        assert_eq!(out.produces, vec!["out/report-v4.md".to_string()]);
    }

    // ── H0 / PR-2: verification checks report like command probes ────

    /// A shell invocation valid for `run_verification_command`, which
    /// takes program + args directly rather than a shell string.
    fn shell(script: &'static str) -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            ("cmd", vec!["/C", script])
        } else {
            ("sh", vec!["-c", script])
        }
    }

    #[tokio::test]
    async fn verification_command_passes_on_exit_zero() {
        let dir = tempfile::tempdir().unwrap();
        let (program, args) = shell("exit 0");

        let result = run_verification_command(dir.path(), program, &args)
            .await
            .unwrap();

        assert!(result.is_none(), "exit 0 means the check passed");
    }

    #[tokio::test]
    async fn verification_command_captures_diagnostics_on_failure() {
        // Finding 2: a failing `cargo check` must tell the agent *why*,
        // not just that it failed.
        let dir = tempfile::tempdir().unwrap();
        let (program, args) = shell("echo compiler-diagnostic && exit 3");

        let output = run_verification_command(dir.path(), program, &args)
            .await
            .unwrap()
            .expect("non-zero exit is a failing check");

        assert!(
            output.contains("compiler-diagnostic"),
            "diagnostics must be captured, got {output:?}"
        );
    }

    #[tokio::test]
    async fn verification_command_that_cannot_spawn_is_a_hard_error() {
        // Finding 1: this used to be `.unwrap_or(false)`, i.e. an
        // ordinary failing gate that burned every remaining iteration.
        let dir = tempfile::tempdir().unwrap();

        let err = run_verification_command(dir.path(), "definitely-not-a-real-binary-xyz", &[])
            .await
            .expect_err("an unspawnable verification command must be a hard error");

        assert!(
            format!("{err:#}").contains("could not be executed"),
            "error must name the failure mode, got {err:#}"
        );
    }

    fn test_report(passed: bool, stdout: &str, stderr: &str) -> crate::swarm::verify::TestReport {
        crate::swarm::verify::TestReport {
            exit_code: if passed { 0 } else { 101 },
            passed,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            duration: std::time::Duration::ZERO,
            targeted_filter: None,
            parsed_results: None,
            execution_error: None,
        }
    }

    #[test]
    fn a_passing_test_suite_is_not_a_gate_failure() {
        let outcome = classify_test_report(test_report(true, "ok", "")).unwrap();
        assert!(outcome.is_none());
    }

    #[test]
    fn a_failing_test_suite_carries_its_diagnostics() {
        let output = classify_test_report(test_report(false, "running 2 tests", "assert failed"))
            .unwrap()
            .expect("a failing suite is a gate failure");

        assert!(output.contains("running 2 tests"));
        assert!(output.contains("assert failed"));
    }

    #[test]
    fn a_test_suite_that_could_not_run_is_a_hard_error() {
        let mut report = test_report(false, "", "Test execution error: no such file");
        report.execution_error = Some("test command could not be executed: no such file".into());

        let err = classify_test_report(report)
            .expect_err("a harness that never started is not a verdict");

        assert!(format!("{err:#}").contains("could not be run"));
    }

    #[test]
    fn a_timed_out_test_suite_stays_an_ordinary_failure() {
        // A suite too slow to finish is still a verdict the agent can
        // act on, unlike a harness that never started.
        let report = test_report(false, "", "Test execution timed out after 600s");

        let output = classify_test_report(report)
            .unwrap()
            .expect("still a failure");

        assert!(output.contains("timed out"));
    }

    #[test]
    fn verification_outcome_names_the_failing_check() {
        assert!(VerificationOutcome::Passed.passed());
        let failed = VerificationOutcome::Failed {
            check: "cargo check".into(),
            output: "compiler diagnostic".into(),
        };
        assert!(!failed.passed());
        assert_eq!(
            failed,
            VerificationOutcome::Failed {
                check: "cargo check".into(),
                output: "compiler diagnostic".into(),
            }
        );
    }

    #[test]
    fn judge_verdict_parser_extracts_fenced_json_block() {
        let text = "Reasoning: the diff looks clean.\n\n```json\n{\"verdict\":\"pass\",\"reason\":\"stable\"}\n```\n";
        assert_eq!(parse_judge_verdict(text), Some(JudgeVerdict::Pass));

        let bare = "```\n{\"pass\":false}\n```";
        assert_eq!(parse_judge_verdict(bare), Some(JudgeVerdict::Fail));
    }

    #[test]
    fn judge_verdict_parser_strips_turn_annotations_sidecar() {
        // Subprocess agents append a <turn_annotations> JSON sidecar after
        // every reply; the verdict should still parse cleanly when this
        // block trails the verdict block (the sidecar's "decision" type
        // tokens must not be confused for a verdict).
        let text = "Reasoning.\n\n```json\n{\"verdict\":\"fail\",\"reason\":\"halted not seen\"}\n```\n\n<turn_annotations>\n{\"v\":1,\"flags\":[{\"type\":\"decision\",\"importance\":0.8,\"scope\":\"repo\",\"text\":\"…\",\"refs\":[]}]}\n</turn_annotations>";
        assert_eq!(parse_judge_verdict(text), Some(JudgeVerdict::Fail));

        // No fenced verdict at all — just prose ending in a sidecar. The
        // line-scan PASS/FAIL fallback should still find the verdict.
        let prose = "Looking at apply-1.md. First line: 'HALTED: nothing to plan'.\nVERDICT: PASS\n<turn_annotations>{\"v\":1,\"flags\":[]}</turn_annotations>";
        assert_eq!(parse_judge_verdict(prose), Some(JudgeVerdict::Pass));
    }

    // ── Loop delivery gate ───────────────────────────────────────────────

    fn reviewer_unit(id: &str, owned: &[&str]) -> WorkUnit {
        let mut u = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        u.id = id.to_string();
        u.scope.owned_paths = owned.iter().map(|s| s.to_string()).collect();
        u
    }

    fn completed(id: &str, modified: &[&str]) -> AgentManifest {
        AgentManifest {
            work_unit_id: id.into(),
            status: AgentStatus::Completed,
            modified_files: modified.iter().map(std::path::PathBuf::from).collect(),
            branch: None,
            summary: Some("done".into()),
            output: None,
            cost_usd: 0.0,
        }
    }

    fn judge_until() -> super::super::plan::LoopUntilCondition {
        super::super::plan::LoopUntilCondition::Agent("convergence-judge".into())
    }

    /// Positional shorthand so the gate tests read as a table of cases.
    #[allow(clippy::too_many_arguments)]
    fn delivery_check<'a>(
        agent_ids: &'a [String],
        unit_map: &'a HashMap<&'a str, &'a WorkUnit>,
        all_manifests: &'a [AgentManifest],
        before: &'a std::collections::HashMap<String, OwnedSnapshot>,
        after: &'a std::collections::HashMap<String, OwnedSnapshot>,
        workspace_root: &'a std::path::Path,
        iter_abs: u32,
    ) -> DeliveryCheck<'a> {
        DeliveryCheck {
            agent_ids,
            unit_map,
            all_manifests,
            before,
            after,
            workspace_root,
            iter_abs,
        }
    }

    #[test]
    fn widen_owned_glob_collapses_placeholders() {
        assert_eq!(
            widen_owned_glob("{{OUT_DIR}}/{{REVIEWER_ID}}-conclusion-v{{ITER}}.md"),
            "*/*-conclusion-v*.md"
        );
        // Already-substituted globs pass through untouched.
        assert_eq!(
            widen_owned_glob("out/claude-summary-v*.md"),
            "out/claude-summary-v*.md"
        );
    }

    #[test]
    fn snapshot_owned_files_tracks_only_owned_globs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("out/claude-summary-v1.md"), "a").unwrap();
        std::fs::write(root.join("out/codex-summary-v1.md"), "b").unwrap();

        let unit = reviewer_unit("claude-refine", &["out/claude-summary-v*.md"]);
        let snaps = snapshot_owned_files(&[("claude-refine", &unit)], root);
        let snap = &snaps["claude-refine"];
        assert_eq!(snap.len(), 1);
        assert!(snap.contains_key("out/claude-summary-v1.md"));
    }

    /// The production failure: one reviewer writes every version, the other
    /// completes its turn having written nothing at all. The judge that
    /// follows would score a panel of one.
    #[test]
    fn delivery_gate_rejects_a_reviewer_that_wrote_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();

        let claude = reviewer_unit("claude-refine", &["out/claude-summary-v*.md"]);
        let codex = reviewer_unit("codex-refine", &["out/codex-summary-v*.md"]);
        let unit_map: HashMap<&str, &WorkUnit> =
            [("claude-refine", &claude), ("codex-refine", &codex)]
                .into_iter()
                .collect();
        let ids = vec!["claude-refine".to_string(), "codex-refine".to_string()];

        let before = snapshot_loop_agents(&ids, &unit_map, root);
        std::fs::write(root.join("out/claude-summary-v2.md"), "written").unwrap();
        let after = snapshot_loop_agents(&ids, &unit_map, root);

        // Both providers report Completed with no modified_files — exactly
        // what Claude Code produces on success and what a dropped file-block
        // proposal produces on failure.
        let manifests = vec![
            completed("claude-refine", &[]),
            completed("codex-refine", &[]),
        ];

        let err = assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &before, &after, root, 2,
        ))
        .expect_err("codex wrote nothing");
        let msg = format!("{err:#}");
        assert!(msg.contains("codex-refine"), "got: {msg}");
        assert!(!msg.contains("'claude-refine'"), "claude delivered: {msg}");
        assert!(msg.contains("1 of 2"), "got: {msg}");
    }

    #[test]
    fn delivery_gate_passes_when_every_reviewer_wrote() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();

        let claude = reviewer_unit("claude-refine", &["out/claude-summary-v*.md"]);
        let codex = reviewer_unit("codex-refine", &["out/codex-summary-v*.md"]);
        let unit_map: HashMap<&str, &WorkUnit> =
            [("claude-refine", &claude), ("codex-refine", &codex)]
                .into_iter()
                .collect();
        let ids = vec!["claude-refine".to_string(), "codex-refine".to_string()];

        let before = snapshot_loop_agents(&ids, &unit_map, root);
        std::fs::write(root.join("out/claude-summary-v2.md"), "a").unwrap();
        std::fs::write(root.join("out/codex-summary-v2.md"), "b").unwrap();
        let after = snapshot_loop_agents(&ids, &unit_map, root);

        let manifests = vec![
            completed("claude-refine", &[]),
            completed("codex-refine", &[]),
        ];
        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &before, &after, root, 2,
        ))
        .expect("both delivered");
    }

    /// Worktree runs report their deliverables through `modified_files`
    /// (the branch commit), not through files under the shared workspace root.
    #[test]
    fn delivery_gate_accepts_manifest_modified_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let codex = reviewer_unit("codex-refine", &["out/codex-summary-v*.md"]);
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("codex-refine", &["out/codex-summary-v2.md"])];

        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 2,
        ))
        .expect("manifest reports the commit");
    }

    #[test]
    fn delivery_gate_ignores_agents_without_owned_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let analyst = reviewer_unit("analyst", &[]);
        let unit_map: HashMap<&str, &WorkUnit> = [("analyst", &analyst)].into_iter().collect();
        let ids = vec!["analyst".to_string()];

        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("analyst", &[])];

        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 2,
        ))
        .expect("no declared deliverables to assert on");
    }

    // ── Irreconcilable disagreement ──────────────────────────────────────

    fn blocker(agent: &str, criterion: &str, conflicts_with: &str) -> JudgeBlocker {
        JudgeBlocker {
            agent: agent.into(),
            position: format!("{agent}'s position"),
            conflicts_with: conflicts_with.into(),
            criterion: criterion.into(),
            evidence_gap: "an unrun experiment".into(),
        }
    }

    /// Repeat detection has to survive the judge rephrasing itself, or a
    /// deadlock would never be recognised — models rarely emit byte-identical
    /// prose twice.
    #[test]
    fn fingerprint_ignores_wording_and_ordering_drift() {
        let a = JudgeReport {
            reason: "Panel splits on the evidence bar.".into(),
            blockers: vec![
                blocker("claude", "evidence bar", "codex"),
                blocker("codex", "evidence bar", "claude"),
            ],
        };
        let b = JudgeReport {
            reason: "Completely different wording this pass!".into(),
            blockers: vec![
                blocker("codex", "Evidence Bar", "claude"),
                blocker("claude", "evidence  bar.", "codex"),
            ],
        };
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn fingerprint_changes_when_the_dispute_moves() {
        let a = JudgeReport {
            reason: String::new(),
            blockers: vec![blocker("claude", "evidence bar", "codex")],
        };
        let b = JudgeReport {
            reason: String::new(),
            blockers: vec![blocker("claude", "stopping criteria", "codex")],
        };
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    /// With no blockers the reason is the only signal available.
    #[test]
    fn fingerprint_falls_back_to_the_reason() {
        let a = JudgeReport {
            reason: "Avenue A vs B.".into(),
            blockers: vec![],
        };
        let b = JudgeReport {
            reason: "avenue   a vs b".into(),
            blockers: vec![],
        };
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(!a.fingerprint().is_empty());
        assert!(JudgeReport::default().fingerprint().is_empty());
    }

    #[test]
    fn judge_verdict_parser_accepts_irreconcilable() {
        for text in [
            "```json\n{\"verdict\":\"irreconcilable\",\"reason\":\"x\"}\n```",
            "VERDICT: IRRECONCILABLE",
            "Deadlock.",
        ] {
            assert_eq!(
                parse_judge_verdict(text),
                Some(JudgeVerdict::Irreconcilable),
                "got none for {text:?}"
            );
        }
    }

    #[test]
    fn judge_report_parses_blockers() {
        let text = "prose\n```json\n{\"verdict\":\"irreconcilable\",\
                    \"reason\":\"values conflict\",\
                    \"blockers\":[{\"agent\":\"claude\",\"position\":\"gate on correctness\",\
                    \"conflicts_with\":\"codex\",\"criterion\":\"evidence bar\",\
                    \"evidence_gap\":\"no honest-student study\"}]}\n```";
        let report = parse_judge_report(text);
        assert_eq!(report.reason, "values conflict");
        assert_eq!(report.blockers.len(), 1);
        assert_eq!(report.blockers[0].agent, "claude");
        assert_eq!(report.blockers[0].criterion, "evidence bar");
    }

    /// A malformed or absent `blockers` array must not discard the verdict —
    /// stopping for a stated reason beats continuing because the JSON was off.
    #[test]
    fn judge_report_survives_missing_or_malformed_blockers() {
        let report = parse_judge_report("```json\n{\"verdict\":\"fail\",\"reason\":\"r\"}\n```");
        assert_eq!(report.reason, "r");
        assert!(report.blockers.is_empty());

        let junk = parse_judge_report("```json\n{\"reason\":\"r\",\"blockers\":\"nope\"}\n```");
        assert_eq!(junk.reason, "r");
        assert!(junk.blockers.is_empty());

        assert_eq!(parse_judge_report("no json here"), JudgeReport::default());
    }

    #[test]
    fn irreconcilable_report_documents_every_reviewer() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let report = JudgeReport {
            reason: "values conflict".into(),
            blockers: vec![blocker("claude", "evidence bar", "codex")],
        };
        let ids = vec![
            "claude".to_string(),
            "codex".to_string(),
            "cursor".to_string(),
        ];

        write_irreconcilable_report(root, Some("out"), 4, "test trigger", &report, &ids);

        let md = std::fs::read_to_string(root.join("out/consensus-irreconcilable.md")).unwrap();
        assert!(md.contains("### claude"));
        assert!(md.contains("claude's position"));
        assert!(md.contains("an unrun experiment"));
        // Reviewers the judge did not name still get a section — silence must
        // not read as agreement.
        assert!(md.contains("### codex"));
        assert!(md.contains("### cursor"));
        assert!(md.contains("did not attribute a blocking position"));
        assert!(md.contains("test trigger"));

        let json = std::fs::read_to_string(root.join("out/consensus-verdict.json")).unwrap();
        assert!(json.contains("\"irreconcilable\""), "got: {json}");
        assert!(json.contains("\"iter\": 4"), "got: {json}");
    }

    /// The shape a real roster run produces: the judge names reviewer ids
    /// (`claude`) because that is what the artefact filenames carry, while the
    /// loop's work units are the cloned templates (`claude-refine`). A live run
    /// filed every blocker under "Unattributed" and told the operator that
    /// neither reviewer held a blocking position — the one thing this document
    /// must never say when the judge did attribute one.
    #[test]
    fn irreconcilable_report_attributes_roster_ids_to_template_clones() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let report = JudgeReport {
            reason: "topic-vs-anchor framing deadlock".into(),
            blockers: vec![
                blocker("claude", "framing", "codex"),
                blocker("codex", "framing", "claude"),
            ],
        };
        let ids = vec!["claude-refine".to_string(), "codex-refine".to_string()];

        write_irreconcilable_report(root, Some("out"), 2, "judge verdict", &report, &ids);

        let md = std::fs::read_to_string(root.join("out/consensus-irreconcilable.md")).unwrap();
        assert!(!md.contains("Unattributed blockers"), "got: {md}");
        assert!(
            !md.contains("did not attribute a blocking position"),
            "got: {md}"
        );
        assert!(md.contains("claude's position"), "got: {md}");
        assert!(md.contains("codex's position"), "got: {md}");
    }

    /// Prefix matching must not swallow a different reviewer: `claude` is not
    /// `claude-2-refine`'s reviewer id, and a bare `-refine` matches nobody.
    #[test]
    fn blocker_attribution_requires_a_role_suffix_boundary() {
        assert!(blocker_names_agent("claude", "claude"));
        assert!(blocker_names_agent("claude", "claude-refine"));
        assert!(!blocker_names_agent("claude", "claude2-refine"));
        assert!(!blocker_names_agent("claude", "anthropic-claude-refine"));
        assert!(!blocker_names_agent("", "claude-refine"));
    }

    /// A blocker naming someone outside the roster (judge typo, hallucinated
    /// id) must still appear, not vanish into a filter.
    #[test]
    fn irreconcilable_report_keeps_unattributed_blockers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let report = JudgeReport {
            reason: String::new(),
            blockers: vec![blocker("ghost", "framing", "claude")],
        };
        write_irreconcilable_report(root, Some("out"), 2, "t", &report, &["claude".to_string()]);
        let md = std::fs::read_to_string(root.join("out/consensus-irreconcilable.md")).unwrap();
        assert!(md.contains("Unattributed blockers"), "got: {md}");
        assert!(md.contains("ghost"), "got: {md}");
    }

    /// Without an output directory there is nowhere to write; the run has
    /// already stopped, so this must not panic.
    #[test]
    fn irreconcilable_report_without_out_dir_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        write_irreconcilable_report(
            tmp.path(),
            None,
            2,
            "t",
            &JudgeReport::default(),
            &["claude".to_string()],
        );
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
    }

    /// The gate no longer inspects the `until` condition at all (D-7).
    ///
    /// `verify` / `command` conditions stay exempt, but structurally:
    /// the gate is only ever called from `evaluate_agent_condition`, so
    /// a loop those conditions decide never reaches it. What that buys
    /// is `until … and …` — a failing `command` short-circuits before
    /// the judge, and a silent agent must not abort a run whose judge
    /// was never going to be dispatched. Once called, the gate always
    /// applies; there is no condition that exempts a member of a panel
    /// that is about to be judged.
    #[test]
    fn delivery_gate_applies_whenever_it_is_reached() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let codex = reviewer_unit("codex-refine", &["out/codex-summary-v*.md"]);
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("codex-refine", &[])];

        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 2,
        ))
        .expect_err("a reviewer that wrote nothing must not be judged");
    }

    // ── H0 / PR-4: `until … and …` ordering ──────────────────────────────

    #[test]
    fn conditions_evaluate_cheapest_first_regardless_of_author_order() {
        use super::super::plan::{LoopUntilCondition as C, VerificationConfig};

        // Author order is deliberately the reverse of the cost order.
        let authored = [
            C::Agent("reviewer".into()),
            C::Command("cargo test".into()),
            C::Verify(VerificationConfig {
                compile: true,
                clippy: false,
                test: false,
                impact_tests: false,
            }),
        ];

        let mut ordered: Vec<(usize, &C)> = authored.iter().enumerate().collect();
        ordered.sort_by_key(|(_, c)| condition_rank(c));

        let ranks: Vec<u8> = ordered.iter().map(|(_, c)| condition_rank(c)).collect();
        assert_eq!(ranks, vec![0, 1, 2], "verify → command → agent");
        // The original slots travel with the conditions, so dedup memos
        // stay attached to the condition the author wrote.
        assert_eq!(
            ordered.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
    }

    #[test]
    fn equal_cost_conditions_keep_author_order() {
        use super::super::plan::LoopUntilCondition as C;

        let authored = [C::Command("first".into()), C::Command("second".into())];
        let mut ordered: Vec<(usize, &C)> = authored.iter().enumerate().collect();
        ordered.sort_by_key(|(_, c)| condition_rank(c));

        assert_eq!(
            ordered.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1],
            "the sort must be stable"
        );
    }

    #[test]
    fn judge_agent_is_found_inside_a_composition() {
        use super::super::plan::LoopUntilCondition as C;

        let composed = C::All(vec![
            C::Command("cargo test".into()),
            C::Agent("reviewer".into()),
        ]);
        assert_eq!(composed.judge_agent(), Some("reviewer"));
        assert_eq!(C::Command("x".into()).judge_agent(), None);
    }

    // ── Total-failure guard (condition-independent) ──────────────────────

    fn failed(id: &str, why: &str) -> AgentManifest {
        AgentManifest {
            work_unit_id: id.into(),
            status: AgentStatus::Failed(why.into()),
            modified_files: vec![],
            branch: None,
            summary: Some(why.into()),
            output: None,
            cost_usd: 0.0,
        }
    }

    #[test]
    fn a_pass_where_every_agent_failed_stops_the_loop() {
        let ids = vec!["alpha".to_string(), "beta".to_string()];
        let manifests = vec![
            failed("alpha", "backend resolution failed"),
            failed("beta", "backend resolution failed"),
        ];

        let err = assert_loop_pass_was_not_a_total_failure(&ids, &manifests, 2)
            .expect_err("a unanimously failed panel cannot be fixed by another round");

        let msg = format!("{err:#}");
        assert!(msg.contains("all 2"), "got {msg}");
        assert!(msg.contains("backend resolution failed"), "got {msg}");
    }

    #[test]
    fn the_total_failure_guard_ignores_the_until_condition() {
        // This is the whole point: unlike the delivery gate, a broken
        // panel is caught under `until command` too.
        let ids = vec!["alpha".to_string()];
        let manifests = vec![failed("alpha", "spawn error")];

        assert!(assert_loop_pass_was_not_a_total_failure(&ids, &manifests, 1).is_err());
    }

    #[test]
    fn a_partially_failing_panel_keeps_going() {
        // One agent still delivered, so the panel can make progress and
        // the delivery gate / judge decide what that is worth.
        let ids = vec!["alpha".to_string(), "beta".to_string()];
        let manifests = vec![failed("alpha", "timeout"), completed("beta", &["out/b.md"])];

        assert_loop_pass_was_not_a_total_failure(&ids, &manifests, 2)
            .expect("a partial failure is not unambiguous");
    }

    #[test]
    fn an_agent_that_never_reported_is_left_to_the_delivery_gate() {
        // No manifest at all means "never dispatched", which the
        // delivery gate diagnoses far better than we could here.
        let ids = vec!["alpha".to_string(), "ghost".to_string()];
        let manifests = vec![failed("alpha", "timeout")];

        assert_loop_pass_was_not_a_total_failure(&ids, &manifests, 2)
            .expect("an absent manifest is not this guard's business");
    }

    #[test]
    fn the_total_failure_guard_uses_the_most_recent_pass() {
        // `all_manifests` accumulates across iterations; a failure from
        // an earlier pass must not condemn a later successful one.
        let ids = vec!["alpha".to_string()];
        let manifests = vec![
            failed("alpha", "transient"),
            completed("alpha", &["out/a.md"]),
        ];

        assert_loop_pass_was_not_a_total_failure(&ids, &manifests, 3)
            .expect("the latest pass succeeded");
    }

    #[test]
    fn an_empty_panel_is_not_a_failure() {
        assert_loop_pass_was_not_a_total_failure(&[], &[], 1).expect("nothing to judge");
    }

    // ── Declared output contract (`agent { produces [...] }`) ────────────

    fn producing_unit(id: &str, owned: &[&str], produces: &[&str]) -> WorkUnit {
        let mut u = reviewer_unit(id, owned);
        u.produces = produces.iter().map(|s| s.to_string()).collect();
        u
    }

    /// The contract is versioned: `{{ITER}}` is substituted for the pass being
    /// judged, so writing the previous version is a failure, not a pass. The
    /// snapshot heuristic alone cannot see this — the owned file set did change.
    #[test]
    fn declared_contract_rejects_the_wrong_iteration() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();

        let codex = producing_unit(
            "codex-refine",
            &["out/codex-summary-v*.md"],
            &["out/codex-summary-v{{ITER}}.md"],
        );
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        let before = snapshot_loop_agents(&ids, &unit_map, root);
        // Wrote v3 when the pass being judged is v4.
        std::fs::write(root.join("out/codex-summary-v3.md"), "stale").unwrap();
        let after = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("codex-refine", &[])];

        let err = assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &before, &after, root, 4,
        ))
        .expect_err("v4 was never written");
        let msg = format!("{err:#}");
        assert!(msg.contains("out/codex-summary-v4.md"), "got: {msg}");
        assert!(msg.contains("missing"), "got: {msg}");
    }

    #[test]
    fn declared_contract_passes_when_every_path_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();

        let codex = producing_unit(
            "codex-refine",
            &["out/codex-*-v*.md"],
            &[
                "out/codex-conclusion-v{{ITER}}.md",
                "out/codex-summary-v{{ITER}}.md",
            ],
        );
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        std::fs::write(root.join("out/codex-conclusion-v4.md"), "c").unwrap();
        std::fs::write(root.join("out/codex-summary-v4.md"), "s").unwrap();
        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("codex-refine", &[])];

        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 4,
        ))
        .expect("both declared artefacts exist");
    }

    /// A zero-byte artefact satisfies a naive existence check while carrying
    /// no information — the judge would read an empty consensus summary.
    #[test]
    fn declared_contract_rejects_an_empty_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("out")).unwrap();

        let codex = producing_unit(
            "codex-refine",
            &["out/codex-summary-v*.md"],
            &["out/codex-summary-v{{ITER}}.md"],
        );
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        std::fs::write(root.join("out/codex-summary-v4.md"), "").unwrap();
        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        let manifests = vec![completed("codex-refine", &[])];

        let err = assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 4,
        ))
        .expect_err("empty file is not a deliverable");
        assert!(
            format!("{err:#}").contains("out/codex-summary-v4.md"),
            "got: {err:#}"
        );
    }

    /// A declared contract is authoritative: a non-empty `modified_files`
    /// must not excuse a missing artefact the way it does for the fallback.
    #[test]
    fn declared_contract_overrides_the_manifest_shortcut() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let codex = producing_unit(
            "codex-refine",
            &["out/codex-summary-v*.md"],
            &["out/codex-summary-v{{ITER}}.md"],
        );
        let unit_map: HashMap<&str, &WorkUnit> = [("codex-refine", &codex)].into_iter().collect();
        let ids = vec!["codex-refine".to_string()];

        let snap = snapshot_loop_agents(&ids, &unit_map, root);
        // The agent touched *something*, just not what it promised.
        let manifests = vec![completed("codex-refine", &["out/scratch.md"])];

        assert_loop_agents_produced_output(delivery_check(
            &ids, &unit_map, &manifests, &snap, &snap, root, 4,
        ))
        .expect_err("declared artefact is still missing");
    }
}

/// Collect a list of git-tracked files in the workspace for coordinator context.
///
/// Uses `git ls-files` so the coordinator only sees files that actually exist in
/// agent worktrees (which are plain git checkouts). Gitignored and untracked files
/// are excluded, preventing the coordinator from telling agents to read files they
/// cannot access.
fn collect_file_list(workspace_root: &PathBuf) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(workspace_root)
        .output()
        .context("failed to run git ls-files")?;

    if !output.status.success() {
        // Not a git repo or other error — fall back to empty list rather than fail
        tracing::warn!(
            "git ls-files failed in {:?}, coordinator will have no file list",
            workspace_root
        );
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

#[cfg(test)]
mod collect_file_list_tests {
    use super::collect_file_list;
    use tempfile::tempdir;

    #[test]
    fn collect_file_list_returns_tracked_files_only() {
        let dir = tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");

        std::fs::write(dir.path().join("tracked.txt"), "tracked").unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "untracked").unwrap();

        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .expect("git add");

        let files = collect_file_list(&dir.path().to_path_buf()).unwrap();

        assert_eq!(files, vec!["tracked.txt"]);
    }

    #[test]
    fn collect_file_list_falls_back_to_empty_for_non_git_directory() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("plain.txt"), "content").unwrap();

        let files = collect_file_list(&dir.path().to_path_buf()).unwrap();

        assert!(files.is_empty());
    }
}

/// Invalidate stale memory entries for a work unit's `staleness_sources`.
///
/// Called immediately before each agent runs. Any memory entry whose
/// `source_file` hash has changed since it was stored gets `importance = 0.0`,
/// making it effectively invisible to semantic search.
///
/// Best-effort: errors are logged but never propagate to the caller.
async fn invalidate_stale_sources(
    memory: &Option<Arc<MemoryStores>>,
    unit: &WorkUnit,
    workspace_root: &std::path::Path,
) {
    let Some(mem) = memory else { return };
    if unit.staleness_sources.is_empty() {
        return;
    };

    let paths: Vec<std::path::PathBuf> = unit
        .staleness_sources
        .iter()
        .map(|s| workspace_root.join(s))
        .collect();

    match mem.workspace().check_staleness(&paths).await {
        Ok(stale) if !stale.is_empty() => {
            let ids: Vec<i64> = stale.iter().map(|(id, _, _, _)| *id).collect();
            tracing::info!(
                "Invalidating {} stale memory entries before running agent '{}'",
                ids.len(),
                unit.id
            );
            if let Err(e) = mem.workspace().mark_stale(&ids).await {
                tracing::warn!("mark_stale failed for agent '{}': {}", unit.id, e);
            }
        }
        Ok(_) => {} // nothing stale
        Err(e) => {
            tracing::warn!("check_staleness failed for agent '{}': {}", unit.id, e);
        }
    }
}

/// Cheapest-first ordering for `until … and …`.
///
/// A judge is an LLM call; a `verify` block is a local build. Evaluating
/// in cost order and short-circuiting means a panel is never paid for
/// when a compile already says the iteration is not done — regardless of
/// the order the script author happened to write the conditions in.
fn condition_rank(condition: &super::plan::LoopUntilCondition) -> u8 {
    match condition {
        super::plan::LoopUntilCondition::Verify(_) => 0,
        super::plan::LoopUntilCondition::Command(_) => 1,
        super::plan::LoopUntilCondition::Agent(_) => 2,
        // The parser flattens nested `All`s, so this is unreachable in
        // practice; rank it last rather than assuming.
        super::plan::LoopUntilCondition::All(_) => 3,
    }
}

/// Run a `verify {}` loop condition. `Ok(None)` means it passed.
///
/// `index` is the condition's slot in its enclosing `All`, which keys
/// the dedup memo — two `verify` blocks in one composition must not
/// share one.
async fn evaluate_verify_condition(
    config: &super::plan::VerificationConfig,
    index: usize,
    ctx: &mut LoopConditionContext<'_>,
) -> Result<Option<GateFailure>> {
    // Bind the config reference out of `ctx` so the closure below
    // borrows the SwarmConfig, not the context we mutate.
    let cfg = ctx.config;

    let failure = evaluate_deterministic_gate(
        ctx.probe_dedup,
        index,
        VERIFY_DEDUP_KEY,
        &cfg.workspace_root,
        || async {
            // A verify block that cannot be run at all is a broken
            // gate, not a failing one — propagate instead of reading
            // it as "not converged yet" and burning every remaining
            // iteration.
            let outcome = run_verification_checks(config, &cfg.workspace_root, &cfg.excludes, None)
                .await
                .context("loop `until` verification checks could not be run")?;

            Ok(match outcome {
                VerificationOutcome::Passed => None,
                VerificationOutcome::Failed { check, output } => Some(GateFailure {
                    probe: check,
                    status: "verification check failed".to_string(),
                    output,
                }),
            })
        },
    )
    .await?;

    Ok(failure)
}

/// Run an `until command` probe. `Ok(None)` means it passed.
async fn evaluate_command_condition(
    cmd: &str,
    index: usize,
    current_iter_abs: u32,
    ctx: &mut LoopConditionContext<'_>,
) -> Result<Option<GateFailure>> {
    // Substitute {{ITER}}/{{PREV_ITER}} so iteration-aware probes
    // (e.g. `git show gaviero/foo-iter{{ITER}}:path/file.md`) can be
    // expressed without going through an LLM judge.
    let iter_str = current_iter_abs.to_string();
    let prev_str = current_iter_abs.saturating_sub(1).to_string();
    let expanded = cmd
        .replace("{{ITER}}", &iter_str)
        .replace("{{PREV_ITER}}", &prev_str);
    let cfg = ctx.config;

    // The expanded command is part of the dedup key: an
    // iteration-aware probe addresses a different target each
    // pass, so an unchanged workspace does not make it redundant.
    let failure = evaluate_deterministic_gate(
        ctx.probe_dedup,
        index,
        &expanded,
        &cfg.workspace_root,
        || run_command_probe(&expanded, &cfg.workspace_root),
    )
    .await?;

    Ok(failure)
}

/// Dispatch the judge agent and turn its verdict into a loop outcome.
///
/// The delivery gate runs here rather than in the loop head: it exists
/// to stop a judge rendering a verdict on a panel that silently lost a
/// member, so it is a precondition of *judging*, not of iterating. Under
/// `until … and …` a cheaper condition may fail first and the judge is
/// never reached — aborting the run for a silent agent in that case
/// would kill a run the loop could still have recovered (D-7).
async fn evaluate_agent_condition(
    agent_id: &str,
    current_iter_abs: u32,
    ctx: &mut LoopConditionContext<'_>,
) -> Result<LoopConditionOutcome> {
    assert_loop_agents_produced_output(DeliveryCheck {
        agent_ids: ctx.loop_agent_ids,
        unit_map: ctx.delivery.unit_map,
        all_manifests: ctx.all_manifests,
        before: ctx.delivery.before,
        after: ctx.delivery.after,
        workspace_root: &ctx.config.workspace_root,
        iter_abs: current_iter_abs,
    })?;

    let agent_id = &agent_id.to_string();
    let Some(unit_template) = ctx.loop_judge_map.get(agent_id.as_str()).copied() else {
        tracing::warn!(
            "loop judge agent '{}' not found in compiled plan (judges must be declared distinct from workflow agents)",
            agent_id
        );
        return Ok(LoopConditionOutcome::Continue(JudgeReport::default()));
    };

    // Build a compact digest of the most recent worker manifests for
    // this loop, substituted into `{{ITER_EVIDENCE}}` if the judge's
    // `coordinator_instructions` template references it. Authors who
    // already supply their own evidence text (or omit the placeholder)
    // are unaffected — the placeholder is only replaced when present.
    let evidence = if unit_template
        .coordinator_instructions
        .contains("{{ITER_EVIDENCE}}")
    {
        build_iter_evidence(ctx.all_manifests, ctx.loop_agent_ids, current_iter_abs)
    } else {
        String::new()
    };
    let unit = apply_iter_vars_with_evidence(unit_template, current_iter_abs, &evidence);
    invalidate_stale_sources(ctx.memory, &unit, &ctx.config.workspace_root).await;

    let effective_read_ns = effective_read_namespaces(&unit, ctx.config, ctx.memory);
    let analysis = WorkspaceAnalysis {
        repo_map: ctx.repo_map.clone(),
        impact_texts: ctx.impact_texts.clone(),
    };
    let agent_ctx = AgentRunContext::for_run(
        ctx.config,
        ctx.context_files,
        &effective_read_ns,
        ctx.observer,
        ctx.memory.clone(),
        ctx.git_coordinator.clone(),
        ctx.validation.clone(),
        Some(ctx.shared_board.clone()),
        &analysis,
        ctx.pre_fetched_memory.clone(),
    );

    // Judges run in read-only mode: the write gate rejects any write
    // proposals the backend tries to emit. See `run_readonly_agent`.
    let run_future = run_readonly_agent(
        &unit,
        &agent_ctx,
        ctx.tier_router,
        ctx.iteration_config,
        (ctx.make_observer)(agent_id),
    );

    // Apply judge timeout if configured (0 = disabled).
    let manifest_result = if ctx.judge_timeout_secs > 0 {
        match tokio::time::timeout(
            std::time::Duration::from_secs(ctx.judge_timeout_secs as u64),
            run_future,
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!(
                "judge agent '{}' timed out after {}s",
                agent_id,
                ctx.judge_timeout_secs
            )),
        }
    } else {
        run_future.await
    };

    let mut manifest = match manifest_result {
        Ok(manifest) => manifest,
        Err(e) => AgentManifest {
            work_unit_id: agent_id.clone(),
            status: AgentStatus::Failed(e.to_string()),
            modified_files: vec![],
            branch: None,
            summary: Some(format!("Judge evaluation error: {}", e)),
            output: None,
            cost_usd: 0.0,
        },
    };

    if !manifest.modified_files.is_empty() {
        tracing::warn!(
            "loop judge agent '{}' modified files during evaluation: {:?}",
            agent_id,
            manifest.modified_files
        );
    }

    let verdict = manifest.output.as_deref().and_then(parse_judge_verdict);
    let report = manifest
        .output
        .as_deref()
        .map(parse_judge_report)
        .unwrap_or_default();
    manifest.summary = Some(match (verdict, &manifest.status) {
        (Some(JudgeVerdict::Pass), _) => "Judge verdict: PASS".into(),
        (Some(JudgeVerdict::Fail), _) => "Judge verdict: FAIL".into(),
        (Some(JudgeVerdict::Partial), _) => "Judge verdict: PARTIAL".into(),
        (Some(JudgeVerdict::Irreconcilable), _) => "Judge verdict: IRRECONCILABLE".into(),
        (None, AgentStatus::Failed(msg)) => format!("Judge failed: {}", msg),
        (None, _) => "Judge verdict: unparseable".into(),
    });

    // Under strict mode, an unparseable verdict on an otherwise completed
    // run is promoted to a hard failure so it surfaces in the manifest/UI
    // instead of silently being treated as FAIL.
    if verdict.is_none() && matches!(manifest.status, AgentStatus::Completed) {
        if ctx.strict_judge {
            tracing::error!(
                "loop judge agent '{}' returned unparseable output (strict mode)",
                agent_id
            );
            manifest.status = AgentStatus::Failed(
                    "judge returned unparseable verdict (enable strict_judge=false for legacy behaviour)"
                        .into(),
                );
        } else {
            tracing::warn!(
                "loop judge agent '{}' completed without a parseable PASS/FAIL verdict",
                agent_id
            );
        }
    }

    if matches!(manifest.status, AgentStatus::Completed) {
        {
            let b = ctx.bus.lock().await;
            b.broadcast(
                &manifest.work_unit_id,
                &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
            );
        }
        let worker_ns = unit
            .write_namespace
            .as_deref()
            .unwrap_or(&ctx.config.write_namespace);
        // Route judge artefacts to a dedicated sub-namespace so they do
        // not pollute worker memory. The store's namespace is treated as
        // an opaque key by callers, so the `judge/` prefix is a pure
        // convention the consolidator and TUI can key off.
        let judge_ns = format!("judge/{}", worker_ns);
        store_agent_result(
            ctx.memory,
            ctx.memory_writer,
            &judge_ns,
            &manifest,
            &unit,
            ctx.run_id,
            &ctx.config.workspace_root,
            ctx.config.extract_agent_findings,
        )
        .await;
    }

    let outcome = match verdict {
        Some(JudgeVerdict::Pass) => LoopConditionOutcome::Pass,
        // The judge itself ruled the disagreement structural. Honour
        // it in every consensus mode: continuing would spend the
        // remaining budget re-deriving a conclusion the judge has
        // already read and rejected as unreachable.
        Some(JudgeVerdict::Irreconcilable) => LoopConditionOutcome::Irreconcilable(report),
        Some(JudgeVerdict::Partial)
            if ctx.consensus_mode == crate::swarm::plan::ConsensusMode::PartialOk =>
        {
            if let Some(dir) = ctx.verdict_output_dir {
                write_consensus_verdict_file(
                    &ctx.config.workspace_root,
                    dir,
                    current_iter_abs,
                    "partial",
                    manifest.summary.as_deref().unwrap_or(""),
                    &ctx.loop_agent_ids,
                );
            }
            LoopConditionOutcome::Partial
        }
        Some(JudgeVerdict::Partial) => LoopConditionOutcome::Continue(report),
        _ => LoopConditionOutcome::Continue(report),
    };

    ctx.all_manifests.push(manifest);
    Ok(outcome)
}

/// Evaluate `until … and …`: every condition must pass.
///
/// Conditions run cheapest-first and short-circuit on the first that
/// does not pass, so the expensive judge is only consulted once the
/// deterministic conditions already agree.
async fn evaluate_all_conditions(
    conditions: &[super::plan::LoopUntilCondition],
    current_iter_abs: u32,
    ctx: &mut LoopConditionContext<'_>,
) -> Result<(LoopConditionOutcome, Option<GateFailure>)> {
    let mut ordered: Vec<(usize, &super::plan::LoopUntilCondition)> =
        conditions.iter().enumerate().collect();
    // Stable, so author order still decides between conditions of equal
    // cost.
    ordered.sort_by_key(|(_, c)| condition_rank(c));

    for (index, condition) in ordered {
        match condition {
            super::plan::LoopUntilCondition::Verify(config) => {
                if let Some(failure) = evaluate_verify_condition(config, index, ctx).await? {
                    let observer = ctx.observer;
                    return Ok(report_deterministic_gate(
                        Some(failure),
                        current_iter_abs,
                        observer,
                    ));
                }
            }
            super::plan::LoopUntilCondition::Command(cmd) => {
                if let Some(failure) =
                    evaluate_command_condition(cmd, index, current_iter_abs, ctx).await?
                {
                    let observer = ctx.observer;
                    return Ok(report_deterministic_gate(
                        Some(failure),
                        current_iter_abs,
                        observer,
                    ));
                }
            }
            super::plan::LoopUntilCondition::Agent(agent_id) => {
                let outcome = evaluate_agent_condition(agent_id, current_iter_abs, ctx).await?;
                if !matches!(outcome, LoopConditionOutcome::Pass) {
                    return Ok((outcome, None));
                }
            }
            // Flattened by the parser; nothing sensible to do but skip.
            super::plan::LoopUntilCondition::All(_) => {}
        }
    }

    Ok((LoopConditionOutcome::Pass, None))
}

/// Evaluate a loop's exit condition.
async fn evaluate_loop_condition(
    condition: &super::plan::LoopUntilCondition,
    current_iter_abs: u32,
    ctx: &mut LoopConditionContext<'_>,
) -> Result<(LoopConditionOutcome, Option<GateFailure>)> {
    if ctx.consensus_mode == crate::swarm::plan::ConsensusMode::Explore {
        return Ok((LoopConditionOutcome::Continue(JudgeReport::default()), None));
    }
    match condition {
        super::plan::LoopUntilCondition::Verify(config) => {
            let failure = evaluate_verify_condition(config, 0, ctx).await?;
            let observer = ctx.observer;
            Ok(report_deterministic_gate(
                failure,
                current_iter_abs,
                observer,
            ))
        }
        super::plan::LoopUntilCondition::Command(cmd) => {
            let failure = evaluate_command_condition(cmd, 0, current_iter_abs, ctx).await?;
            let observer = ctx.observer;
            Ok(report_deterministic_gate(
                failure,
                current_iter_abs,
                observer,
            ))
        }
        super::plan::LoopUntilCondition::Agent(agent_id) => {
            let outcome = evaluate_agent_condition(agent_id, current_iter_abs, ctx).await?;
            Ok((outcome, None))
        }
        super::plan::LoopUntilCondition::All(conditions) => {
            evaluate_all_conditions(conditions, current_iter_abs, ctx).await
        }
    }
}

/// Does a judge-reported `agent` name this loop work unit?
///
/// The judge is asked for the *reviewer id* as it appears in the artefact
/// filenames (`claude`), but the loop's work units are the roster's cloned
/// templates (`claude-refine`). Matching on equality alone dropped every
/// blocker of a real run into "Unattributed" and reported each reviewer as
/// unnamed — the exact ambiguity `consensus-irreconcilable.md` exists to
/// remove. Roster clones are always `<reviewer-id>-<template-role>`
/// (`workflow_params.rs`), so the prefix is the reviewer id.
fn blocker_names_agent(blocker_agent: &str, agent_id: &str) -> bool {
    if blocker_agent.is_empty() {
        return false;
    }
    agent_id == blocker_agent
        || agent_id
            .strip_prefix(blocker_agent)
            .is_some_and(|role| role.starts_with('-'))
}

/// Render the irreconcilable-disagreement hand-off.
///
/// Writes `consensus-irreconcilable.md` (one section per reviewer, so a human
/// can see which position each one holds and what evidence would settle it)
/// plus the machine-readable `consensus-verdict.json` the partial path already
/// emits. Reviewers the judge did not name still get a section saying so —
/// a silent omission would read as "this reviewer agreed", which is exactly
/// the ambiguity the document exists to remove.
///
/// Best-effort: the run has already stopped, and failing to write the report
/// must not mask the reason it stopped.
fn write_irreconcilable_report(
    workspace_root: &std::path::Path,
    out_dir: Option<&str>,
    iter_abs: u32,
    trigger: &str,
    report: &JudgeReport,
    reviewer_ids: &[String],
) {
    let Some(out_dir) = out_dir else {
        tracing::warn!(
            "irreconcilable disagreement at iteration {iter_abs} ({trigger}), but the loop has no \
             verdict output directory — skipping the hand-off document. Reason: {}",
            report.reason
        );
        return;
    };

    let mut md = String::with_capacity(2048);
    md.push_str("# Consensus not reachable\n\n");
    md.push_str(&format!(
        "The panel stopped at iteration {iter_abs} because the disagreement is structural, \
         not because it ran out of iterations.\n\n\
         - **Trigger:** {trigger}\n\
         - **Judge's reason:** {}\n\n",
        if report.reason.is_empty() {
            "(the judge gave no reason)"
        } else {
            report.reason.as_str()
        }
    ));

    md.push_str("## Positions by reviewer\n\n");
    for id in reviewer_ids {
        md.push_str(&format!("### {id}\n\n"));
        let mine: Vec<&JudgeBlocker> = report
            .blockers
            .iter()
            .filter(|b| blocker_names_agent(&b.agent, id))
            .collect();
        if mine.is_empty() {
            md.push_str(
                "The judge did not attribute a blocking position to this reviewer. That is not \
                 the same as agreement — read this reviewer's latest `*-summary-v*.md` \
                 (`## Substantive disagreements`) before assuming it was aligned.\n\n",
            );
            continue;
        }
        for b in mine {
            if !b.position.is_empty() {
                md.push_str(&format!("- **Holds:** {}\n", b.position));
            }
            if !b.conflicts_with.is_empty() {
                md.push_str(&format!("- **Conflicts with:** {}\n", b.conflicts_with));
            }
            if !b.criterion.is_empty() {
                md.push_str(&format!("- **Convergence criterion:** {}\n", b.criterion));
            }
            if !b.evidence_gap.is_empty() {
                md.push_str(&format!("- **Would be settled by:** {}\n", b.evidence_gap));
            }
            md.push('\n');
        }
    }

    // Blockers naming a reviewer outside the roster would otherwise vanish.
    let orphans: Vec<&JudgeBlocker> = report
        .blockers
        .iter()
        .filter(|b| {
            !reviewer_ids
                .iter()
                .any(|id| blocker_names_agent(&b.agent, id))
        })
        .collect();
    if !orphans.is_empty() {
        md.push_str("## Unattributed blockers\n\n");
        for b in orphans {
            md.push_str(&format!(
                "- ({}) {} — conflicts with {}\n",
                if b.agent.is_empty() { "?" } else { &b.agent },
                b.position,
                b.conflicts_with
            ));
        }
        md.push('\n');
    }

    md.push_str(
        "## What to do next\n\n\
         Nothing further will change by re-running the same panel on the same anchor: it has \
         restated this disagreement rather than closing it. Either resolve the conflict outside \
         the panel (gather the evidence named above, or amend the anchor so the question is \
         decidable), or accept the competing conclusions as the deliverable and choose between \
         them yourself.\n",
    );

    let dir = workspace_root.join(out_dir);
    let _ = std::fs::create_dir_all(&dir);
    let md_path = dir.join("consensus-irreconcilable.md");
    match std::fs::write(&md_path, md) {
        Ok(()) => tracing::info!("Wrote irreconcilable report to {}", md_path.display()),
        Err(e) => tracing::warn!("Failed to write {}: {}", md_path.display(), e),
    }

    write_consensus_verdict_file(
        workspace_root,
        out_dir,
        iter_abs,
        "irreconcilable",
        &report.reason,
        reviewer_ids,
    );
}

fn write_consensus_verdict_file(
    workspace_root: &std::path::Path,
    out_dir: &str,
    iter_abs: u32,
    verdict: &str,
    reason: &str,
    reviewer_ids: &[String],
) {
    let path = workspace_root.join(out_dir).join("consensus-verdict.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::json!({
        "verdict": verdict,
        "iter": iter_abs,
        "reason": reason,
        "reviewers": reviewer_ids,
    });
    if let Ok(serialized) = serde_json::to_string_pretty(&body) {
        let _ = std::fs::write(path, serialized);
    }
}

fn parse_judge_verdict(text: &str) -> Option<JudgeVerdict> {
    // Subprocess agents (notably Claude Code) append a
    // `<turn_annotations>{…}</turn_annotations>` sidecar after every reply.
    // The sidecar JSON contains literal "decision"/"importance" tokens which
    // are not the judge's verdict and which can shadow the trailing fenced
    // block the prompt asks for. Strip these blocks before parsing so the
    // verdict-shaped output the prompt requested is what the parser sees.
    let stripped = strip_turn_annotations(text);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. ```json ... ``` fenced block (most reliable).
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(fenced.trim()) {
            if let Some(verdict) = parse_judge_verdict_json(&value) {
                return Some(verdict);
            }
        }
    }

    // 2. Whole text is raw JSON.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(verdict) = parse_judge_verdict_json(&value) {
            return Some(verdict);
        }
    }

    // 3. Line scan, last-to-first: VERDICT-style line wins over incidental tokens.
    trimmed.lines().rev().find_map(parse_judge_verdict_line)
}

/// Remove every `<turn_annotations>...</turn_annotations>` block from `text`,
/// returning a borrowed view when nothing was stripped and an owned `String`
/// when at least one block was present. The sidecar is editor metadata, not
/// part of the verdict the judge prompt asked for.
fn strip_turn_annotations(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("<turn_annotations>") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("<turn_annotations>") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open..];
        if let Some(close) = after_open.find("</turn_annotations>") {
            rest = &after_open[close + "</turn_annotations>".len()..];
        } else {
            // Unterminated tag — drop the rest as it's clearly the sidecar
            // and not the verdict.
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// Extract the contents of the first ```json … ``` or ``` … ``` fenced block
/// in `text`, if any. Used as a resilience layer — LLMs often wrap JSON in a
/// fenced block surrounded by prose.
fn extract_fenced_json(text: &str) -> Option<&str> {
    let start = text.find("```")?;
    let after_open = &text[start + 3..];
    // Skip an optional language tag like "json\n".
    let body = after_open
        .split_once('\n')
        .map(|(first, rest)| {
            if first.trim().eq_ignore_ascii_case("json") || first.trim().is_empty() {
                rest
            } else {
                after_open
            }
        })
        .unwrap_or(after_open);
    let end = body.find("```")?;
    Some(&body[..end])
}

fn parse_judge_verdict_json(value: &serde_json::Value) -> Option<JudgeVerdict> {
    let obj = value.as_object()?;

    for key in ["pass", "passed", "ok"] {
        if let Some(flag) = obj.get(key).and_then(|v| v.as_bool()) {
            return Some(if flag {
                JudgeVerdict::Pass
            } else {
                JudgeVerdict::Fail
            });
        }
    }

    for key in ["verdict", "decision", "result", "status"] {
        if let Some(text) = obj.get(key).and_then(|v| v.as_str()) {
            if let Some(verdict) = parse_judge_token(text) {
                return Some(verdict);
            }
        }
    }

    None
}

fn parse_judge_verdict_line(line: &str) -> Option<JudgeVerdict> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed
        .trim_matches(|c: char| matches!(c, '`' | '*' | '#' | '_' | '>' | '-'))
        .trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(verdict) = parse_judge_token(normalized) {
        return Some(verdict);
    }

    for prefix in ["FINAL VERDICT", "VERDICT", "RESULT", "DECISION"] {
        if normalized.is_char_boundary(prefix.len())
            && normalized[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            let rest = normalized[prefix.len()..]
                .trim_start_matches(|c: char| c == ':' || c == '-' || c == '—' || c.is_whitespace())
                .trim();
            if let Some(verdict) = parse_judge_token(rest) {
                return Some(verdict);
            }
        }
    }

    None
}

fn parse_judge_token(token: &str) -> Option<JudgeVerdict> {
    // Consume the leading alphabetic run (e.g. "PASS." → "PASS",
    // "**FAIL**" → "FAIL" after outer trim, "APPROVED: …" → "APPROVED").
    let trimmed = token.trim();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if head.is_empty() {
        return None;
    }
    // Keep the accepted set small and documented.
    match head.to_ascii_uppercase().as_str() {
        "PASS" | "PASSED" | "APPROVED" | "OK" | "LGTM" | "CONVERGED" | "DONE" => {
            Some(JudgeVerdict::Pass)
        }
        "FAIL" | "FAILED" | "REJECTED" | "REJECT" => Some(JudgeVerdict::Fail),
        "PARTIAL" | "PARTIALLY" => Some(JudgeVerdict::Partial),
        // `irreconcilable` is the whole token; `deadlock`/`impasse` are the
        // words models reach for when asked to name a structural disagreement.
        "IRRECONCILABLE" | "DEADLOCK" | "DEADLOCKED" | "IMPASSE" => {
            Some(JudgeVerdict::Irreconcilable)
        }
        _ => None,
    }
}

/// Extract the judge's `reason` and `blockers` from whatever JSON it emitted.
///
/// Independent of verdict parsing: a judge that returns FAIL with blockers is
/// still telling the runtime what the deadlock is, which is what repeat
/// detection fingerprints. Returns an empty report when nothing parses — the
/// verdict alone is still actionable.
fn parse_judge_report(text: &str) -> JudgeReport {
    let stripped = strip_turn_annotations(text);
    let trimmed = stripped.trim();
    let value = extract_fenced_json(trimmed)
        .and_then(|f| serde_json::from_str::<serde_json::Value>(f.trim()).ok())
        .or_else(|| serde_json::from_str::<serde_json::Value>(trimmed).ok());

    let Some(obj) = value.as_ref().and_then(|v| v.as_object()) else {
        return JudgeReport::default();
    };
    let reason = ["reason", "explanation", "summary", "detail"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    let blockers = ["blockers", "disagreements", "conflicts"]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(|v| serde_json::from_value::<Vec<JudgeBlocker>>(v.clone()).ok())
        .unwrap_or_default();
    JudgeReport { reason, blockers }
}

/// No-op write gate observer for parallel agents (AutoAccept mode).
struct NoopWriteGateObserver;

impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}
