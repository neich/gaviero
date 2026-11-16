//! Runtime-sized `for` and `pattern map_reduce`, through the real
//! `swarm::pipeline::execute` path against a mock Ollama server.
//!
//! Plans are built by hand: `gaviero-core` cannot depend on the DSL. A
//! template unit `worker` carries `{{q.*}}` placeholders, and a
//! `RuntimeForOp` tells the scheduler to clone it once per item of a JSON
//! list the test writes into the workspace.

use std::collections::HashMap;
use std::path::Path;

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::execution_state::{ExecutionState, NodeStatus};
use gaviero_core::swarm::models::{AgentStatus, SwarmResult, WorkUnit};
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::{
    CompiledPlan, ExecutionMode, FanoutOp, LoopConfig, LoopUntilCondition, RuntimeForOp,
};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[allow(deprecated)]
fn unit(
    id: &str,
    owned: &[&str],
    produces: &[&str],
    instructions: &str,
    depends_on: &[&str],
) -> WorkUnit {
    WorkUnit {
        id: id.to_string(),
        description: format!("test unit {id}"),
        scope: FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        },
        produces_json: vec![],
        produces: produces.iter().map(|s| s.to_string()).collect(),
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

/// The template every test clones: one finding per item.
fn worker() -> WorkUnit {
    unit(
        "worker",
        &["out/{{q.id}}.md"],
        &["out/{{q.id}}.md"],
        "MARK-{{q.id}}: investigate focus={{q.focus}}",
        &[],
    )
}

fn op(source: &str, max_items: u32) -> RuntimeForOp {
    RuntimeForOp {
        unit_id: "worker".into(),
        var: "q".into(),
        source: source.into(),
        max_items,
    }
}

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
    fn on_completed(&self, _result: &SwarmResult) {}
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
    fn on_completed(&self, _result: &SwarmResult) {}
}

struct NoopAcpObserver;
impl AcpObserver for NoopAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

fn make_obs(_id: &str) -> Box<dyn AcpObserver> {
    Box::new(NoopAcpObserver)
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
        run_timeout_secs: 120,
        chat_injection: Default::default(),
        skills_emit: Default::default(),
        skill_catalog: None,
        memory_root: None,
        graph_db_path: None,
    }
}

fn ndjson(content: &str) -> String {
    let chunk = serde_json::json!({
        "model": "qwen",
        "message": { "role": "assistant", "content": content },
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
                .set_body_string(ndjson(&format!("<file path=\"{file}\">\ndone\n</file>")))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(server)
        .await;
}

/// Answer every prompt containing `marker` with prose only (no file).
async fn answer_prose(server: &MockServer, marker: &str) {
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains(marker))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson("I looked into it."))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(server)
        .await;
}

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

fn last_index(prompts: &[String], needle: &str) -> Option<usize> {
    prompts.iter().rposition(|p| p.contains(needle))
}

fn first_index(prompts: &[String], needle: &str) -> Option<usize> {
    prompts.iter().position(|p| p.contains(needle))
}

fn workspace_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("out")).unwrap();
    for (rel, content) in files {
        std::fs::write(tmp.path().join(rel), content).unwrap();
    }
    tmp
}

/// `worker` over `out/items.json`, an unrelated `side` unit in the same tier
/// (so the parallel tier branch runs), and `summary` depending on `worker`.
fn top_level_plan(max_items: u32) -> CompiledPlan {
    let mut plan = CompiledPlan::from_work_units(
        vec![
            worker(),
            unit("side", &["out/side.md"], &[], "MARK-SIDE", &[]),
            unit("summary", &["out/summary.md"], &[], "MARK-SUM", &["worker"]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan.runtime_fors = vec![op("out/items.json", max_items)];
    plan
}

async fn standard_server() -> MockServer {
    let server = MockServer::start().await;
    for id in ["a", "b", "c"] {
        answer(&server, &format!("MARK-{id}:"), &format!("out/{id}.md")).await;
    }
    answer(&server, "MARK-SIDE", "out/side.md").await;
    answer(&server, "MARK-SUM", "out/summary.md").await;
    server
}

const THREE_ITEMS: &str =
    r#"[{"id":"a","focus":"x"},{"id":"b","focus":"y"},{"id":"c","focus":"z"}]"#;

fn manifest_status(result: &SwarmResult, id: &str) -> AgentStatus {
    result
        .manifests
        .iter()
        .rev()
        .find(|m| m.work_unit_id == id)
        .unwrap_or_else(|| panic!("no manifest for {id}"))
        .status
        .clone()
}

#[tokio::test]
async fn top_level_runtime_for_runs_every_item_then_the_dependent() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = standard_server().await;
    let plan = top_level_plan(16);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let prompts = prompts_seen(&server).await;
    assert_eq!(count_containing(&prompts, "MARK-a: investigate focus=x"), 1);
    assert_eq!(count_containing(&prompts, "MARK-b:"), 1);
    assert_eq!(count_containing(&prompts, "MARK-c:"), 1);
    assert_eq!(manifest_status(&result, "summary"), AgentStatus::Completed);
    let sum = first_index(&prompts, "MARK-SUM").expect("summary ran");
    for id in ["a", "b", "c"] {
        assert!(
            last_index(&prompts, &format!("MARK-{id}:")).unwrap() < sum,
            "summary must run after every item"
        );
    }

    let state = ExecutionState::load(tmp.path(), &plan.hash())
        .unwrap()
        .expect("checkpoint written");
    for id in ["worker@a", "worker@b", "worker@c", "worker"] {
        assert_eq!(state.status(id), NodeStatus::Completed, "{id}");
    }
}

#[tokio::test]
async fn runtime_for_clones_and_their_template_are_reported_running() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = standard_server().await;
    let plan = top_level_plan(16);
    let observer = RecordingObserver::default();

    // Three clones, two slots: one clone waits for a slot.
    execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &observer,
        make_obs,
    )
    .await
    .expect("run succeeds");

    for id in ["worker@a", "worker@b", "worker@c"] {
        assert_eq!(
            observer.of(id),
            vec!["pending", "running", "completed"],
            "{id}: queued, then running, then completed"
        );
    }
    assert_eq!(
        observer.of("worker"),
        vec!["pending", "running", "completed"],
        "the template runs while its clones do"
    );
}

#[tokio::test]
async fn resume_skips_completed_items() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = standard_server().await;
    let plan = top_level_plan(16);
    let mut state = ExecutionState::new_from_plan(&plan);
    state.set_status("worker@a", NodeStatus::Completed);
    state.save(tmp.path(), &plan.hash()).unwrap();

    execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        Some(state),
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let prompts = prompts_seen(&server).await;
    assert_eq!(count_containing(&prompts, "MARK-a:"), 0, "a was already done");
    assert_eq!(count_containing(&prompts, "MARK-b:"), 1);
    assert_eq!(count_containing(&prompts, "MARK-c:"), 1);
    assert_eq!(count_containing(&prompts, "MARK-SUM"), 1);
}

#[tokio::test]
async fn over_cap_aborts_the_run() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = standard_server().await;
    let plan = top_level_plan(2);

    let err = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect_err("three items over `max 2` must abort the run");
    assert!(format!("{err:#}").contains("max 2"), "{err:#}");
    assert_eq!(count_containing(&prompts_seen(&server).await, "MARK-a:"), 0);
}

#[tokio::test]
async fn empty_list_runs_the_dependent() {
    let tmp = workspace_with(&[("out/items.json", "[]")]);
    let server = standard_server().await;
    let plan = top_level_plan(16);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let prompts = prompts_seen(&server).await;
    assert_eq!(count_containing(&prompts, "investigate"), 0);
    assert_eq!(manifest_status(&result, "worker"), AgentStatus::Completed);
    assert_eq!(manifest_status(&result, "summary"), AgentStatus::Completed);
}

#[tokio::test]
async fn a_failed_item_skips_the_dependent() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = MockServer::start().await;
    answer(&server, "MARK-a:", "out/a.md").await;
    answer_prose(&server, "MARK-b:").await;
    answer(&server, "MARK-c:", "out/c.md").await;
    answer(&server, "MARK-SIDE", "out/side.md").await;
    answer(&server, "MARK-SUM", "out/summary.md").await;
    let plan = top_level_plan(16);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("a failed item is not a run error");

    assert!(matches!(manifest_status(&result, "worker@b"), AgentStatus::Failed(_)));
    match manifest_status(&result, "worker") {
        AgentStatus::Failed(msg) => assert!(msg.contains("worker@b"), "{msg}"),
        other => panic!("the barrier must fail, got {other:?}"),
    }
    let summary = result
        .manifests
        .iter()
        .find(|m| m.work_unit_id == "summary")
        .expect("summary has a skip record");
    assert_eq!(summary.summary.as_deref(), Some("dependency did not complete"));
    assert_eq!(count_containing(&prompts_seen(&server).await, "MARK-SUM"), 0);
}

#[tokio::test]
async fn unknown_item_placeholder_aborts_before_dispatch() {
    let tmp = workspace_with(&[("out/items.json", THREE_ITEMS)]);
    let server = standard_server().await;
    let mut plan = top_level_plan(16);
    // Swap the template for one that asks for a field no item has.
    let mut bad = worker();
    bad.coordinator_instructions = "MARK-{{q.id}}: {{q.nope}}".into();
    plan = {
        let mut units: Vec<WorkUnit> = plan.work_units_unordered().into_iter().cloned().collect();
        units.retain(|u| u.id != "worker");
        units.push(bad);
        let mut p = CompiledPlan::from_work_units(units, Some(2));
        p.execution_mode = ExecutionMode::Document;
        p.runtime_fors = std::mem::take(&mut plan.runtime_fors);
        p
    };

    let err = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect_err("an unknown item placeholder aborts the run");
    assert!(format!("{err:#}").contains("{{q.nope}}"), "{err:#}");
    assert_eq!(count_containing(&prompts_seen(&server).await, "MARK-a:"), 0);
}

/// A loop over a runtime `for` whose list is versioned per pass.
fn loop_plan(with_curator: bool) -> CompiledPlan {
    let template = unit(
        "worker",
        &["out/{{q.id}}-v*.md"],
        &["out/{{q.id}}-v{{ITER}}.md"],
        "MARK-{{q.id}}: investigate focus={{q.focus}} pass {{ITER}}",
        &[],
    );
    let mut units = vec![template];
    let mut members = vec!["worker"];
    if with_curator {
        units.push(unit(
            "cur",
            &["out/cur-v*.md"],
            &[],
            "MARK-CUR iteration {{ITER}}",
            &["worker"],
        ));
        members.push("cur");
    }
    let mut plan = CompiledPlan::from_work_units(units, Some(2));
    plan.execution_mode = ExecutionMode::Document;
    plan.runtime_fors = vec![op("out/items-v{{ITER}}.json", 16)];
    plan.loop_configs = vec![command_loop(&members, 2)];
    plan
}

async fn loop_server() -> MockServer {
    let server = MockServer::start().await;
    answer(&server, "MARK-a:", "out/a-v1.md").await;
    answer(&server, "MARK-b:", "out/b-v1.md").await;
    answer(&server, "MARK-c:", "out/c-v2.md").await;
    answer(&server, "MARK-CUR", "out/cur-v1.md").await;
    server
}

const PASS_ONE: &str = r#"[{"id":"a","focus":"x"},{"id":"b","focus":"y"}]"#;
const PASS_TWO: &str = r#"[{"id":"c","focus":"z"}]"#;

#[tokio::test]
async fn runtime_for_inside_a_loop_rereads_its_list_each_pass() {
    let tmp = workspace_with(&[("out/items-v1.json", PASS_ONE), ("out/items-v2.json", PASS_TWO)]);
    let server = loop_server().await;
    let plan = loop_plan(false);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
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
    assert_eq!(count_containing(&prompts, "MARK-a: investigate focus=x pass 1"), 1);
    assert_eq!(count_containing(&prompts, "MARK-b: investigate focus=y pass 1"), 1);
    assert_eq!(count_containing(&prompts, "MARK-c: investigate focus=z pass 2"), 1);
    assert_eq!(count_containing(&prompts, "investigate"), 3, "one prompt per item per pass");
}

#[tokio::test]
async fn a_curator_after_runtime_for_runs_each_pass() {
    let tmp = workspace_with(&[("out/items-v1.json", PASS_ONE), ("out/items-v2.json", PASS_TWO)]);
    let server = loop_server().await;
    let plan = loop_plan(true);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
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
    let cur1 = first_index(&prompts, "MARK-CUR iteration 1").expect("curator ran on pass 1");
    let cur2 = first_index(&prompts, "MARK-CUR iteration 2").expect("curator ran on pass 2");
    for marker in ["MARK-a:", "MARK-b:"] {
        assert!(last_index(&prompts, marker).unwrap() < cur1, "{marker} before curator pass 1");
    }
    assert!(last_index(&prompts, "MARK-c:").unwrap() < cur2, "MARK-c before curator pass 2");
    assert!(first_index(&prompts, "MARK-c:").unwrap() > cur1, "pass 2 starts after pass 1");
    assert_eq!(count_containing(&prompts, "MARK-CUR"), 2);
}

// ── pattern map_reduce on the shared dispatcher ─────────────────────────

fn spawn_manifest(workers: &[&str]) -> String {
    let workers: Vec<serde_json::Value> = workers
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w,
                "tier": "local",
                "prompt": format!("MARK-W-{w}: work"),
                "owned": [format!("out/{w}.md")],
            })
        })
        .collect();
    serde_json::json!({ "version": 1, "workers": workers }).to_string()
}

/// `d` discovers (writes the manifest), `side` shares its tier so the
/// parallel tier branch runs, and `r` reduces after `d`.
fn map_reduce_plan(max_spawn: u32) -> CompiledPlan {
    let mut plan = CompiledPlan::from_work_units(
        vec![
            // `produces` makes the runner apply the mock's `<file>` block.
            unit("d", &["spawn_manifest.json"], &["spawn_manifest.json"], "MARK-DISCOVER", &[]),
            unit("side", &["out/side.md"], &["out/side.md"], "MARK-SIDE", &[]),
            unit("r", &["out/reduce.md"], &[], "MARK-REDUCE", &["d"]),
        ],
        Some(2),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan.fanout_ops = vec![FanoutOp {
        after_unit: "d".into(),
        max_spawn,
        default_model: None,
        barrier_id: None,
    }];
    // The profile tier the manifest's workers name.
    plan.tier_bindings.insert(
        "local".into(),
        gaviero_core::swarm::plan::TierBinding {
            model: Some("ollama:qwen".into()),
            effort: None,
            extra: vec![],
            privacy: Default::default(),
            tier: Default::default(),
        },
    );
    plan
}

async fn map_reduce_server(workers: &[&str], failing: Option<&str>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains("MARK-DISCOVER"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson(&format!(
                    "<file path=\"spawn_manifest.json\">\n{}\n</file>",
                    spawn_manifest(workers)
                )))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;
    for w in workers {
        let marker = format!("MARK-W-{w}:");
        if Some(*w) == failing {
            Mock::given(method("POST"))
                .and(path("/api/chat"))
                .and(body_string_contains(marker.as_str()))
                .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
                .mount(&server)
                .await;
        } else {
            answer(&server, &marker, &format!("out/{w}.md")).await;
        }
    }
    answer(&server, "MARK-SIDE", "out/side.md").await;
    answer(&server, "MARK-REDUCE", "out/reduce.md").await;
    server
}

#[tokio::test]
async fn map_reduce_workers_run_in_parallel_tiers_and_reduce_waits() {
    let tmp = workspace_with(&[]);
    let server = map_reduce_server(&["w1", "w2"], None).await;
    let plan = map_reduce_plan(8);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let prompts = prompts_seen(&server).await;
    let reduce = first_index(&prompts, "MARK-REDUCE").expect("reduce ran");
    for w in ["w1", "w2"] {
        let at = first_index(&prompts, &format!("MARK-W-{w}:"))
            .unwrap_or_else(|| panic!("worker {w} never ran"));
        assert!(at < reduce, "worker {w} must run before reduce");
    }
    assert_eq!(manifest_status(&result, "r"), AgentStatus::Completed);
}

#[tokio::test]
async fn map_reduce_over_cap_aborts() {
    let tmp = workspace_with(&[]);
    let server = map_reduce_server(&["w1", "w2", "w3"], None).await;
    let plan = map_reduce_plan(2);

    let err = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect_err("three workers over max_spawn 2 must abort");
    assert!(format!("{err:#}").contains("max_spawn"), "{err:#}");
    assert_eq!(count_containing(&prompts_seen(&server).await, "MARK-REDUCE"), 0);
}

#[tokio::test]
async fn map_reduce_failed_worker_skips_reduce() {
    let tmp = workspace_with(&[]);
    let server = map_reduce_server(&["w1", "w2"], Some("w2")).await;
    let plan = map_reduce_plan(8);

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 2),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await
    .expect("a failed worker is not a run error");

    assert!(matches!(manifest_status(&result, "w2"), AgentStatus::Failed(_)));
    let reduce = result
        .manifests
        .iter()
        .find(|m| m.work_unit_id == "r")
        .expect("reduce has a skip record");
    assert_eq!(reduce.summary.as_deref(), Some("dependency did not complete"));
    assert_eq!(count_containing(&prompts_seen(&server).await, "MARK-REDUCE"), 0);
}
