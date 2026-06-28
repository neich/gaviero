//! T2 — `swarm::pipeline::execute` scope-validation gate is wired in.
//!
//! Verifies that scope overlap is rejected before any backend dispatch.
//! Build a `CompiledPlan` from two work units sharing an owned path, call
//! `execute`, and assert it errors out with the validation diagnostic.
//! This pins the orchestrator's "validate first" contract: a refactor
//! that drops the call to `validate_scopes` would let the test execute
//! agents with overlapping writes — the bug class this gate exists to
//! prevent.

use std::collections::HashMap;
use std::path::Path;

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::CompiledPlan;
use gaviero_core::swarm::models::WorkUnit;
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};

#[allow(deprecated)]
fn unit(id: &str, owned: &[&str]) -> WorkUnit {
    WorkUnit {
        id: id.to_string(),
        description: format!("test unit {id}"),
        scope: FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        },
        depends_on: vec![],
        backend: Default::default(),
        model: None,
        effort: None,
        extra: vec![],
        tier: ModelTier::Cheap,
        privacy: PrivacyLevel::Public,
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
        extra_allowed_tools: vec![],
    }
}

struct NoopSwarmObserver;
impl SwarmObserver for NoopSwarmObserver {
    fn on_phase_changed(&self, _phase: &str) {}
    fn on_agent_state_changed(
        &self,
        _work_unit_id: &str,
        _status: &gaviero_core::swarm::models::AgentStatus,
        _detail: &str,
    ) {
    }
    fn on_tier_started(&self, _current: usize, _total: usize) {}
    fn on_merge_conflict(&self, _branch: &str, _files: &[String]) {}
    fn on_completed(&self, _result: &gaviero_core::swarm::models::SwarmResult) {}
}

struct NoopAcpObserver;
impl AcpObserver for NoopAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(
        &self,
        _path: &Path,
        _old_content: Option<&str>,
        _new_content: &str,
    ) {
    }
}

fn make_config(workspace: &std::path::Path) -> SwarmConfig {
    SwarmConfig {
        execution_mode: gaviero_core::swarm::plan::ExecutionMode::Repo,
        max_parallel: 1,
        workspace_root: workspace.to_path_buf(),
        // Pipeline must abort at validation BEFORE this model spec is
        // even consulted; pick something obviously invalid so any future
        // bug that lets us reach backend resolution surfaces loudly.
        model: "claude:sonnet".to_string(),
        ollama_base_url: None,
        use_worktrees: false,
        read_namespaces: vec!["default".to_string()],
        write_namespace: "default".to_string(),
        context_files: vec![],
        worktree_context_paths: vec![],
        excludes: vec![],
        memory_writer: None,
        mcp_config: None,
        specificity: SpecificityConfig::default(),
        swarm_extra_tools: vec![],
        extract_agent_findings: false,
    }
}

#[tokio::test]
async fn execute_rejects_plans_with_overlapping_owned_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();

    // Two work units sharing the exact same owned path.
    let units = vec![
        unit("alpha", &["src/foo.rs"]),
        unit("beta", &["src/foo.rs"]),
    ];
    let plan = CompiledPlan::from_work_units(units, Some(2));

    let config = make_config(workspace);
    let observer = NoopSwarmObserver;
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let err = execute(&plan, &config, None, None, &observer, make_obs)
        .await
        .expect_err("execute must reject overlapping-scope plans");

    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("scope"),
        "error must mention scope, got: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("validation") || msg.contains("overlap"),
        "error must reference scope validation/overlap, got: {msg}"
    );
}

#[tokio::test]
async fn execute_rejects_plans_with_directory_prefix_overlap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();

    // Directory prefix containment: `src/` strictly contains `src/auth/login.rs`.
    let units = vec![
        unit("alpha", &["src/"]),
        unit("beta", &["src/auth/login.rs"]),
    ];
    let plan = CompiledPlan::from_work_units(units, Some(2));

    let config = make_config(workspace);
    let observer = NoopSwarmObserver;
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let err = execute(&plan, &config, None, None, &observer, make_obs)
        .await
        .expect_err("execute must reject prefix-containment scope overlap");

    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("scope"),
        "error must mention scope, got: {msg}"
    );
}
