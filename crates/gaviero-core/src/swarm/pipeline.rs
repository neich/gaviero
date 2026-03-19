//! Swarm pipeline: validates → tiers → parallel execution → merge.
//!
//! Orchestrates multi-agent execution with git worktree isolation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, Semaphore};

use super::bus::AgentBus;
use super::models::{AgentManifest, AgentStatus, MergeResult, SwarmResult, WorkUnit};
use super::runner::AgentRunner;
use super::validation;
use super::merge;
use crate::git::WorktreeManager;
use crate::memory::MemoryStore;
use crate::observer::{AcpObserver, SwarmObserver};
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
        Some(WorktreeManager::new(config.workspace_root.clone()))
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
                    store_agent_result(&memory, &config.write_namespace, &manifest, unit).await;
                }
                all_manifests.push(manifest);
                if failed {
                    break;
                }
            }
        } else {
            // Parallel execution within tier
            let mut handles = Vec::new();

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

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    let write_gate = Arc::new(Mutex::new(
                        WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
                    ));
                    let runner = AgentRunner::new(write_gate, agent_root, mem)
                        .with_read_namespaces(ns);
                    runner.run(&unit, obs, None).await
                }));
            }

            // Collect results
            for handle in handles {
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
                                store_agent_result(&memory, &config.write_namespace, &manifest, unit).await;
                            }
                        }
                        all_manifests.push(manifest);
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Agent task error: {}", e);
                    }
                    Err(e) => {
                        tracing::error!("Agent task panicked: {}", e);
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

    let write_gate = Arc::new(Mutex::new(
        WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
    ));
    let runner = AgentRunner::new(write_gate, agent_root, memory)
        .with_read_namespaces(read_namespaces.to_vec());

    let mut manifest = runner.run(unit, acp_observer, Some(swarm_observer)).await?;

    // Record branch name if using worktrees
    manifest.branch = Some(format!("gaviero/{}", unit.id));

    Ok(manifest)
}

/// Store an agent's execution result to memory (best-effort, never fails the pipeline).
async fn store_agent_result(
    memory: &Option<Arc<MemoryStore>>,
    write_ns: &str,
    manifest: &AgentManifest,
    unit: &WorkUnit,
) {
    let Some(mem) = memory else { return };
    let key = format!("agents:{}", manifest.work_unit_id);
    let files: Vec<String> = manifest.modified_files.iter()
        .map(|p| p.display().to_string())
        .collect();
    let content = format!(
        "Task: {}\nModified: {}\nSummary: {}",
        unit.description,
        files.join(", "),
        manifest.summary.as_deref().unwrap_or("none"),
    );
    if let Err(e) = mem.store(write_ns, &key, &content, None).await {
        tracing::warn!("Failed to store agent result to memory: {}", e);
    }
}

/// No-op write gate observer for parallel agents (AutoAccept mode).
struct NoopWriteGateObserver;

impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}
