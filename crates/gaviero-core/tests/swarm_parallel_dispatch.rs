//! Parallel dispatch through the real `swarm::pipeline::execute` path.
//!
//! Every other pipeline-level test runs with `max_parallel: 1`, which takes
//! the sequential tier branch. These drive the parallel branch and the
//! parallel loop passes against a mock Ollama server, and check what the
//! scheduler promises: a unit that depends on a parallel tier runs, a loop
//! whose first pass ran in parallel iterates, and `depends_on` between two
//! agents of the same loop orders them inside every pass.

use std::collections::HashMap;
use std::path::Path;

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::execution_state::{ExecutionState, NodeStatus};
use gaviero_core::swarm::models::{AgentStatus, WorkUnit};
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode, LoopConfig, LoopUntilCondition};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[allow(deprecated)]
fn unit(id: &str, owned: &[&str], instructions: &str, depends_on: &[&str]) -> WorkUnit {
    WorkUnit {
        id: id.to_string(),
        description: format!("test unit {id}"),
        scope: FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        },
        produces_json: vec![],
        produces: vec![],
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        backend: Default::default(),
        model: Some("ollama:qwen".to_string()),
        effort: None,
        extra: vec![],
        tier: ModelTier::Cheap,
        privacy: PrivacyLevel::Public,
        coordinator_instructions: instructions.to_string(),
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

/// A command-gated loop over `agent_ids` that never converges, so it runs
/// exactly `max_iterations` passes.
fn command_loop(agent_ids: &[&str], max_iterations: u32) -> LoopConfig {
    LoopConfig {
        agent_ids: agent_ids.iter().map(|s| s.to_string()).collect(),
        until: LoopUntilCondition::Command("exit 1".to_string()),
        max_iterations,
        iter_start: 1,
        strict_judge: true,
        stability: 1,
        judge_timeout_secs: 120,
        branch_chain: Default::default(),
        consensus_mode: Default::default(),
        verdict_output_dir: None,
        irreconcilable_after: 2,
        baseline_units: Vec::new(),
    }
}

struct QuietObserver;

impl SwarmObserver for QuietObserver {
    fn on_phase_changed(&self, _phase: &str) {}
    fn on_agent_state_changed(&self, _work_unit_id: &str, _status: &AgentStatus, _detail: &str) {}
    fn on_tier_started(&self, _current: usize, _total: usize) {}
    fn on_merge_conflict(&self, _branch: &str, _files: &[String]) {}
    fn on_completed(&self, _result: &gaviero_core::swarm::models::SwarmResult) {}
}

/// Records every agent status change as `(id, "pending" | "running" | …)`.
#[derive(Default)]
struct RecordingObserver {
    states: std::sync::Mutex<Vec<(String, &'static str)>>,
}

impl RecordingObserver {
    /// The statuses `id` went through, in order.
    fn of(&self, id: &str) -> Vec<&'static str> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .filter(|(agent, _)| agent == id)
            .map(|(_, status)| *status)
            .collect()
    }
}

impl SwarmObserver for RecordingObserver {
    fn on_phase_changed(&self, _phase: &str) {}
    fn on_agent_state_changed(&self, work_unit_id: &str, status: &AgentStatus, _detail: &str) {
        let label = match status {
            AgentStatus::Pending => "pending",
            AgentStatus::Running => "running",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed(_) => "failed",
        };
        self.states
            .lock()
            .unwrap()
            .push((work_unit_id.to_string(), label));
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
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

fn config_for(workspace: &Path, ollama_url: &str, max_parallel: usize) -> SwarmConfig {
    SwarmConfig {
        execution_mode: ExecutionMode::Document,
        max_parallel,
        workspace_root: workspace.to_path_buf(),
        model: "ollama:qwen".to_string(),
        ollama_base_url: Some(ollama_url.to_string()),
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
        tool_policy: None,
        exposed_tools: None,
        extract_agent_findings: false,
        resume_from_artifacts: false,
        knowledge_invalidation: None,
        // A scheduling bug that loops forever must fail the test, not hang it.
        run_timeout_secs: 120,
        chat_injection: Default::default(),
        skills_emit: Default::default(),
        skill_catalog: None,
        memory_root: None,
        graph_db_path: None,
        claims: None,
    }
}

/// A mock Ollama turn that writes `content` to `file`.
fn ndjson_turn(file: &str, content: &str) -> String {
    let body = format!("<file path=\"{file}\">\n{content}\n</file>");
    let chunk = serde_json::json!({
        "model": "qwen",
        "message": { "role": "assistant", "content": body },
        "done": false
    });
    let done = serde_json::json!({
        "model": "qwen",
        "done": true,
        "total_duration": 1_000_000,
        "eval_count": 2,
        "prompt_eval_count": 5
    });
    format!("{chunk}\n{done}")
}

/// Answer every prompt containing `marker` by writing `file`.
async fn answer(server: &MockServer, marker: &str, file: &str) {
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains(marker))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson_turn(file, "done"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(server)
        .await;
}

/// Every prompt the mock server was asked to complete, in arrival order.
async fn prompts_seen(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|req| String::from_utf8_lossy(&req.body).into_owned())
        .collect()
}

fn count_containing(prompts: &[String], needle: &str) -> usize {
    prompts.iter().filter(|p| p.contains(needle)).count()
}

fn first_index(prompts: &[String], needle: &str) -> Option<usize> {
    prompts.iter().position(|p| p.contains(needle))
}

fn make_obs(_id: &str) -> Box<dyn AcpObserver> {
    Box::new(NoopAcpObserver)
}

/// Units `a` and `b` share a tier (run in parallel); `c` depends on both.
fn fan_in_plan() -> CompiledPlan {
    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("a", &["out/a.md"], "Write the report. MARK-AAA", &[]),
            unit("b", &["out/b.md"], "Write the report. MARK-BBB", &[]),
            unit("c", &["out/c.md"], "Summarise. MARK-CCC", &["a", "b"]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan
}

async fn fan_in_server() -> MockServer {
    let server = MockServer::start().await;
    answer(&server, "MARK-AAA", "out/a.md").await;
    answer(&server, "MARK-BBB", "out/b.md").await;
    answer(&server, "MARK-CCC", "out/c.md").await;
    server
}

#[tokio::test]
async fn a_dependent_of_a_parallel_tier_runs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = fan_in_server().await;
    let plan = fan_in_plan();

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let c = result
        .manifests
        .iter()
        .find(|m| m.work_unit_id == "c")
        .expect("c has a manifest");
    assert_eq!(
        c.status,
        AgentStatus::Completed,
        "c depends on a parallel tier and must run, got {:?}",
        c.status
    );
    let prompts = prompts_seen(&server).await;
    assert_eq!(count_containing(&prompts, "MARK-CCC"), 1, "c was dispatched once");
}

#[tokio::test]
async fn parallel_tier_checkpoint_records_each_unit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = fan_in_server().await;
    let plan = fan_in_plan();

    execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let state = ExecutionState::load(workspace, &plan.hash())
        .expect("checkpoint readable")
        .expect("checkpoint written");
    assert_eq!(state.status("a"), NodeStatus::Completed);
    assert_eq!(state.status("b"), NodeStatus::Completed);
}

#[tokio::test]
async fn a_loop_agent_that_depends_on_a_sibling_runs_after_it_every_pass() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = MockServer::start().await;
    answer(&server, "MARK-INV", "out/inv.md").await;
    answer(&server, "MARK-CUR", "out/cur.md").await;

    // `cur` is listed first on purpose: the order must come from depends_on.
    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("inv", &["out/inv.md"], "MARK-INV iteration {{ITER}}", &[]),
            unit("cur", &["out/cur.md"], "MARK-CUR iteration {{ITER}}", &["inv"]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop(&["cur", "inv"], 2)];

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await;
    assert!(
        result.is_ok(),
        "a non-converging loop is not a run error: {:?}",
        result.err().map(|e| format!("{e:#}"))
    );

    let prompts = prompts_seen(&server).await;
    for iter in [1, 2] {
        let inv = first_index(&prompts, &format!("MARK-INV iteration {iter}"))
            .unwrap_or_else(|| panic!("inv did not run on pass {iter}"));
        let cur = first_index(&prompts, &format!("MARK-CUR iteration {iter}"))
            .unwrap_or_else(|| panic!("cur did not run on pass {iter}"));
        assert!(
            cur > inv,
            "pass {iter}: cur (request {cur}) must run after inv (request {inv})"
        );
    }
    assert_eq!(
        count_containing(&prompts, "MARK-CUR"),
        2,
        "cur runs once per pass and never again after the loop"
    );
}

#[tokio::test]
async fn a_loop_whose_first_pass_ran_in_parallel_iterates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = MockServer::start().await;
    answer(&server, "MARK-XXX", "out/x.md").await;
    answer(&server, "MARK-YYY", "out/y.md").await;

    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("x", &["out/x.md"], "Iteration {{ITER}}. MARK-XXX", &[]),
            unit("y", &["out/y.md"], "Iteration {{ITER}}. MARK-YYY", &[]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop(&["x", "y"], 2)];

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await;
    assert!(
        result.is_ok(),
        "a non-converging loop is not a run error: {:?}",
        result.err().map(|e| format!("{e:#}"))
    );

    let prompts = prompts_seen(&server).await;
    assert!(
        count_containing(&prompts, "MARK-XXX") >= 2,
        "x must run on pass 1 and pass 2, saw {}",
        count_containing(&prompts, "MARK-XXX")
    );
    assert!(
        count_containing(&prompts, "MARK-YYY") >= 2,
        "y must run on pass 1 and pass 2, saw {}",
        count_containing(&prompts, "MARK-YYY")
    );
}

/// Each run must be reported: a unit that finished without a `running` just
/// before it shows as queued for its whole run in the SWARM panel.
fn assert_each_run_reported(statuses: &[&str], id: &str, runs: usize) {
    let running = statuses.iter().filter(|s| **s == "running").count();
    assert_eq!(running, runs, "{id} reported running {running} times: {statuses:?}");
    for (i, status) in statuses.iter().enumerate() {
        if *status == "completed" {
            assert!(
                i > 0 && statuses[i - 1] == "running",
                "{id} completed without running first: {statuses:?}"
            );
        }
    }
}

#[tokio::test]
async fn parallel_tier_agents_are_reported_running_even_when_waiting_for_a_slot() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = MockServer::start().await;
    answer(&server, "MARK-AAA", "out/a.md").await;
    answer(&server, "MARK-BBB", "out/b.md").await;
    answer(&server, "MARK-DDD", "out/d.md").await;
    // Three units in one tier, two slots: one of them must wait.
    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("a", &["out/a.md"], "Write. MARK-AAA", &[]),
            unit("b", &["out/b.md"], "Write. MARK-BBB", &[]),
            unit("d", &["out/d.md"], "Write. MARK-DDD", &[]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    let observer = RecordingObserver::default();

    execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &observer,
        make_obs,
    )
    .await
    .expect("run succeeds");

    for id in ["a", "b", "d"] {
        assert_eq!(
            observer.of(id),
            vec!["pending", "running", "completed"],
            "{id}: queued, then running, then completed"
        );
    }
}

#[tokio::test]
async fn parallel_loop_pass_agents_are_reported_running_every_pass() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    let server = MockServer::start().await;
    answer(&server, "MARK-XXX", "out/x.md").await;
    answer(&server, "MARK-YYY", "out/y.md").await;
    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("x", &["out/x.md"], "Iteration {{ITER}}. MARK-XXX", &[]),
            unit("y", &["out/y.md"], "Iteration {{ITER}}. MARK-YYY", &[]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop(&["x", "y"], 2)];
    let observer = RecordingObserver::default();

    let _ = execute(
        &plan,
        &config_for(workspace, &server.uri(), 2),
        None,
        None,
        &observer,
        make_obs,
    )
    .await;

    let prompts = prompts_seen(&server).await;
    for (id, marker) in [("x", "MARK-XXX"), ("y", "MARK-YYY")] {
        let runs = count_containing(&prompts, marker);
        assert!(runs >= 2, "{id} ran {runs} times");
        assert_each_run_reported(&observer.of(id), id, runs);
    }
}
