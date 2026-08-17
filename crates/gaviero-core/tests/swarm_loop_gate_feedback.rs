//! H0 — a failing deterministic loop gate reaches the next iteration's
//! agent, through the real `swarm::pipeline::execute` path.
//!
//! The unit tests in `swarm::loop_gate` cover the gate machinery in
//! isolation: truncation, dedup keys, spawn-error classification. What
//! they cannot show is that the pieces are actually *wired* — that a
//! probe's output survives the trip from `evaluate_loop_condition`
//! through the loop head, into `apply_iter_vars_with_gate_feedback`, and
//! out to the body agent's prompt. A refactor could keep every unit test
//! green while dropping the feedback on the floor.
//!
//! So this drives a two-iteration loop against a mock Ollama server and
//! asserts on the prompt the agent actually received on iteration 2.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::models::WorkUnit;
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode, LoopConfig, LoopUntilCondition};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The probe every test in this file gates on. It fails, and says so on
/// stderr, so there is a specific string to hunt for downstream.
const FAILING_PROBE: &str = "echo GATE-DIAGNOSTIC-MARKER; exit 3";

#[allow(deprecated)]
fn loop_unit(id: &str, owned: &[&str]) -> WorkUnit {
    WorkUnit {
        id: id.to_string(),
        description: format!("test unit {id}"),
        scope: FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        },
        produces: vec![],
        depends_on: vec![],
        backend: Default::default(),
        model: Some("ollama:qwen".to_string()),
        effort: None,
        extra: vec![],
        tier: ModelTier::Cheap,
        privacy: PrivacyLevel::Public,
        coordinator_instructions: "Do the work for iteration {{ITER}}.".to_string(),
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

/// A command-gated loop over `agent_ids`. `LoopConfig` has no `Default`,
/// so the non-judge fields are spelled out at their documented defaults.
fn command_loop(agent_id: &str, probe: &str, max_iterations: u32) -> LoopConfig {
    LoopConfig {
        agent_ids: vec![agent_id.to_string()],
        until: LoopUntilCondition::Command(probe.to_string()),
        max_iterations,
        iter_start: 1,
        strict_judge: true,
        stability: 1,
        judge_timeout_secs: 120,
        branch_chain: Default::default(),
        consensus_mode: Default::default(),
        verdict_output_dir: None,
        irreconcilable_after: 2,
    }
}

/// Records the gate-failure events the pipeline emits.
#[derive(Default)]
struct RecordingObserver {
    gate_failures: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl SwarmObserver for RecordingObserver {
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

    fn on_loop_gate_failed(&self, probe: &str, status: &str, output: &str) {
        self.gate_failures.lock().unwrap().push((
            probe.to_string(),
            status.to_string(),
            output.to_string(),
        ));
    }
}

struct NoopAcpObserver;
impl AcpObserver for NoopAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

fn config_for(workspace: &Path, ollama_url: &str) -> SwarmConfig {
    SwarmConfig {
        execution_mode: ExecutionMode::Document,
        max_parallel: 1,
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
        extract_agent_findings: false,
        resume_from_artifacts: false,
        knowledge_invalidation: None,
        // Bound the whole run: a wiring bug that loops forever should
        // fail this test, not hang CI.
        run_timeout_secs: 120,
    }
}

/// A mock Ollama turn that writes `content` to `out/report.md`, so the
/// loop's delivery gate sees the agent actually produce something.
fn ndjson_turn(content: &str) -> String {
    let body = format!("<file path=\"out/report.md\">\n{content}\n</file>");
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

/// A mock Ollama turn that produces prose only — no `<file>` block, so
/// the agent delivers nothing.
fn ndjson_turn_text(content: &str) -> String {
    let chunk = serde_json::json!({
        "model": "qwen",
        "message": { "role": "assistant", "content": content },
        "done": false
    });
    let done = serde_json::json!({
        "model": "qwen", "done": true, "total_duration": 1_000_000,
        "eval_count": 2, "prompt_eval_count": 5
    });
    format!("{chunk}\n{done}")
}

/// Every prompt the mock server was asked to complete, in order.
async fn prompts_seen(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|req| String::from_utf8_lossy(&req.body).into_owned())
        .collect()
}

#[tokio::test]
async fn a_failing_gate_reaches_the_next_iterations_agent_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson_turn("draft"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;

    let mut plan = CompiledPlan::from_work_units(vec![loop_unit("writer", &["out/**"])], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop("writer", FAILING_PROBE, 3)];

    let observer = RecordingObserver::default();
    let seen = observer.gate_failures.clone();
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri()),
        None,
        None,
        &observer,
        make_obs,
    )
    .await;
    assert!(
        result.is_ok(),
        "a failing gate is an ordinary non-convergence, not a run error: {:?}",
        result.err().map(|e| format!("{e:#}"))
    );

    // 1. The gate failed, and the observer was told why.
    let failures = seen.lock().unwrap().clone();
    assert!(
        !failures.is_empty(),
        "a failing `until command` must emit on_loop_gate_failed"
    );
    let (probe, _status, output) = &failures[0];
    assert_eq!(probe, FAILING_PROBE);
    assert!(
        output.contains("GATE-DIAGNOSTIC-MARKER"),
        "the probe's own output must be captured, got {output:?}"
    );

    // 2. The diagnostic reached a later agent prompt. This is the wiring
    //    the unit tests cannot see.
    let prompts = prompts_seen(&server).await;
    assert!(
        prompts.len() >= 2,
        "the loop must have dispatched more than one pass, saw {}",
        prompts.len()
    );
    assert!(
        prompts[1..]
            .iter()
            .any(|p| p.contains("GATE-DIAGNOSTIC-MARKER")),
        "no later prompt carried the gate diagnostic"
    );
    assert!(
        prompts[1..]
            .iter()
            .any(|p| p.contains("Previous gate failure")),
        "the fallback feedback section is missing from every later prompt"
    );
}

#[tokio::test]
async fn an_unchanged_workspace_reports_the_gate_was_not_rerun() {
    // The agent satisfies its `produces` contract every pass — the
    // artefact is already on disk — while changing nothing. So it counts
    // as delivering (the no-progress bound stays clear) and the
    // workspace is identical, which is exactly when re-running the probe
    // cannot tell us anything new.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    std::fs::write(workspace.join("out/report.md"), "identical every pass\n").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson_turn("identical every pass"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;

    let mut unit = loop_unit("writer", &["out/**"]);
    unit.produces = vec!["out/report.md".to_string()];
    let mut plan = CompiledPlan::from_work_units(vec![unit], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop("writer", FAILING_PROBE, 4)];

    let observer = RecordingObserver::default();
    let seen = observer.gate_failures.clone();
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    execute(
        &plan,
        &config_for(workspace, &server.uri()),
        None,
        None,
        &observer,
        make_obs,
    )
    .await
    .expect("execute");

    let failures = seen.lock().unwrap().clone();
    assert!(failures.len() >= 2, "expected several gate evaluations");

    // The first evaluation ran the probe; a later one, with the
    // workspace unchanged, must report the skip instead.
    assert!(
        failures
            .iter()
            .any(|(_, status, _)| status.contains("not rerun")),
        "an unchanged workspace should have skipped a probe, statuses: {:?}",
        failures.iter().map(|(_, s, _)| s).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_panel_that_cannot_run_stops_instead_of_burning_iterations() {
    // The provider is down, so every agent hard-fails. The `until
    // command` condition exempts this pass from the delivery gate, so
    // without the total-failure guard the loop would re-dispatch a panel
    // that cannot run for all `max_iterations` and still return Ok.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("provider exploded"))
        .mount(&server)
        .await;

    let mut plan = CompiledPlan::from_work_units(vec![loop_unit("writer", &["out/**"])], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![command_loop("writer", FAILING_PROBE, 6)];

    let observer = RecordingObserver::default();
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let err = execute(
        &plan,
        &config_for(workspace, &server.uri()),
        None,
        None,
        &observer,
        make_obs,
    )
    .await
    .expect_err("a unanimously failed panel must stop the run");

    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed for all"),
        "the error must name the total failure, got: {msg}"
    );

    // The point of stopping: it did not spend the whole iteration budget
    // re-dispatching a panel that cannot run.
    let dispatches = prompts_seen(&server).await.len();
    assert!(
        dispatches < 6,
        "expected the loop to stop early, saw {dispatches} dispatches"
    );
}

/// Prompt marker only the judge's instructions carry, so the mock
/// server's request log tells us whether the judge was ever dispatched.
const JUDGE_MARKER: &str = "JUDGE-PROMPT-MARKER";

fn judge_unit(id: &str) -> WorkUnit {
    let mut u = loop_unit(id, &[]);
    u.coordinator_instructions = format!("{JUDGE_MARKER} render a verdict");
    u
}

/// `until <a> and <b>`, with the judge written *first* so the test also
/// pins that evaluation order ignores author order.
fn composed_loop(agent_id: &str, judge: &str, probe: &str, max_iterations: u32) -> LoopConfig {
    LoopConfig {
        until: LoopUntilCondition::All(vec![
            LoopUntilCondition::Agent(judge.to_string()),
            LoopUntilCondition::Command(probe.to_string()),
        ]),
        ..command_loop(agent_id, probe, max_iterations)
    }
}

#[tokio::test]
async fn a_failing_command_short_circuits_before_the_judge() {
    // D-7. The agent delivers nothing, so the delivery gate *would*
    // abort — but the command fails first and the judge is never
    // dispatched, so there is no panel to protect. Widening the old
    // gate to "condition contains an Agent" would have killed this run.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                // No <file> block: the agent writes nothing at all.
                .set_body_string(ndjson_turn_text("thinking about it"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;

    let mut plan = CompiledPlan::from_work_units(vec![loop_unit("writer", &["out/**"])], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_judge_units = vec![judge_unit("reviewer")];
    plan.loop_configs = vec![composed_loop("writer", "reviewer", FAILING_PROBE, 4)];

    let observer = RecordingObserver::default();
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri()),
        None,
        None,
        &observer,
        make_obs,
    )
    .await;

    assert!(
        result.is_ok(),
        "a silent agent must not abort the run when the judge is never reached: {:?}",
        result.err().map(|e| format!("{e:#}"))
    );

    let prompts = prompts_seen(&server).await;
    assert!(
        !prompts.iter().any(|p| p.contains(JUDGE_MARKER)),
        "the judge must not be dispatched once a cheaper condition has failed"
    );
}

#[tokio::test]
async fn a_passing_command_lets_the_judge_run() {
    // The other half of the contract: once the deterministic condition
    // passes, the judge is consulted — and the delivery gate applies to
    // it, so a silent panel is caught exactly where it matters.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson_turn("draft"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;

    let mut plan = CompiledPlan::from_work_units(vec![loop_unit("writer", &["out/**"])], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_judge_units = vec![judge_unit("reviewer")];
    plan.loop_configs = vec![composed_loop("writer", "reviewer", "exit 0", 3)];

    let observer = RecordingObserver::default();
    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(NoopAcpObserver) };

    let result = execute(
        &plan,
        &config_for(workspace, &server.uri()),
        None,
        None,
        &observer,
        make_obs,
    )
    .await;

    // Either the judge ran, or the delivery gate stopped it from being
    // judged — both prove the composition reached the Agent condition.
    match result {
        Ok(_) => {
            let prompts = prompts_seen(&server).await;
            assert!(
                prompts.iter().any(|p| p.contains(JUDGE_MARKER)),
                "a passing command must let the judge run"
            );
        }
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("did not deliver"),
                "the only acceptable failure here is the delivery gate, got: {msg}"
            );
        }
    }
}
