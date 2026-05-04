//! T4 — `ExecutionState::save` + `load` checkpoint round-trip integration test.
//!
//! Backs `gaviero-cli --resume`: the writer task records per-node state
//! into `.gaviero/state/<plan_hash>.json` after every commit, and a fresh
//! invocation reads it back to skip already-completed nodes. This test
//! pins the on-disk JSON shape against a real serde round-trip in a
//! tempdir so a breaking field-rename or path change shows up here
//! before it bites a user mid-resume.

use std::collections::HashMap;
use std::path::PathBuf;

use gaviero_core::swarm::execution_state::{ExecutionState, NodeStatus};
use gaviero_core::swarm::models::{AgentManifest, AgentStatus, WorkUnit};
use gaviero_core::swarm::plan::CompiledPlan;
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

fn manifest(id: &str, status: AgentStatus, modified: Vec<&str>, cost: f64) -> AgentManifest {
    AgentManifest {
        work_unit_id: id.to_string(),
        status,
        modified_files: modified.iter().map(PathBuf::from).collect(),
        branch: Some(format!("gaviero/{id}")),
        summary: Some(format!("ran {id}")),
        output: Some("hello".into()),
        cost_usd: cost,
    }
}

/// Run `f` with the working directory temporarily set to `dir`. Required
/// because `ExecutionState::save` / `load` use a hardcoded `.gaviero/state/`
/// path relative to CWD.
///
/// Cargo runs each integration-test binary as a single process, but
/// `cargo test` may schedule tests in this binary in parallel — guard
/// with a process-wide mutex so the chdir doesn't race across tests.
fn with_cwd<P: AsRef<std::path::Path>, R>(dir: P, f: impl FnOnce() -> R) -> R {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    let prev = std::env::current_dir().expect("getcwd");
    std::env::set_current_dir(dir.as_ref()).expect("set cwd to tempdir");
    let result = f();
    std::env::set_current_dir(prev).expect("restore cwd");
    result
}

#[test]
fn save_then_load_preserves_per_node_state() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let units = vec![
        unit("alpha", &["src/a.rs"]),
        unit("beta", &["src/b.rs"]),
        unit("gamma", &["src/c.rs"]),
    ];
    let plan = CompiledPlan::from_work_units(units, None);
    let plan_hash = plan.hash();

    // Build an in-memory state, mark nodes with three different terminal
    // statuses so the round-trip exercises every Status variant the
    // `record_result` path can produce.
    let mut state = ExecutionState::new_from_plan(&plan);
    state.record_result(
        "alpha",
        manifest("alpha", AgentStatus::Completed, vec!["src/a.rs"], 0.05),
    );
    state.record_result(
        "beta",
        manifest("beta", AgentStatus::Failed("model timeout".into()), vec![], 0.02),
    );
    // Leave `gamma` Pending — `load` must reconstruct it without a result.

    assert_eq!(state.status("alpha"), NodeStatus::Completed);
    assert_eq!(state.status("beta"), NodeStatus::HardFailure);
    assert_eq!(state.status("gamma"), NodeStatus::Pending);
    let pre_save_cost = state.cost_estimate_usd;

    with_cwd(tmp.path(), || {
        state.save(&plan_hash).expect("save state");

        let path = ExecutionState::checkpoint_path(&plan_hash);
        assert!(
            path.exists(),
            "checkpoint file must exist after save: {}",
            path.display()
        );

        let loaded = ExecutionState::load(&plan_hash)
            .expect("load state")
            .expect("checkpoint must be present");

        assert_eq!(loaded.status("alpha"), NodeStatus::Completed);
        assert_eq!(loaded.status("beta"), NodeStatus::HardFailure);
        assert_eq!(loaded.status("gamma"), NodeStatus::Pending);
        assert!(
            (loaded.cost_estimate_usd - pre_save_cost).abs() < 1e-9,
            "cost estimate must round-trip: {} vs {}",
            loaded.cost_estimate_usd,
            pre_save_cost,
        );
        // The recorded manifest survives — used by `--resume` to skip
        // completed nodes and surface their summaries.
        let alpha_state = loaded
            .node_states
            .get("alpha")
            .expect("alpha node present");
        let alpha_manifest = alpha_state.result.as_ref().expect("alpha manifest saved");
        assert_eq!(alpha_manifest.work_unit_id, "alpha");
        assert!(matches!(alpha_manifest.status, AgentStatus::Completed));
        assert_eq!(alpha_manifest.modified_files, vec![PathBuf::from("src/a.rs")]);
    });
}

#[test]
fn load_returns_none_when_no_checkpoint_exists() {
    let tmp = tempfile::tempdir().expect("tempdir");
    with_cwd(tmp.path(), || {
        let result = ExecutionState::load("ffffffffffffffff").expect("load is Ok-None");
        assert!(result.is_none(), "missing checkpoint must return Ok(None)");
    });
}

#[test]
fn checkpoint_path_uses_plan_hash() {
    let path = ExecutionState::checkpoint_path("deadbeef");
    let s = path.to_string_lossy();
    // Stable shape — `gaviero-cli --resume` and any external tooling
    // assume `.gaviero/state/<hash>.json`.
    assert!(s.ends_with("deadbeef.json"), "got {s}");
    assert!(s.contains(".gaviero"), "got {s}");
    assert!(s.contains("state"), "got {s}");
}
