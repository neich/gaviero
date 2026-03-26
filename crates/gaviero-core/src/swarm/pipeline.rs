//! Swarm pipeline: validates → tiers → parallel execution → merge.
//!
//! Orchestrates multi-agent execution with git worktree isolation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, Semaphore};

use super::bus::AgentBus;
use super::coordinator::{Coordinator, CoordinatorConfig};
use super::models::{AgentManifest, AgentStatus, MergeResult, SwarmResult, WorkUnit};
use super::router::{TierConfig, TierRouter};
use super::validation;
use super::verify::CostEstimate;
use super::merge;
use crate::git::WorktreeManager;
use crate::memory::MemoryStore;
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
}

/// Execute a swarm of work units.
///
/// 1. Validate scopes (no overlaps)
/// 2. Compute dependency tiers
/// 3. For each tier: provision worktrees, run agents in parallel, collect manifests
/// 4. Merge agent branches into main
/// 5. Return SwarmResult
pub async fn execute(
    work_units: Vec<WorkUnit>,
    config: &SwarmConfig,
    memory: Option<Arc<MemoryStore>>,
    observer: &dyn SwarmObserver,
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver>,
) -> Result<SwarmResult> {
    // Generate a unique run ID for this execution
    let run_id = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    observer.on_phase_changed("validating");

    // 1. Validate scopes
    let scope_errors = validation::validate_scopes(&work_units);
    if !scope_errors.is_empty() {
        let msg = scope_errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
        anyhow::bail!("scope validation failed: {}", msg);
    }

    // 2. Compute dependency tiers
    let tiers = validation::dependency_tiers(&work_units)
        .map_err(|e| anyhow::anyhow!("dependency cycle: {}", e))?;

    // Build lookup map
    let unit_map: std::collections::HashMap<&str, &WorkUnit> =
        work_units.iter().map(|u| (u.id.as_str(), u)).collect();

    let mut all_manifests: Vec<AgentManifest> = Vec::new();
    let mut all_merges: Vec<MergeResult> = Vec::new();
    let semaphore = Arc::new(Semaphore::new(config.max_parallel));

    // Inter-agent communication bus (available for future coordination)
    let bus = Arc::new(tokio::sync::Mutex::new(AgentBus::new()));
    // Register all agents upfront so they can send messages to each other
    {
        let mut b = bus.lock().await;
        for unit in &work_units {
            b.register(&unit.id);
        }
    }

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

        if config.max_parallel <= 1 || tier.len() <= 1 {
            // Sequential execution
            for unit_id in tier {
                let unit = unit_map.get(unit_id.as_str())
                    .with_context(|| format!("work unit '{}' not found", unit_id))?;

                observer.on_agent_state_changed(
                    unit_id,
                    &AgentStatus::Running,
                    &unit.description,
                );

                let manifest = run_single_agent(
                    unit,
                    &config.workspace_root,
                    worktree_mgr.as_mut(),
                    memory.clone(),
                    &config.read_namespaces,
                    observer,
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
                    store_agent_result(&memory, &config.write_namespace, &manifest, unit, &run_id).await;
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
                let ns = config.read_namespaces.clone();
                let obs = make_observer(unit_id);

                // Provision worktree if enabled
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

                    let write_gate = Arc::new(Mutex::new(
                        WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
                    ));
                    super::backend::runner::run_backend(
                        &backend,
                        &unit,
                        write_gate,
                        &agent_root,
                        mem.as_deref(),
                        &ns,
                        obs.as_ref(),
                    ).await
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
                                store_agent_result(&memory, &config.write_namespace, &manifest, unit, &run_id).await;
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
                            });
                        }
                    }
                }
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
    };

    observer.on_phase_changed("completed");
    observer.on_completed(&result);

    Ok(result)
}

/// Run a single agent, optionally in a worktree.
async fn run_single_agent(
    unit: &WorkUnit,
    workspace_root: &PathBuf,
    worktree_mgr: Option<&mut WorktreeManager>,
    memory: Option<Arc<MemoryStore>>,
    read_namespaces: &[String],
    swarm_observer: &dyn SwarmObserver,
    acp_observer: Box<dyn AcpObserver>,
) -> Result<AgentManifest> {
    let agent_root = if let Some(mgr) = worktree_mgr {
        let handle = mgr.provision(&unit.id)?;
        handle.path.clone()
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
    )
    .await?;

    swarm_observer.on_agent_state_changed(
        &unit.id,
        &manifest.status,
        manifest.summary.as_deref().unwrap_or(""),
    );

    // Record branch name if using worktrees
    manifest.branch = Some(format!("gaviero/{}", unit.id));

    Ok(manifest)
}

/// Store an agent's execution result to memory (best-effort, never fails the pipeline).
///
/// Uses `store_with_privacy` to tag entries with the unit's privacy level.
/// The `run_id` is included in the key pattern for cross-run continuity.
async fn store_agent_result(
    memory: &Option<Arc<MemoryStore>>,
    write_ns: &str,
    manifest: &AgentManifest,
    unit: &WorkUnit,
    run_id: &str,
) {
    let Some(mem) = memory else { return };
    let key = format!("agents:{}:{}", run_id, manifest.work_unit_id);
    let files: Vec<String> = manifest.modified_files.iter()
        .map(|p| p.display().to_string())
        .collect();
    let content = format!(
        "Task: {}\nTier: {:?}\nModified: {}\nSummary: {}",
        unit.description,
        unit.tier,
        files.join(", "),
        manifest.summary.as_deref().unwrap_or("none"),
    );
    let privacy = match unit.privacy {
        PrivacyLevel::LocalOnly => "local_only",
        PrivacyLevel::Public => "public",
    };
    let metadata = EntryMetadata {
        privacy: unit.privacy,
        format_version: 1,
        source: "swarm_pipeline".into(),
    };
    let metadata_json = serde_json::to_string(&metadata).ok();
    if let Err(e) = mem.store_with_privacy(
        write_ns,
        &key,
        &content,
        privacy,
        metadata_json.as_deref(),
    ).await {
        tracing::warn!("Failed to store agent result to memory: {}", e);
    }
}

/// Execute a coordinated swarm with tier routing.
///
/// This is the new entry point for the tier routing architecture.
/// The coordinator produces a TaskDAG, then the pipeline dispatches
/// each unit to the appropriate backend via `TierRouter`.
///
/// The existing `execute()` remains for backward compatibility with
/// non-coordinated swarm runs.
pub async fn execute_coordinated(
    prompt: &str,
    config: &SwarmConfig,
    tier_config: TierConfig,
    coordinator_config: CoordinatorConfig,
    memory: Option<Arc<MemoryStore>>,
    observer: &dyn SwarmObserver,
    make_observer: impl Fn(&str) -> Box<dyn AcpObserver>,
) -> Result<SwarmResult> {
    let run_id = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    // Phase 0: PLAN — coordinator produces TaskDAG
    observer.on_coordination_started(prompt);

    // Register a "coordinator" pseudo-agent so the dashboard shows streaming progress
    observer.on_agent_state_changed("coordinator", &AgentStatus::Running, "Opus planning...");
    observer.on_tier_dispatch("coordinator", crate::types::ModelTier::Coordinator, &config.model);

    let file_list = collect_file_list(&config.workspace_root)?;
    let coordinator = Coordinator::new(memory.clone(), coordinator_config);
    let coord_observer = make_observer("coordinator");
    let dag = match coordinator.plan(
        prompt,
        &config.workspace_root,
        &file_list,
        &config.read_namespaces,
        Some(coord_observer),
    ).await {
        Ok(dag) => {
            observer.on_agent_state_changed(
                "coordinator",
                &AgentStatus::Completed,
                &format!("Planned {} agents", dag.units.len()),
            );
            dag
        }
        Err(e) => {
            observer.on_agent_state_changed(
                "coordinator",
                &AgentStatus::Failed(e.to_string()),
                &e.to_string(),
            );
            return Err(e);
        }
    };

    observer.on_coordination_complete(&dag);

    // Phase 1: VALIDATE (scopes + dependencies already validated by coordinator)
    observer.on_phase_changed("validating");
    let tiers = validation::dependency_tiers(&dag.units)
        .map_err(|e| anyhow::anyhow!("dependency cycle: {}", e))?;

    // Initialize tier router
    let router = TierRouter::new(tier_config.clone(), false); // TODO: Ollama health check in Phase 3

    // Build lookup map
    let unit_map: std::collections::HashMap<&str, &WorkUnit> =
        dag.units.iter().map(|u| (u.id.as_str(), u)).collect();

    let mut all_manifests: Vec<AgentManifest> = Vec::new();
    let mut all_merges: Vec<MergeResult> = Vec::new();
    let mut cost_estimate = CostEstimate::default();

    // Per-model-tier semaphores
    let sem_reasoning = Arc::new(Semaphore::new(tier_config.reasoning_max_parallel));
    let sem_execution = Arc::new(Semaphore::new(tier_config.execution_max_parallel));
    let sem_mechanical = Arc::new(Semaphore::new(tier_config.mechanical.max_parallel));

    let bus = Arc::new(tokio::sync::Mutex::new(AgentBus::new()));
    {
        let mut b = bus.lock().await;
        for unit in &dag.units {
            b.register(&unit.id);
        }
    }

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

    // Phase 2: EXECUTE
    let tier_summary: Vec<String> = tiers.iter().enumerate()
        .map(|(i, t)| format!("T{}: {} agents", i, t.len()))
        .collect();
    tracing::info!("Execution plan: {} tiers [{}]", tiers.len(), tier_summary.join(", "));
    observer.on_phase_changed(&format!("running ({} tiers)", tiers.len()));

    for (tier_idx, tier) in tiers.iter().enumerate() {
        observer.on_tier_started(tier_idx + 1, tiers.len());

        // Register all agents in this tier as Pending + dispatch tier info
        for unit_id in tier {
            if let Some(unit) = unit_map.get(unit_id.as_str()) {
                observer.on_agent_state_changed(
                    unit_id,
                    &AgentStatus::Pending,
                    &unit.description,
                );
                let backend_str = match router.resolve_backend(unit) {
                    Ok(b) => b.name().to_string(),
                    Err(reason) => format!("blocked:{}", reason),
                };
                observer.on_tier_dispatch(unit_id, unit.tier, &backend_str);
            }
        }

        let mut handles = Vec::new();

        for unit_id in tier {
            let unit = (*unit_map.get(unit_id.as_str())
                .with_context(|| format!("work unit '{}' not found", unit_id))?)
                .clone();

            // Select the appropriate per-model-tier semaphore
            let sem = match unit.tier {
                crate::types::ModelTier::Reasoning | crate::types::ModelTier::Coordinator => sem_reasoning.clone(),
                crate::types::ModelTier::Execution => sem_execution.clone(),
                crate::types::ModelTier::Mechanical => sem_mechanical.clone(),
            };

            let root = config.workspace_root.clone();
            let mem = memory.clone();
            let ns = config.read_namespaces.clone();
            let obs = make_observer(unit_id);
            let run_id_clone = run_id.clone();
            let write_ns = config.write_namespace.clone();

            let agent_root = if let Some(ref mut mgr) = worktree_mgr {
                match mgr.provision(&unit.id) {
                    Ok(handle) => handle.path.clone(),
                    Err(e) => {
                        let err_msg = format!("worktree provision failed: {}", e);
                        observer.on_agent_state_changed(
                            &unit.id,
                            &AgentStatus::Failed(err_msg.clone()),
                            &err_msg,
                        );
                        all_manifests.push(AgentManifest {
                            work_unit_id: unit.id.clone(),
                            status: AgentStatus::Failed(err_msg),
                            modified_files: vec![],
                            branch: None,
                            summary: Some("Worktree provision failed".into()),
                        });
                        continue;
                    }
                }
            } else {
                root.clone()
            };

            // Resolve to a trait-object backend
            let backend = match router.resolve_backend(&unit) {
                Ok(b) => b,
                Err(reason) => {
                    all_manifests.push(AgentManifest {
                        work_unit_id: unit.id.clone(),
                        status: AgentStatus::Failed(format!("Blocked: {}", reason)),
                        modified_files: vec![],
                        branch: None,
                        summary: Some(format!("Blocked: {}", reason)),
                    });
                    continue;
                }
            };

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                let write_gate = Arc::new(Mutex::new(
                    WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
                ));

                // 10-minute timeout per agent to prevent blocking
                let agent_future = super::backend::runner::run_backend(
                    backend.as_ref(),
                    &unit,
                    write_gate,
                    &agent_root,
                    mem.as_deref(),
                    &ns,
                    obs.as_ref(),
                );
                let manifest = match tokio::time::timeout(
                    std::time::Duration::from_secs(600),
                    agent_future,
                ).await {
                    Ok(result) => result?,
                    Err(_) => {
                        return Ok(AgentManifest {
                            work_unit_id: unit.id.clone(),
                            status: AgentStatus::Failed("timed out after 10 minutes".into()),
                            modified_files: vec![],
                            branch: None,
                            summary: Some("Agent timed out".into()),
                        });
                    }
                };

                // Store result to memory (privacy-aware)
                store_agent_result(&mem, &write_ns, &manifest, &unit, &run_id_clone).await;

                Ok::<_, anyhow::Error>(manifest)
            }));
        }

        // Collect results for this dependency tier
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
                    }
                    all_manifests.push(manifest);
                }
                Ok(Err(e)) => {
                    let err_msg = format!("{:#}", e);
                    tracing::error!("Agent task error: {}", err_msg);
                    // Find the unit ID for this handle and report failure
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
                        });
                    }
                }
            }
        }

        // Update cost estimate
        cost_estimate.estimated_usd += 0.01 * tier.len() as f64; // placeholder
        observer.on_cost_update(&cost_estimate);
    }

    // Phase 3: MERGE
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

                        observer.on_phase_changed("resolving conflicts");
                        let resolved = merge::auto_resolve_conflicts(
                            &config.workspace_root,
                            branch,
                            &result.conflicts,
                            "opus", // Use Opus for conflict resolution in coordinated mode
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

    // Phase 4: VERIFY
    observer.on_phase_changed("verifying");
    let verification_report = super::verify::combined::run_verification(
        &dag.verification_strategy,
        &all_manifests,
        &dag.units,
        &config.workspace_root,
        observer,
    ).await?;

    if !verification_report.overall_passed {
        tracing::warn!(
            "Verification failed: {} escalations",
            verification_report.escalations_performed.len()
        );
    }

    // Post-verification: store tier stats + verification summary + prune
    {
        use super::calibration;

        let stats = calibration::TierStats::from_results(&all_manifests, &dag.units);
        calibration::store_tier_stats(&memory, &config.write_namespace, &run_id, &stats).await;
        calibration::store_verification_summary(
            &memory,
            &config.write_namespace,
            &run_id,
            verification_report.overall_passed,
            verification_report.escalations_performed.len(),
            &dag.plan_summary,
        ).await;

        // Prune old entries (best-effort)
        if let Some(ref mem) = memory {
            if let Err(e) = mem.prune(&config.write_namespace, 30, 50).await {
                tracing::warn!("Memory pruning failed: {}", e);
            }
        }
    }

    // Teardown
    if let Some(ref mut mgr) = worktree_mgr {
        mgr.teardown_all();
    }

    let success = all_manifests.iter().all(|m| matches!(m.status, AgentStatus::Completed))
        && all_merges.iter().all(|m| m.success);

    let result = SwarmResult {
        manifests: all_manifests,
        merge_results: all_merges,
        success,
    };

    observer.on_phase_changed("completed");
    observer.on_completed(&result);

    Ok(result)
}

/// Collect a list of files in the workspace for coordinator context.
fn collect_file_list(workspace_root: &PathBuf) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect_files_recursive(workspace_root, workspace_root, &mut files, 0);
    Ok(files)
}

fn collect_files_recursive(root: &PathBuf, dir: &PathBuf, files: &mut Vec<String>, depth: usize) {
    if depth > 10 || files.len() > 500 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip hidden dirs and common non-source dirs
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(root, &path, files, depth + 1);
        } else if let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.to_string_lossy().to_string());
        }
    }
}

/// No-op write gate observer for parallel agents (AutoAccept mode).
struct NoopWriteGateObserver;

impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}
