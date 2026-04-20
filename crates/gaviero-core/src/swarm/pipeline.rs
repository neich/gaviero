//! Swarm pipeline: validates → tiers → parallel execution → merge.
//!
//! Orchestrates multi-agent execution with git worktree isolation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, Semaphore};

use super::board::SharedBoard;
use super::bus::AgentBus;
use super::context_bundle::build_bundle;
use super::coordinator::{Coordinator, CoordinatorConfig};
use super::execution_state::{ExecutionState, NodeStatus};
use super::merge;
use super::models::{AgentManifest, AgentStatus, MergeResult, SwarmResult, WorkUnit};
use super::plan::CompiledPlan;
use super::router::{TierConfig, TierRouter};
use super::validation;
use crate::git::{GitCoordinator, WorktreeManager};
use crate::memory::store::file_hash;
use crate::memory::{MemoryStore, StoreOptions};
use crate::observer::{AcpObserver, SwarmObserver};
use crate::types::{EntryMetadata, PrivacyLevel};
use crate::write_gate::{WriteGatePipeline, WriteMode};

/// Configuration for a swarm execution.
pub struct SwarmConfig {
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
    /// Folder names or glob patterns to skip when scanning the workspace for
    /// repo-map / code-graph building. Bare names (no `/`) match any directory
    /// basename; entries with `/` are glob-matched against workspace-relative
    /// paths (see [`crate::path_pattern::matches`]).
    pub excludes: Vec<String>,
}

/// Execute a swarm of work units from a compiled plan.
///
/// 1. Extract work units from plan graph (topological order)
/// 2. Validate scopes (no overlaps)
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
    memory: Option<Arc<MemoryStore>>,
    observer: &dyn SwarmObserver,
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver> + Send + Sync,
) -> Result<SwarmResult> {
    tracing::info!(
        agents = plan.graph.node_count(),
        max_parallel = config.max_parallel,
        "swarm.execute starting"
    );

    // Extract work units in topological order from the plan graph
    let work_units = plan
        .work_units_ordered()
        .map_err(|e| anyhow::anyhow!("plan graph error: {}", e))?;

    // Override max_parallel from plan if declared
    let effective_max_parallel = plan.max_parallel.unwrap_or(config.max_parallel);
    let mut tier_config = TierConfig::default();
    let selected_local_model = config
        .model
        .strip_prefix("ollama:")
        .or_else(|| config.model.strip_prefix("local:"))
        .map(str::to_string);
    if let Some(base_url) = config.ollama_base_url.as_ref() {
        tier_config.local.base_url = base_url.clone();
    }
    if let Some(local_model) = selected_local_model.as_ref() {
        tier_config.local.enabled = true;
        tier_config.local.model = local_model.clone();
        tier_config.cheap_model = local_model.clone();
        tier_config.expensive_model = local_model.clone();
    } else if crate::swarm::backend::shared::is_codex_model(&config.model) {
        // Codex is API-backed like Claude. Propagate to both tier defaults so
        // work units without an explicit `model` override stay on Codex.
        tier_config.cheap_model = config.model.clone();
        tier_config.expensive_model = config.model.clone();
    }
    let tier_router = TierRouter::new(tier_config, selected_local_model.is_some());
    let git_coordinator = Arc::new(GitCoordinator::new());

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
    let pre_swarm_sha = if config.use_worktrees {
        crate::git::current_head_sha(&config.workspace_root).unwrap_or_default()
    } else {
        String::new()
    };

    observer.on_phase_changed("validating");

    // 1. Validate scopes
    let loop_groups: Vec<Vec<String>> = plan
        .loop_configs
        .iter()
        .map(|lc| lc.agent_ids.clone())
        .collect();
    let scope_errors = validation::validate_scopes(&work_units, &loop_groups);
    if !scope_errors.is_empty() {
        let msg = scope_errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("scope validation failed: {}", msg);
    }

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
        let single_repo_map: Arc<Option<crate::repo_map::RepoMap>> = {
            let workspace = config.workspace_root.clone();
            let excludes = config.excludes.clone();
            Arc::new(
                tokio::task::spawn_blocking(move || {
                    crate::repo_map::RepoMap::build(&workspace, &excludes)
                        .map_err(|e| {
                            tracing::debug!("repo_map build skipped: {}", e);
                            e
                        })
                        .ok()
                })
                .await
                .unwrap_or(None),
            )
        };
        // Pre-compute impact analysis for the single agent
        let single_impact_text: Option<String> = {
            let workspace = config.workspace_root.clone();
            let excludes = config.excludes.clone();
            let owned_paths = unit.scope.owned_paths.clone();
            tokio::task::spawn_blocking(move || {
                crate::repo_map::graph_builder::build_graph(&workspace, &excludes)
                    .map(|(store, result)| {
                        tracing::info!(
                            "code graph: {} nodes, {} edges ({} files changed, {} unchanged)",
                            result.total_nodes,
                            result.total_edges,
                            result.files_changed,
                            result.files_unchanged,
                        );
                        let owned: Vec<&str> = owned_paths.iter().map(|s| s.as_str()).collect();
                        if owned.is_empty() {
                            return None;
                        }
                        store.impact_radius(&owned, 3).ok().and_then(|impact| {
                            if impact.affected_files.is_empty() {
                                None
                            } else {
                                Some(
                                    crate::repo_map::store::GraphStore::format_impact_for_prompt(
                                        &impact,
                                    ),
                                )
                            }
                        })
                    })
                    .unwrap_or(None)
            })
            .await
            .unwrap_or(None)
        };

        let effective_read_ns: Vec<String> = unit
            .read_namespaces
            .as_deref()
            .unwrap_or(config.read_namespaces.as_slice())
            .to_vec();
        let agent_ctx = AgentRunContext {
            workspace_root: &config.workspace_root,
            context_files: &config.context_files,
            memory: memory.clone(),
            read_namespaces: &effective_read_ns,
            swarm_observer: observer,
            git_coordinator: git_coordinator.clone(),
            validation: single_validation.clone(),
            board: None,
            repo_map: single_repo_map.clone(),
            impact_texts: Arc::new({
                let mut map = std::collections::HashMap::new();
                if let Some(text) = single_impact_text.clone() {
                    map.insert(unit.id.clone(), text);
                }
                map
            }),
            // Single-agent fast path: no bundle pre-fetch (1 coordinator query
            // + 1 runner query = 2, already within M7 ≤2 gate).
            pre_fetched_memory: Arc::new(None),
        };

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
            let effective_write_ns = unit
                .write_namespace
                .as_deref()
                .unwrap_or(&config.write_namespace);
            store_agent_result(
                &memory,
                effective_write_ns,
                &manifest,
                &unit,
                &run_id,
                &config.workspace_root,
            )
            .await;
        }
        exec_state.record_result(&unit.id, manifest.clone());
        let _ = exec_state.save(&plan_hash);

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
        if config.workspace_root.join("Cargo.toml").exists() {
            Some(Arc::new(
                crate::validation_gate::ValidationPipeline::default_for_rust(),
            ))
        } else {
            Some(Arc::new(
                crate::validation_gate::ValidationPipeline::fast_only(),
            ))
        };

    // Build repo map once for context optimization (best-effort; failures are non-fatal).
    // Runs on a blocking thread to avoid starving the async executor during workspace scan.
    tracing::info!("repo_map: scanning workspace");
    let repo_map: Arc<Option<crate::repo_map::RepoMap>> = {
        let workspace = config.workspace_root.clone();
        let excludes = config.excludes.clone();
        Arc::new(
            tokio::task::spawn_blocking(move || {
                crate::repo_map::RepoMap::build(&workspace, &excludes)
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

    // Build code knowledge graph and pre-compute impact analysis + context queries per agent.
    // GraphStore uses rusqlite (!Send), so we compute all texts upfront and share them as a
    // Send-safe HashMap. Runs on a blocking thread for the same reason as repo_map above.
    tracing::info!("code graph: indexing workspace");
    let units_for_graph: Vec<WorkUnit> = work_units
        .iter()
        .chain(plan.loop_judge_units.iter())
        .cloned()
        .collect();
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
                                let refs: Vec<&str> =
                                    wu.context_callers_of.iter().map(|s| s.as_str()).collect();
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
                                let refs: Vec<&str> =
                                    wu.context_tests_for.iter().map(|s| s.as_str()).collect();
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
        memory.as_deref(),
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

    // Shared discovery board: agents post tagged findings for downstream agents
    let shared_board = Arc::new(SharedBoard::new());

    // Optional worktree manager
    let mut worktree_mgr = if config.use_worktrees {
        let mgr = WorktreeManager::new(config.workspace_root.clone());
        if mgr.can_use_worktrees() {
            Some(mgr)
        } else {
            tracing::warn!(
                "Worktrees unavailable (no git commits?), running agents in shared workspace"
            );
            None
        }
    } else {
        None
    };

    observer.on_phase_changed("running");

    // Build a map from loop-agent id → iter_start for first-pass {{ITER}} substitution.
    // Agents that appear in a loop block get {{ITER}}/{{PREV_ITER}} substituted before
    // every dispatch (first pass uses iter_start; subsequent passes increment).
    let loop_agent_first_iter: std::collections::HashMap<String, u32> = plan
        .loop_configs
        .iter()
        .flat_map(|lc| {
            lc.agent_ids
                .iter()
                .map(move |id| (id.clone(), lc.iter_start))
        })
        .collect();
    let loop_judge_map: std::collections::HashMap<&str, &WorkUnit> = plan
        .loop_judge_units
        .iter()
        .map(|u| (u.id.as_str(), u))
        .collect();

    // 3. Execute tiers
    for (tier_idx, tier) in tiers.iter().enumerate() {
        observer.on_tier_started(tier_idx + 1, tiers.len());

        if effective_max_parallel <= 1 || tier.len() <= 1 {
            // Sequential execution
            for unit_id in tier {
                // Skip if already completed (resume support)
                if exec_state.status(unit_id) == NodeStatus::Completed {
                    tracing::info!("Skipping completed node '{}' (resume)", unit_id);
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

                let effective_read_ns: Vec<String> = unit
                    .read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();

                let agent_ctx = AgentRunContext {
                    workspace_root: &config.workspace_root,
                    context_files: &config.context_files,
                    memory: memory.clone(),
                    read_namespaces: &effective_read_ns,
                    swarm_observer: observer,
                    git_coordinator: git_coordinator.clone(),
                    validation: validation_pipeline.clone(),
                    board: Some(shared_board.clone()),
                    repo_map: repo_map.clone(),
                    impact_texts: impact_texts.clone(),
                    pre_fetched_memory: pre_fetched_memory.clone(),
                };
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
                    let effective_write_ns = unit
                        .write_namespace
                        .as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(
                        &memory,
                        effective_write_ns,
                        &manifest,
                        unit,
                        &run_id,
                        &config.workspace_root,
                    )
                    .await;
                }
                // Record result in execution state and checkpoint
                exec_state.record_result(unit_id, manifest.clone());
                if let Err(e) = exec_state.save(&plan_hash) {
                    tracing::warn!("Failed to save execution state checkpoint: {}", e);
                }
                all_manifests.push(manifest);
                if failed {
                    break;
                }
            }
        } else {
            // Parallel execution within tier
            let mut handles = Vec::new();

            // Register all agents as Pending before spawning
            for unit_id in tier {
                observer.on_agent_state_changed(unit_id, &AgentStatus::Pending, "queued");
            }

            for unit_id in tier {
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
                            mem.as_deref(),
                            &ns,
                            obs.as_ref(),
                            val_pipeline.as_deref(),
                            board_ref.as_deref(),
                            (*rm).as_ref(),
                            agent_impact.as_deref(),
                            (*pfm).as_deref(),
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
                        let changed = git_coord
                            .lock_git(move || {
                                commit_agent_changes(&agent_root_c, &unit_id_c, &summary)
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
                                let effective_write_ns = unit
                                    .write_namespace
                                    .as_deref()
                                    .unwrap_or(&config.write_namespace);
                                store_agent_result(
                                    &memory,
                                    effective_write_ns,
                                    &manifest,
                                    unit,
                                    &run_id,
                                    &config.workspace_root,
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
    for loop_config in &plan.loop_configs {
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
        for iteration in 1..loop_config.max_iterations {
            let current_iter_abs = loop_config.iter_start + iteration - 1;
            let condition_met = {
                let mut loop_ctx = LoopConditionContext {
                    config,
                    memory: &memory,
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
                };
                evaluate_loop_condition(&loop_config.until, current_iter_abs, &mut loop_ctx).await
            };

            if condition_met {
                consecutive_pass = consecutive_pass.saturating_add(1);
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
            } else {
                if consecutive_pass > 0 {
                    tracing::debug!(
                        "Loop PASS streak broken by FAIL at iteration {}, resetting counter",
                        iteration
                    );
                }
                consecutive_pass = 0;
            }

            tracing::info!(
                "Loop iteration {}/{} for agents {:?}",
                iteration + 1,
                loop_config.max_iterations,
                loop_config.agent_ids
            );
            observer.on_phase_changed(&format!("loop iteration {}", iteration + 1));

            // Re-run each agent in the loop sequentially
            for agent_id in &loop_config.agent_ids {
                let unit_template = match unit_map.get(agent_id.as_str()) {
                    Some(u) => u,
                    None => continue,
                };

                // Substitute {{ITER}} / {{PREV_ITER}} for this specific iteration.
                // iteration is 1-indexed here (1..max_iterations); iter_abs = iter_start + iteration.
                let iter_abs = loop_config.iter_start + iteration as u32;
                let iter_unit = apply_iter_vars(unit_template, iter_abs);
                let unit = &iter_unit;

                observer.on_agent_state_changed(agent_id, &AgentStatus::Running, &unit.description);

                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns: Vec<String> = unit
                    .read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();

                let agent_ctx = AgentRunContext {
                    workspace_root: &config.workspace_root,
                    context_files: &config.context_files,
                    memory: memory.clone(),
                    read_namespaces: &effective_read_ns,
                    swarm_observer: observer,
                    git_coordinator: git_coordinator.clone(),
                    validation: validation_pipeline.clone(),
                    board: Some(shared_board.clone()),
                    repo_map: repo_map.clone(),
                    impact_texts: impact_texts.clone(),
                    pre_fetched_memory: pre_fetched_memory.clone(),
                };
                let manifest = run_single_agent(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    &tier_router,
                    &plan.iteration_config,
                    make_observer(agent_id),
                )
                .await?;

                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    let effective_write_ns = unit
                        .write_namespace
                        .as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(
                        &memory,
                        effective_write_ns,
                        &manifest,
                        unit,
                        &run_id,
                        &config.workspace_root,
                    )
                    .await;
                }
                exec_state.record_result(agent_id, manifest.clone());
                all_manifests.push(manifest);
            }
        }

        // Final check after all iterations, but avoid re-running a judge after
        // the loop already terminated successfully.
        if !loop_terminated {
            let final_iter_abs =
                loop_config.iter_start + loop_config.max_iterations.saturating_sub(1);
            let final_met = {
                let mut loop_ctx = LoopConditionContext {
                    config,
                    memory: &memory,
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
                };
                evaluate_loop_condition(&loop_config.until, final_iter_abs, &mut loop_ctx).await
            };
            if final_met {
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
            } else {
                tracing::warn!(
                    "Loop exhausted max_iterations ({}) without condition being met for agents {:?}",
                    loop_config.max_iterations,
                    loop_config.agent_ids
                );
            }
        }
    }

    // 4. Merge phase (only if using worktrees)
    if config.use_worktrees {
        observer.on_phase_changed("merging");

        for manifest in &all_manifests {
            if let Some(ref branch) = manifest.branch {
                if matches!(manifest.status, AgentStatus::Completed) {
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
        let consolidator = crate::memory::consolidation::Consolidator::new(Arc::clone(mem));
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
    memory: Option<Arc<MemoryStore>>,
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
}

struct LoopConditionContext<'a> {
    config: &'a SwarmConfig,
    memory: &'a Option<Arc<MemoryStore>>,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JudgeVerdict {
    Pass,
    Fail,
}

/// Clone `unit` and substitute `{{ITER}}` / `{{PREV_ITER}}` with `iter_abs`
/// and `iter_abs - 1` respectively. Called for every loop-agent dispatch.
fn apply_iter_vars(unit: &WorkUnit, iter_abs: u32) -> WorkUnit {
    apply_iter_vars_with_evidence(unit, iter_abs, "")
}

/// Clone `unit` and substitute `{{ITER}}`, `{{PREV_ITER}}`, and
/// `{{ITER_EVIDENCE}}` in `coordinator_instructions`. Evidence is intended
/// for judge agents — it summarises the previous iteration's manifests and
/// modified files so the judge can decide on facts instead of hallucinating.
fn apply_iter_vars_with_evidence(unit: &WorkUnit, iter_abs: u32, evidence: &str) -> WorkUnit {
    let prev = iter_abs.saturating_sub(1);
    WorkUnit {
        coordinator_instructions: unit
            .coordinator_instructions
            .replace("{{ITER}}", &iter_abs.to_string())
            .replace("{{PREV_ITER}}", &prev.to_string())
            .replace("{{ITER_EVIDENCE}}", evidence),
        ..unit.clone()
    }
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
        if loop_set.contains(m.work_unit_id.as_str()) && !by_agent.contains_key(m.work_unit_id.as_str()) {
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
                out.push_str(&format!(
                    "    … and {} more\n",
                    m.modified_files.len() - 20
                ));
            }
        }
    }
    out
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
    run_agent_inner(unit, worktree_mgr, ctx, tier_router, iteration_config, acp_observer, false)
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
    run_agent_inner(unit, None, ctx, tier_router, iteration_config, acp_observer, true)
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
    let agent_root = if let Some(mgr) = worktree_mgr {
        let handle = mgr.provision(&unit.id)?;
        let path = handle.path.clone();
        if !context_files.is_empty() {
            if let Err(e) = mgr.inject_context_files(&unit.id, context_files) {
                tracing::warn!("Failed to inject context files for {}: {}", unit.id, e);
            }
        }
        path
    } else {
        workspace_root.clone()
    };

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
            memory.as_deref(),
            read_namespaces,
            acp_observer.as_ref(),
            validation.as_deref(),
            board.as_deref(),
            (*repo_map).as_ref(),
            impact_text.as_deref(),
            pre_fetched_memory_text.as_deref(),
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
        let changed = git_coordinator
            .lock_git(move || commit_agent_changes(&agent_root_c, &unit_id_c, &summary))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e);
                vec![]
            });
        manifest.modified_files = changed;
        manifest.branch = Some(format!("gaviero/{}", unit.id));
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
    let passed = run_verification_checks(
        config,
        workspace_root,
        excludes,
        Some(modified_files.as_slice()),
    )
    .await?;
    if !passed {
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

async fn run_verification_checks(
    config: &super::plan::VerificationConfig,
    workspace_root: &std::path::Path,
    excludes: &[String],
    modified_files: Option<&[std::path::PathBuf]>,
) -> Result<bool> {
    if config.compile && !run_verification_command(workspace_root, "cargo", &["check"]).await {
        return Ok(false);
    }

    if config.test && !run_test_verification(workspace_root, &[], false).await? {
        return Ok(false);
    }

    if config.impact_tests {
        let passed = if let Some(files) = modified_files {
            run_test_verification(workspace_root, files, true).await?
        } else {
            run_conservative_impact_tests(workspace_root, excludes).await
        };
        if !passed {
            return Ok(false);
        }
    }

    if config.clippy
        && !run_verification_command(workspace_root, "cargo", &["clippy", "--", "-D", "warnings"])
            .await
    {
        return Ok(false);
    }

    Ok(true)
}

async fn run_verification_command(
    workspace_root: &std::path::Path,
    program: &str,
    args: &[&str],
) -> bool {
    tokio::process::Command::new(program)
        .args(args)
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run_test_verification(
    workspace_root: &std::path::Path,
    modified_files: &[std::path::PathBuf],
    targeted: bool,
) -> Result<bool> {
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
    Ok(report.passed)
}

async fn run_conservative_impact_tests(
    workspace_root: &std::path::Path,
    excludes: &[String],
) -> bool {
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
                    if !run_verification_command(workspace_root, "cargo", &["test", test_mod]).await
                    {
                        return false;
                    }
                }
            }
            true
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
fn commit_agent_changes(
    worktree_path: &std::path::Path,
    agent_id: &str,
    summary: &str,
) -> Result<Vec<std::path::PathBuf>> {
    use std::process::Command;

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

/// Store an agent's execution result to memory (best-effort, never fails the pipeline).
///
/// Writes one aggregate entry for the agent's run, plus one sentinel entry per
/// `staleness_source` path recording the current file hash. On the next run,
/// `invalidate_stale_sources` checks these hashes and marks changed entries stale.
async fn store_agent_result(
    memory: &Option<Arc<MemoryStore>>,
    write_ns: &str,
    manifest: &AgentManifest,
    unit: &WorkUnit,
    run_id: &str,
    workspace_root: &std::path::Path,
) {
    let Some(mem) = memory else { return };

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
    if let Err(e) = mem
        .store_with_options(write_ns, &key, &content, &opts)
        .await
    {
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
        if let Err(e) = mem
            .store_with_options(write_ns, &src_key, &src_content, &src_opts)
            .await
        {
            tracing::warn!("Failed to store source snapshot for {}: {}", source_path, e);
        }
    }
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
    memory: Option<Arc<MemoryStore>>,
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
        }
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

    #[test]
    fn judge_verdict_parser_extracts_fenced_json_block() {
        let text = "Reasoning: the diff looks clean.\n\n```json\n{\"verdict\":\"pass\",\"reason\":\"stable\"}\n```\n";
        assert_eq!(parse_judge_verdict(text), Some(JudgeVerdict::Pass));

        let bare = "```\n{\"pass\":false}\n```";
        assert_eq!(parse_judge_verdict(bare), Some(JudgeVerdict::Fail));
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
    memory: &Option<Arc<MemoryStore>>,
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

    match mem.check_staleness(&paths).await {
        Ok(stale) if !stale.is_empty() => {
            let ids: Vec<i64> = stale.iter().map(|(id, _, _, _)| *id).collect();
            tracing::info!(
                "Invalidating {} stale memory entries before running agent '{}'",
                ids.len(),
                unit.id
            );
            if let Err(e) = mem.mark_stale(&ids).await {
                tracing::warn!("mark_stale failed for agent '{}': {}", unit.id, e);
            }
        }
        Ok(_) => {} // nothing stale
        Err(e) => {
            tracing::warn!("check_staleness failed for agent '{}': {}", unit.id, e);
        }
    }
}

/// Evaluate a loop's exit condition.
///
/// Returns `true` if the condition is met and the loop should stop.
async fn evaluate_loop_condition(
    condition: &super::plan::LoopUntilCondition,
    current_iter_abs: u32,
    ctx: &mut LoopConditionContext<'_>,
) -> bool {
    match condition {
        super::plan::LoopUntilCondition::Verify(config) => {
            run_verification_checks(config, &ctx.config.workspace_root, &ctx.config.excludes, None)
                .await
                .unwrap_or(false)
        }
        super::plan::LoopUntilCondition::Agent(agent_id) => {
            let Some(unit_template) = ctx.loop_judge_map.get(agent_id.as_str()).copied() else {
                tracing::warn!(
                    "loop judge agent '{}' not found in compiled plan (judges must be declared distinct from workflow agents)",
                    agent_id
                );
                return false;
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
                build_iter_evidence(
                    ctx.all_manifests,
                    ctx.loop_agent_ids,
                    current_iter_abs,
                )
            } else {
                String::new()
            };
            let unit =
                apply_iter_vars_with_evidence(unit_template, current_iter_abs, &evidence);
            invalidate_stale_sources(ctx.memory, &unit, &ctx.config.workspace_root).await;

            let effective_read_ns: Vec<String> = unit
                .read_namespaces
                .as_deref()
                .unwrap_or(ctx.config.read_namespaces.as_slice())
                .to_vec();

            let agent_ctx = AgentRunContext {
                workspace_root: &ctx.config.workspace_root,
                context_files: &ctx.config.context_files,
                memory: ctx.memory.clone(),
                read_namespaces: &effective_read_ns,
                swarm_observer: ctx.observer,
                git_coordinator: ctx.git_coordinator.clone(),
                validation: ctx.validation.clone(),
                board: Some(ctx.shared_board.clone()),
                repo_map: ctx.repo_map.clone(),
                impact_texts: ctx.impact_texts.clone(),
                pre_fetched_memory: ctx.pre_fetched_memory.clone(),
            };

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
            manifest.summary = Some(match (verdict, &manifest.status) {
                (Some(JudgeVerdict::Pass), _) => "Judge verdict: PASS".into(),
                (Some(JudgeVerdict::Fail), _) => "Judge verdict: FAIL".into(),
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
                    &judge_ns,
                    &manifest,
                    &unit,
                    ctx.run_id,
                    &ctx.config.workspace_root,
                )
                .await;
            }

            let condition_met = matches!(verdict, Some(JudgeVerdict::Pass));

            ctx.all_manifests.push(manifest);
            condition_met
        }
        super::plan::LoopUntilCondition::Command(cmd) => {
            // Run the shell command; exit code 0 = condition met
            let result = tokio::process::Command::new("sh")
                .args(["-c", cmd])
                .current_dir(&ctx.config.workspace_root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            result.map(|s| s.success()).unwrap_or(false)
        }
    }
}

fn parse_judge_verdict(text: &str) -> Option<JudgeVerdict> {
    let trimmed = text.trim();
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
        _ => None,
    }
}

/// No-op write gate observer for parallel agents (AutoAccept mode).
struct NoopWriteGateObserver;

impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}
