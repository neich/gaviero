//! Swarm pipeline: validates → tiers → parallel execution → merge.
//!
//! Orchestrates multi-agent execution with git worktree isolation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, Semaphore};

use super::board::SharedBoard;
use super::bus::AgentBus;
use super::coordinator::{Coordinator, CoordinatorConfig};
use super::execution_state::{ExecutionState, NodeStatus};
use super::models::{AgentManifest, AgentStatus, MergeResult, SwarmResult, WorkUnit};
use super::plan::CompiledPlan;
use super::validation;
use super::merge;
use crate::git::{GitCoordinator, WorktreeManager};
use crate::memory::{MemoryStore, StoreOptions};
use crate::memory::store::file_hash;
use crate::observer::{AcpObserver, SwarmObserver};
use crate::types::{EntryMetadata, PrivacyLevel};
use crate::write_gate::{WriteGatePipeline, WriteMode};

/// Configuration for a swarm execution.
pub struct SwarmConfig {
    pub max_parallel: usize,
    pub workspace_root: PathBuf,
    pub model: String,
    pub use_worktrees: bool,
    pub read_namespaces: Vec<String>,
    pub write_namespace: String,
    /// Extra files to inject into each agent's worktree after provisioning.
    /// Populated from `@file` references in the user prompt that are not git-tracked
    /// (e.g. `tmp/` plan documents). Each entry is `(rel_path, content)`.
    pub context_files: Vec<(String, String)>,
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
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver>,
) -> Result<SwarmResult> {
    tracing::info!(
        agents = plan.graph.node_count(),
        max_parallel = config.max_parallel,
        "swarm.execute starting"
    );

    // Extract work units in topological order from the plan graph
    let work_units = plan.work_units_ordered()
        .map_err(|e| anyhow::anyhow!("plan graph error: {}", e))?;

    // Override max_parallel from plan if declared
    let effective_max_parallel = plan.max_parallel.unwrap_or(config.max_parallel);

    // Execution state tracks per-node progress (populated as nodes complete)
    let mut exec_state = initial_state
        .unwrap_or_else(|| ExecutionState::new_from_plan(plan));
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
    let run_id = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    // Capture HEAD SHA before any merges (for revert support)
    let pre_swarm_sha = if config.use_worktrees {
        crate::git::current_head_sha(&config.workspace_root).unwrap_or_default()
    } else {
        String::new()
    };

    observer.on_phase_changed("validating");

    // 1. Validate scopes
    let scope_errors = validation::validate_scopes(&work_units);
    if !scope_errors.is_empty() {
        let msg = scope_errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
        anyhow::bail!("scope validation failed: {}", msg);
    }

    // ── Single-agent fast path ────────────────────────────────────────────────
    // One work unit → bypass worktrees, bus, and merge; run directly through
    // the IterationEngine so strategy / retry / model-escalation all apply.
    if work_units.len() == 1 {
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

        let backend = super::backend::claude_code::ClaudeCodeBackend::new(
            unit.model.as_deref().unwrap_or("sonnet"),
        );
        let write_gate = Arc::new(Mutex::new(
            WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
        ));
        let single_validation: Option<Arc<crate::validation_gate::ValidationPipeline>> =
            if config.workspace_root.join("Cargo.toml").exists() {
                Some(Arc::new(crate::validation_gate::ValidationPipeline::default_for_rust()))
            } else {
                Some(Arc::new(crate::validation_gate::ValidationPipeline::fast_only()))
            };
        let single_repo_map: Arc<Option<crate::repo_map::RepoMap>> = Arc::new(
            crate::repo_map::RepoMap::build(&config.workspace_root)
                .map_err(|e| { tracing::debug!("repo_map build skipped: {}", e); e })
                .ok(),
        );
        // Pre-compute impact analysis for the single agent
        let single_impact_text: Option<String> = crate::repo_map::graph_builder::build_graph(&config.workspace_root)
            .map(|(store, result)| {
                tracing::info!(
                    "code graph: {} nodes, {} edges ({} files changed, {} unchanged)",
                    result.total_nodes, result.total_edges, result.files_changed, result.files_unchanged,
                );
                let owned: Vec<&str> = unit.scope.owned_paths.iter().map(|s| s.as_str()).collect();
                if owned.is_empty() { return None; }
                store.impact_radius(&owned, 3).ok().and_then(|impact| {
                    if impact.affected_files.is_empty() { None }
                    else { Some(crate::repo_map::store::GraphStore::format_impact_for_prompt(&impact)) }
                })
            })
            .unwrap_or(None);

        let engine = crate::iteration::IterationEngine::new(plan.iteration_config.clone());
        let effective_read_ns: Vec<String> = unit.read_namespaces
            .as_deref()
            .unwrap_or(config.read_namespaces.as_slice())
            .to_vec();
        let acp_obs = make_observer(&unit.id);

        invalidate_stale_sources(&memory, &unit, &config.workspace_root).await;

        let iter_result = engine
            .run(
                &backend,
                unit.clone(),
                write_gate,
                &config.workspace_root,
                memory.as_deref(),
                &effective_read_ns,
                acp_obs.as_ref(),
                single_validation.as_deref(),
                None,
                (*single_repo_map).as_ref(),
                single_impact_text.as_deref(),
            )
            .await;

        let manifest = iter_result.manifest;
        let success = matches!(manifest.status, AgentStatus::Completed);
        observer.on_agent_state_changed(
            &manifest.work_unit_id,
            &manifest.status,
            manifest.summary.as_deref().unwrap_or(""),
        );

        if success {
            let effective_write_ns = unit.write_namespace.as_deref()
                .unwrap_or(&config.write_namespace);
            store_agent_result(&memory, effective_write_ns, &manifest, &unit, &run_id, &config.workspace_root).await;
        }
        exec_state.record_result(&unit.id, manifest.clone());
        let _ = exec_state.save(&plan_hash);

        let swarm_result = SwarmResult {
            manifests: vec![manifest],
            merge_results: vec![],
            success,
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

    // Serialize concurrent git metadata operations (prevents .git/index.lock races)
    let git_coordinator = Arc::new(GitCoordinator::new());

    // Build validation pipeline based on workspace type (shared across all agents via Arc)
    let validation_pipeline: Option<Arc<crate::validation_gate::ValidationPipeline>> =
        if config.workspace_root.join("Cargo.toml").exists() {
            Some(Arc::new(crate::validation_gate::ValidationPipeline::default_for_rust()))
        } else {
            Some(Arc::new(crate::validation_gate::ValidationPipeline::fast_only()))
        };

    // Build repo map once for context optimization (best-effort; failures are non-fatal)
    let repo_map: Arc<Option<crate::repo_map::RepoMap>> = Arc::new(
        crate::repo_map::RepoMap::build(&config.workspace_root)
            .map_err(|e| { tracing::debug!("repo_map build skipped: {}", e); e })
            .ok()
    );

    // Build code knowledge graph and pre-compute impact analysis + context queries per agent.
    // GraphStore uses rusqlite (!Send), so we compute all texts upfront
    // and share them as a Send-safe HashMap.
    let impact_texts: Arc<std::collections::HashMap<String, String>> = Arc::new({
        let mut map = std::collections::HashMap::new();
        match crate::repo_map::graph_builder::build_graph(&config.workspace_root) {
            Ok((store, result)) => {
                tracing::info!(
                    "code graph: {} nodes, {} edges ({} files changed, {} unchanged)",
                    result.total_nodes, result.total_edges, result.files_changed, result.files_unchanged,
                );
                for wu in &work_units {
                    let mut sections: Vec<String> = Vec::new();

                    // Impact analysis from owned paths
                    let owned: Vec<&str> = wu.scope.owned_paths.iter().map(|s| s.as_str()).collect();
                    if !owned.is_empty() {
                        let depth = if wu.impact_scope { wu.context_depth.max(3) as usize } else { 3 };
                        if let Ok(impact) = store.impact_radius(&owned, depth) {
                            if !impact.affected_files.is_empty() {
                                sections.push(
                                    crate::repo_map::store::GraphStore::format_impact_for_prompt(&impact),
                                );
                            }
                        }
                    }

                    // Context block: callers_of queries
                    if !wu.context_callers_of.is_empty() {
                        let refs: Vec<&str> = wu.context_callers_of.iter().map(|s| s.as_str()).collect();
                        if let Ok(impact) = store.impact_radius(&refs, wu.context_depth as usize) {
                            let callers: Vec<&str> = impact.affected_files.iter()
                                .filter(|f| !wu.context_callers_of.contains(f))
                                .map(|s| s.as_str())
                                .collect();
                            if !callers.is_empty() {
                                sections.push(format!("[Callers of {:?}]:\n{}", wu.context_callers_of, callers.join(", ")));
                            }
                        }
                    }

                    // Context block: tests_for queries
                    if !wu.context_tests_for.is_empty() {
                        let refs: Vec<&str> = wu.context_tests_for.iter().map(|s| s.as_str()).collect();
                        if let Ok(impact) = store.impact_radius(&refs, wu.context_depth as usize) {
                            if !impact.affected_tests.is_empty() {
                                sections.push(format!("[Tests for {:?}]:\n{}", wu.context_tests_for, impact.affected_tests.join(", ")));
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
        map
    });

    // Inter-agent communication bus (available for future coordination)
    let bus = Arc::new(tokio::sync::Mutex::new(AgentBus::new()));
    // Register all agents upfront so they can send messages to each other
    {
        let mut b = bus.lock().await;
        for unit in &work_units {
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
            tracing::warn!("Worktrees unavailable (no git commits?), running agents in shared workspace");
            None
        }
    } else {
        None
    };

    observer.on_phase_changed("running");

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

                let unit = unit_map.get(unit_id.as_str())
                    .with_context(|| format!("work unit '{}' not found", unit_id))?;

                exec_state.set_status(unit_id, NodeStatus::Running);
                observer.on_agent_state_changed(
                    unit_id,
                    &AgentStatus::Running,
                    &unit.description,
                );

                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns: Vec<String> = unit.read_namespaces
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
                };
                let manifest = run_single_agent(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    make_observer(unit_id),
                ).await?;

                let failed = matches!(manifest.status, AgentStatus::Failed(_));
                // Broadcast completion to bus so later tiers can see results
                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    // Store result to memory
                    let effective_write_ns = unit.write_namespace.as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(&memory, effective_write_ns, &manifest, unit, &run_id, &config.workspace_root).await;
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
                observer.on_agent_state_changed(
                    unit_id,
                    &AgentStatus::Pending,
                    "queued",
                );
            }

            for unit_id in tier {
                let unit = (*unit_map.get(unit_id.as_str())
                    .with_context(|| format!("work unit '{}' not found", unit_id))?)
                    .clone();

                let sem = semaphore.clone();
                let root = config.workspace_root.clone();
                let mem = memory.clone();
                let ns: Vec<String> = unit.read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();
                let obs = make_observer(unit_id);
                let git_coord = git_coordinator.clone();
                let val_pipeline = validation_pipeline.clone();
                let board_ref = Some(shared_board.clone());
                let rm = repo_map.clone();
                let agent_impact = impact_texts.get(unit_id).cloned();

                // Provision worktree if enabled
                let in_worktree = worktree_mgr.is_some();
                let agent_root = if let Some(ref mut mgr) = worktree_mgr {
                    let handle = mgr.provision(&unit.id)?;
                    handle.path.clone()
                } else {
                    root.clone()
                };

                // Resolve backend for this unit
                let backend = super::backend::claude_code::ClaudeCodeBackend::new(
                    unit.model.as_deref().unwrap_or("sonnet"),
                );

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    invalidate_stale_sources(&mem, &unit, &root).await;

                    let write_gate = Arc::new(Mutex::new(
                        WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
                    ));
                    let mut manifest = super::backend::runner::run_backend(
                        &backend,
                        &unit,
                        write_gate,
                        &agent_root,
                        mem.as_deref(),
                        &ns,
                        obs.as_ref(),
                        val_pipeline.as_deref(),
                        board_ref.as_deref(),
                        (*rm).as_ref(),
                        agent_impact.as_deref(),
                    ).await?;

                    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
                        let summary = manifest.summary.as_deref().unwrap_or("task complete").to_string();
                        let agent_root_c = agent_root.clone();
                        let unit_id_c = unit.id.clone();
                        let changed = git_coord.lock_git(move || {
                            commit_agent_changes(&agent_root_c, &unit_id_c, &summary)
                        }).await.unwrap_or_else(|e| { tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e); vec![] });
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
                                &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                            );
                            // Store result to memory
                            if let Some(unit) = unit_map.get(manifest.work_unit_id.as_str()) {
                                let effective_write_ns = unit.write_namespace.as_deref()
                                    .unwrap_or(&config.write_namespace);
                                store_agent_result(&memory, effective_write_ns, &manifest, unit, &run_id, &config.workspace_root).await;
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
        for iteration in 1..loop_config.max_iterations {
            let condition_met = evaluate_loop_condition(
                &loop_config.until,
                &config.workspace_root,
            ).await;

            if condition_met {
                tracing::info!(
                    "Loop condition met after {} iteration(s) for agents {:?}",
                    iteration,
                    loop_config.agent_ids
                );
                break;
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
                let unit = match unit_map.get(agent_id.as_str()) {
                    Some(u) => u,
                    None => continue,
                };

                observer.on_agent_state_changed(
                    agent_id,
                    &AgentStatus::Running,
                    &unit.description,
                );

                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns: Vec<String> = unit.read_namespaces
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
                };
                let manifest = run_single_agent(
                    unit,
                    worktree_mgr.as_mut(),
                    &agent_ctx,
                    make_observer(agent_id),
                ).await?;

                if matches!(manifest.status, AgentStatus::Completed) {
                    let b = bus.lock().await;
                    b.broadcast(
                        &manifest.work_unit_id,
                        &format!("completed: {}", manifest.summary.as_deref().unwrap_or("")),
                    );
                    let effective_write_ns = unit.write_namespace.as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(&memory, effective_write_ns, &manifest, unit, &run_id, &config.workspace_root).await;
                }
                exec_state.record_result(agent_id, manifest.clone());
                all_manifests.push(manifest);
            }
        }

        // Final check after all iterations
        let final_met = evaluate_loop_condition(
            &loop_config.until,
            &config.workspace_root,
        ).await;
        if !final_met {
            tracing::warn!(
                "Loop exhausted max_iterations ({}) without condition being met for agents {:?}",
                loop_config.max_iterations,
                loop_config.agent_ids
            );
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
                        let files: Vec<String> = result.conflicts
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
                        ).await;

                        match resolved {
                            Ok(resolved_conflicts) => {
                                let all_ok = resolved_conflicts.iter().all(|c| c.resolved);
                                result.conflicts = resolved_conflicts;
                                result.success = all_ok;
                                if !all_ok {
                                    tracing::warn!("some conflicts could not be auto-resolved for {}", branch);
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

    let success = all_manifests.iter().all(|m| matches!(m.status, AgentStatus::Completed))
        && all_merges.iter().all(|m| m.success);

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
}

/// Run a single agent, optionally in a worktree.
async fn run_single_agent(
    unit: &WorkUnit,
    worktree_mgr: Option<&mut WorktreeManager>,
    ctx: &AgentRunContext<'_>,
    acp_observer: Box<dyn AcpObserver>,
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

    // Resolve backend from model override or default to Claude Code
    let backend = super::backend::claude_code::ClaudeCodeBackend::new(
        unit.model.as_deref().unwrap_or("sonnet"),
    );

    let write_gate = Arc::new(Mutex::new(
        WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
    ));

    swarm_observer.on_agent_state_changed(&unit.id, &AgentStatus::Running, "starting");

    let mut manifest = super::backend::runner::run_backend(
        &backend,
        unit,
        write_gate,
        &agent_root,
        memory.as_deref(),
        read_namespaces,
        acp_observer.as_ref(),
        validation.as_deref(),
        board.as_deref(),
        (*repo_map).as_ref(),
        impact_text.as_deref(),
    )
    .await?;

    swarm_observer.on_agent_state_changed(
        &unit.id,
        &manifest.status,
        manifest.summary.as_deref().unwrap_or(""),
    );

    // Commit changes and record branch name if running in a worktree.
    // The GitCoordinator serializes concurrent commits to prevent .git/index.lock races.
    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
        let summary = manifest.summary.as_deref().unwrap_or("task complete").to_string();
        let agent_root_c = agent_root.clone();
        let unit_id_c = unit.id.clone();
        let changed = git_coordinator.lock_git(move || {
            commit_agent_changes(&agent_root_c, &unit_id_c, &summary)
        }).await.unwrap_or_else(|e| { tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e); vec![] });
        manifest.modified_files = changed;
        manifest.branch = Some(format!("gaviero/{}", unit.id));
    }

    Ok(manifest)
}

/// Commit all changes in a worktree after an agent completes.
///
/// Stages everything with `git add -A` then commits. Returns the list of files
/// changed in the commit, or an empty vec if the working tree was already clean.
fn commit_agent_changes(worktree_path: &std::path::Path, agent_id: &str, summary: &str) -> Result<Vec<std::path::PathBuf>> {
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
    anyhow::ensure!(add.success(), "git add failed in worktree {}", worktree_path.display());

    // Commit — silence stdout/stderr so git's progress output doesn't corrupt the TUI
    let msg = format!(
        "gaviero: agent {} — {}",
        agent_id,
        if summary.is_empty() { "task complete" } else { summary }
    );
    let commit = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(worktree_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git commit in worktree")?;
    anyhow::ensure!(commit.success(), "git commit failed in worktree {}", worktree_path.display());

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
    let files: Vec<String> = manifest.modified_files.iter()
        .map(|p| p.display().to_string())
        .collect();
    // {{SUMMARY}} resolves to the agent's full text output (preferred) or short summary.
    let summary_text = manifest.output.as_deref()
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
    if let Err(e) = mem.store_with_options(write_ns, &key, &content, &opts).await {
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
        let src_key = format!("agents:{}:{}:src:{}", run_id, manifest.work_unit_id, source_path);
        let src_content = format!("Source snapshot: {} (hash: {})", source_path, hash);
        let src_opts = StoreOptions {
            privacy: privacy.to_string(),
            importance,
            metadata: metadata_json.clone(),
            source_file: Some(abs_str),  // absolute path — matches check_staleness input
            source_hash: Some(hash),
        };
        if let Err(e) = mem.store_with_options(write_ns, &src_key, &src_content, &src_opts).await {
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
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver>,
) -> Result<String> {
    observer.on_coordination_started(prompt);
    observer.on_agent_state_changed("coordinator", &AgentStatus::Running, "Opus planning (DSL)...");
    observer.on_tier_dispatch("coordinator", crate::types::ModelTier::Expensive, &config.model);

    let file_list = collect_file_list(&config.workspace_root)?;
    let coordinator = Coordinator::new(memory, coordinator_config);
    let coord_observer = make_observer("coordinator");

    match coordinator.plan_as_dsl(
        prompt,
        &config.workspace_root,
        &file_list,
        &config.read_namespaces,
        Some(coord_observer),
    ).await {
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

/// Merge one agent branch into the workspace, attempting auto-resolution on conflict.
///
/// Returns a `MergeResult`. Errors from `abort_merge` are logged but not propagated
/// so the caller can continue with remaining branches.
async fn merge_one_branch(
    workspace_root: &PathBuf,
    branch: &str,
    observer: &dyn SwarmObserver,
) -> MergeResult {
    let mut result = match merge::merge_branch(workspace_root, branch) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("merge_branch failed for {}: {}", branch, e);
            return MergeResult {
                work_unit_id: branch.to_string(),
                success: false,
                conflicts: vec![],
            };
        }
    };

    if !result.success && !result.conflicts.is_empty() {
        let files: Vec<String> = result
            .conflicts
            .iter()
            .map(|c| c.file.to_string_lossy().to_string())
            .collect();
        observer.on_merge_conflict(branch, &files);
        observer.on_phase_changed("resolving conflicts");

        let resolved = merge::auto_resolve_conflicts(
            workspace_root,
            branch,
            &result.conflicts,
            "opus",
        )
        .await;

        match resolved {
            Ok(resolved_conflicts) => {
                let all_ok = resolved_conflicts.iter().all(|c| c.resolved);
                result.conflicts = resolved_conflicts;
                result.success = all_ok;
                if !all_ok {
                    tracing::warn!("some conflicts could not be auto-resolved for {}", branch);
                    let _ = merge::abort_merge(workspace_root);
                }
            }
            Err(e) => {
                tracing::error!("auto-resolve failed for {}: {}", branch, e);
                let _ = merge::abort_merge(workspace_root);
            }
        }
    }
    result
}

/// Undo a swarm run by hard-resetting the repo to its pre-swarm state.
///
/// Deletes all agent branches that were part of `result`, then runs
/// `git reset --hard <pre_swarm_sha>`. This is destructive but recoverable
/// via `git reflog`.
pub fn revert_swarm(workspace_root: &std::path::Path, result: &super::models::SwarmResult) -> Result<()> {
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
        tracing::warn!("git ls-files failed in {:?}, coordinator will have no file list", workspace_root);
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
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
    if unit.staleness_sources.is_empty() { return };

    let paths: Vec<std::path::PathBuf> = unit.staleness_sources.iter()
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

/// Determine the next escalation tier in the chain.
///
/// Cheap → Expensive → None (ceiling reached).
/// Evaluate a loop's exit condition.
///
/// Returns `true` if the condition is met and the loop should stop.
async fn evaluate_loop_condition(
    condition: &super::plan::LoopUntilCondition,
    workspace_root: &std::path::Path,
) -> bool {
    match condition {
        super::plan::LoopUntilCondition::Verify(config) => {
            // Run compile/clippy/test checks and return true if all pass
            let mut all_pass = true;
            if config.compile {
                let result = tokio::process::Command::new("cargo")
                    .arg("check")
                    .current_dir(workspace_root)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                if !result.map(|s| s.success()).unwrap_or(false) {
                    all_pass = false;
                }
            }
            if config.test && all_pass {
                let result = tokio::process::Command::new("cargo")
                    .arg("test")
                    .current_dir(workspace_root)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                if !result.map(|s| s.success()).unwrap_or(false) {
                    all_pass = false;
                }
            }
            if config.impact_tests && all_pass {
                // Run only tests affected by the blast radius.
                // Build the graph, find affected test files, run them.
                match crate::repo_map::graph_builder::build_graph(workspace_root) {
                    Ok((store, _)) => {
                        // Use all source files as the "changed" set (conservative)
                        let all_src: Vec<String> = store.all_file_hashes()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|(f, _)| !f.contains("test"))
                            .map(|(f, _)| f)
                            .collect();
                        let refs: Vec<&str> = all_src.iter().map(|s| s.as_str()).collect();
                        if let Ok(impact) = store.impact_radius(&refs, 3) {
                            let test_modules: Vec<String> = impact.affected_tests.iter()
                                .filter_map(|t| {
                                    // Convert file path to test module name for cargo test filter
                                    t.strip_suffix(".rs")
                                        .map(|s| s.replace('/', "::"))
                                })
                                .collect();
                            if !test_modules.is_empty() {
                                for test_mod in &test_modules {
                                    let result = tokio::process::Command::new("cargo")
                                        .args(["test", test_mod])
                                        .current_dir(workspace_root)
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .status()
                                        .await;
                                    if !result.map(|s| s.success()).unwrap_or(false) {
                                        all_pass = false;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("impact_tests: graph build failed, falling back to full test: {}", e);
                        let result = tokio::process::Command::new("cargo")
                            .arg("test")
                            .current_dir(workspace_root)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await;
                        if !result.map(|s| s.success()).unwrap_or(false) {
                            all_pass = false;
                        }
                    }
                }
            }
            if config.clippy && all_pass {
                let result = tokio::process::Command::new("cargo")
                    .arg("clippy")
                    .args(["--", "-D", "warnings"])
                    .current_dir(workspace_root)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                if !result.map(|s| s.success()).unwrap_or(false) {
                    all_pass = false;
                }
            }
            all_pass
        }
        super::plan::LoopUntilCondition::Agent(_agent_id) => {
            // Judge agent evaluation: run the agent and parse its output for PASS/FAIL.
            // For now, return false (not met) — full implementation requires running
            // the agent through run_backend and parsing its output.
            tracing::warn!("judge agent loop condition not yet fully implemented at runtime");
            false
        }
        super::plan::LoopUntilCondition::Command(cmd) => {
            // Run the shell command; exit code 0 = condition met
            let result = tokio::process::Command::new("sh")
                .args(["-c", cmd])
                .current_dir(workspace_root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            result.map(|s| s.success()).unwrap_or(false)
        }
    }
}

fn next_escalation_tier(tier: crate::types::ModelTier) -> Option<crate::types::ModelTier> {
    use crate::types::ModelTier;
    match tier {
        ModelTier::Cheap => Some(ModelTier::Expensive),
        ModelTier::Expensive => None,
    }
}

/// No-op write gate observer for parallel agents (AutoAccept mode).
struct NoopWriteGateObserver;

impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}
