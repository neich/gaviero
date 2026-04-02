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
use super::verify::{CostEstimate, EscalationRecord, EscalationReason};
use super::merge;
use crate::git::WorktreeManager;
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

                invalidate_stale_sources(&memory, unit, &config.workspace_root).await;

                let effective_read_ns: Vec<String> = unit.read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();

                let manifest = run_single_agent(
                    unit,
                    &config.workspace_root,
                    worktree_mgr.as_mut(),
                    &config.context_files,
                    memory.clone(),
                    &effective_read_ns,
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
                    let effective_write_ns = unit.write_namespace.as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(&memory, effective_write_ns, &manifest, unit, &run_id, &config.workspace_root).await;
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
                    ).await?;

                    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
                        let summary = manifest.summary.as_deref().unwrap_or("task complete").to_string();
                        let changed = commit_agent_changes(&agent_root, &unit.id, &summary)
                            .unwrap_or_else(|e| { tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e); vec![] });
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
        pre_swarm_sha,
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
    context_files: &[(String, String)],
    memory: Option<Arc<MemoryStore>>,
    read_namespaces: &[String],
    swarm_observer: &dyn SwarmObserver,
    acp_observer: Box<dyn AcpObserver>,
) -> Result<AgentManifest> {
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
    )
    .await?;

    swarm_observer.on_agent_state_changed(
        &unit.id,
        &manifest.status,
        manifest.summary.as_deref().unwrap_or(""),
    );

    // Commit changes and record branch name if running in a worktree
    if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
        let summary = manifest.summary.as_deref().unwrap_or("task complete").to_string();
        let changed = commit_agent_changes(&agent_root, &unit.id, &summary)
            .unwrap_or_else(|e| { tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e); vec![] });
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
        .status()
        .context("git add in worktree")?;
    anyhow::ensure!(add.success(), "git add failed in worktree {}", worktree_path.display());

    // Commit
    let msg = format!(
        "gaviero: agent {} — {}",
        agent_id,
        if summary.is_empty() { "task complete" } else { summary }
    );
    let commit = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(worktree_path)
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
    let content = format!(
        "Task: {}\nTier: {:?}\nModified: {}\nSummary: {}",
        unit.description,
        unit.tier,
        files.join(", "),
        manifest.summary.as_deref().unwrap_or("none"),
    );
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
    observer.on_tier_dispatch("coordinator", crate::types::ModelTier::Coordinator, &config.model);

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

/// Execute a coordinated swarm with tier routing.
///
/// # Deprecated
///
/// Use [`plan_coordinated`] + [`execute`] instead. This function combines
/// planning and execution in one shot with no user review step, which makes
/// phantom file references (files the coordinator expects agents to create)
/// invisible until they cause silent agent failures.
///
/// The existing `execute()` remains for backward compatibility with
/// non-coordinated swarm runs.
#[deprecated(since = "0.1.0", note = "use plan_coordinated() + execute() instead")]
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

    // Capture HEAD SHA before any merges (for revert support)
    let pre_swarm_sha = if config.use_worktrees {
        crate::git::current_head_sha(&config.workspace_root).unwrap_or_default()
    } else {
        String::new()
    };

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
            let ns: Vec<String> = unit.read_namespaces
                .as_deref()
                .unwrap_or(config.read_namespaces.as_slice())
                .to_vec();
            let obs = make_observer(unit_id);
            let run_id_clone = run_id.clone();
            let write_ns = unit.write_namespace.clone()
                .unwrap_or_else(|| config.write_namespace.clone());

            let in_worktree = worktree_mgr.is_some();
            let agent_root = if let Some(ref mut mgr) = worktree_mgr {
                match mgr.provision(&unit.id) {
                    Ok(handle) => {
                        let path = handle.path.clone();
                        // Inject @file context files so agents can Read them via their tools
                        if !config.context_files.is_empty() {
                            if let Err(e) = mgr.inject_context_files(&unit.id, &config.context_files) {
                                tracing::warn!("Failed to inject context files for {}: {}", unit.id, e);
                            }
                        }
                        path
                    }
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
                let mut manifest = match tokio::time::timeout(
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

                if in_worktree && matches!(manifest.status, AgentStatus::Completed) {
                    let summary = manifest.summary.as_deref().unwrap_or("task complete").to_string();
                    let changed = commit_agent_changes(&agent_root, &unit.id, &summary)
                        .unwrap_or_else(|e| { tracing::warn!("Failed to commit worktree changes for {}: {}", unit.id, e); vec![] });
                    manifest.modified_files = changed;
                    manifest.branch = Some(format!("gaviero/{}", unit.id));
                }

                // Store result to memory (privacy-aware)
                store_agent_result(&mem, &write_ns, &manifest, &unit, &run_id_clone, &root).await;

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

        // --- Retry failed agents with escalation ---
        let tier_failed_indices: Vec<usize> = (0..all_manifests.len())
            .filter(|&i| {
                let m = &all_manifests[i];
                tier.contains(&m.work_unit_id) && matches!(m.status, AgentStatus::Failed(_))
            })
            .collect();

        for &idx in &tier_failed_indices {
            let failed_id = all_manifests[idx].work_unit_id.clone();
            let failure_reason = match &all_manifests[idx].status {
                AgentStatus::Failed(msg) => msg.clone(),
                _ => "unknown".into(),
            };
            let original_unit = *unit_map.get(failed_id.as_str())
                .with_context(|| format!("work unit '{}' not found for retry", failed_id))?;

            let mut retry_unit = original_unit.clone();
            let mut attempts_left = retry_unit.max_retries;

            while attempts_left > 0 {
                let Some(escalation_tier) = retry_unit.escalation_tier else {
                    break; // No escalation path available
                };

                let from_tier = retry_unit.tier;
                retry_unit.tier = escalation_tier;
                retry_unit.escalation_tier = next_escalation_tier(escalation_tier);

                let backend = match router.resolve_backend(&retry_unit) {
                    Ok(b) => b,
                    Err(reason) => {
                        tracing::warn!(
                            "Retry escalation blocked for '{}': {}",
                            failed_id, reason
                        );
                        break;
                    }
                };

                observer.on_escalation(&EscalationRecord {
                    unit_id: failed_id.clone(),
                    reason: EscalationReason::AgentFailure {
                        reason: failure_reason.clone(),
                    },
                    from_tier,
                    to_tier: escalation_tier,
                    succeeded: false,
                });
                observer.on_agent_state_changed(
                    &failed_id,
                    &AgentStatus::Running,
                    &format!("retrying (escalated {:?} → {:?})", from_tier, escalation_tier),
                );

                // Retry must run in an isolated worktree, not the shared workspace.
                // Re-provision the worktree (provision() cleans up any stale state from
                // the failed attempt), inject context files, then commit on success.
                let retry_id = format!("{}-retry", failed_id);
                let retry_root = if let Some(ref mut mgr) = worktree_mgr {
                    match mgr.provision(&retry_id) {
                        Ok(handle) => {
                            let path = handle.path.clone();
                            if !config.context_files.is_empty() {
                                if let Err(e) = mgr.inject_context_files(&retry_id, &config.context_files) {
                                    tracing::warn!("Failed to inject context files for retry {}: {}", retry_id, e);
                                }
                            }
                            path
                        }
                        Err(e) => {
                            tracing::warn!("Worktree provision failed for retry '{}': {}; falling back to workspace", retry_id, e);
                            config.workspace_root.clone()
                        }
                    }
                } else {
                    config.workspace_root.clone()
                };
                let in_retry_worktree = worktree_mgr.is_some();

                let write_gate = Arc::new(Mutex::new(
                    WriteGatePipeline::new(WriteMode::AutoAccept, Box::new(NoopWriteGateObserver)),
                ));
                let retry_obs = make_observer(&failed_id);
                let retry_ns: Vec<String> = retry_unit.read_namespaces
                    .as_deref()
                    .unwrap_or(config.read_namespaces.as_slice())
                    .to_vec();

                let retry_result = tokio::time::timeout(
                    std::time::Duration::from_secs(600),
                    super::backend::runner::run_backend(
                        backend.as_ref(),
                        &retry_unit,
                        write_gate,
                        &retry_root,
                        memory.as_deref(),
                        &retry_ns,
                        retry_obs.as_ref(),
                    ),
                ).await;

                let mut retry_manifest = match retry_result {
                    Ok(Ok(m)) => m,
                    Ok(Err(e)) => AgentManifest {
                        work_unit_id: failed_id.clone(),
                        status: AgentStatus::Failed(format!("{:#}", e)),
                        modified_files: vec![],
                        branch: None,
                        summary: Some("Retry error".into()),
                    },
                    Err(_) => AgentManifest {
                        work_unit_id: failed_id.clone(),
                        status: AgentStatus::Failed("retry timed out".into()),
                        modified_files: vec![],
                        branch: None,
                        summary: Some("Retry timed out".into()),
                    },
                };

                observer.on_agent_state_changed(
                    &failed_id,
                    &retry_manifest.status,
                    retry_manifest.summary.as_deref().unwrap_or(""),
                );

                if matches!(retry_manifest.status, AgentStatus::Completed) {
                    // Commit the retry worktree and assign the branch for later merging
                    if in_retry_worktree {
                        let summary = retry_manifest.summary.as_deref().unwrap_or("retry complete").to_string();
                        let changed = commit_agent_changes(&retry_root, &failed_id, &summary)
                            .unwrap_or_else(|e| { tracing::warn!("Failed to commit retry worktree for {}: {}", failed_id, e); vec![] });
                        retry_manifest.modified_files = changed;
                        retry_manifest.branch = Some(format!("gaviero/{}", failed_id));
                    }
                    observer.on_escalation(&EscalationRecord {
                        unit_id: failed_id.clone(),
                        reason: EscalationReason::AgentFailure {
                            reason: failure_reason.clone(),
                        },
                        from_tier,
                        to_tier: escalation_tier,
                        succeeded: true,
                    });
                    let b = bus.lock().await;
                    b.broadcast(
                        &retry_manifest.work_unit_id,
                        &format!("completed (retry): {}", retry_manifest.summary.as_deref().unwrap_or("")),
                    );
                    let effective_write_ns = retry_unit.write_namespace.as_deref()
                        .unwrap_or(&config.write_namespace);
                    store_agent_result(&memory, effective_write_ns, &retry_manifest, &retry_unit, &run_id, &config.workspace_root).await;
                    all_manifests[idx] = retry_manifest;
                    break;
                }

                attempts_left -= 1;
            }
        }

        // Abort if any agent in this tier is still failed after retries
        let still_failed: Vec<String> = all_manifests.iter()
            .filter(|m| tier.contains(&m.work_unit_id) && matches!(m.status, AgentStatus::Failed(_)))
            .map(|m| {
                let err = match &m.status {
                    AgentStatus::Failed(msg) => msg.as_str(),
                    _ => "unknown",
                };
                format!("'{}': {}", m.work_unit_id, err)
            })
            .collect();

        if !still_failed.is_empty() {
            tracing::error!(
                "Aborting coordination: {} agent(s) failed after retries: {}",
                still_failed.len(),
                still_failed.join(", ")
            );
            observer.on_phase_changed("aborted");

            if let Some(ref mut mgr) = worktree_mgr {
                mgr.teardown_all();
            }

            let result = SwarmResult {
                manifests: all_manifests,
                merge_results: vec![],
                success: false,
                pre_swarm_sha: pre_swarm_sha.clone(),
            };
            observer.on_completed(&result);
            return Ok(result);
        }

        // Per-tier merge: immediately after each tier completes, merge its branches back
        // into the main workspace. This lets the NEXT tier's worktrees be provisioned
        // from an up-to-date HEAD that includes this tier's changes — essential for
        // multi-phase plans where later agents depend on files created by earlier agents.
        if config.use_worktrees {
            observer.on_phase_changed(&format!("merging tier {}", tier_idx + 1));
            let tier_completed_branches: Vec<(String, String)> = all_manifests
                .iter()
                .filter(|m| {
                    tier.contains(&m.work_unit_id)
                        && m.branch.is_some()
                        && matches!(m.status, AgentStatus::Completed)
                })
                .map(|m| (m.work_unit_id.clone(), m.branch.clone().unwrap()))
                .collect();

            for (unit_id, branch) in &tier_completed_branches {
                let merge_result = merge_one_branch(&config.workspace_root, branch, observer).await;
                // Backfill work_unit_id (merge_one_branch uses branch name as fallback)
                all_merges.push(MergeResult {
                    work_unit_id: unit_id.clone(),
                    ..merge_result
                });
            }
        }

        // Update cost estimate
        cost_estimate.estimated_usd += 0.01 * tier.len() as f64; // placeholder
        observer.on_cost_update(&cost_estimate);
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
        pre_swarm_sha,
    };

    observer.on_phase_changed("completed");
    observer.on_completed(&result);

    Ok(result)
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
/// Mechanical → Execution → Reasoning → None (ceiling reached).
fn next_escalation_tier(tier: crate::types::ModelTier) -> Option<crate::types::ModelTier> {
    use crate::types::ModelTier;
    match tier {
        ModelTier::Mechanical => Some(ModelTier::Execution),
        ModelTier::Execution => Some(ModelTier::Reasoning),
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
