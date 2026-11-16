//! `budget { wall usd }` through the real `swarm::pipeline::execute` path,
//! against a mock Ollama server.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::models::{AgentStatus, SwarmResult, WorkUnit};
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::{
    CompiledPlan, ExecutionMode, LoopConfig, LoopUntilCondition, RunBudget, RuntimeForOp,
};
use gaviero_core::swarm::verify::CostEstimate;
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


/// Counts `on_cost_update` calls.
#[derive(Default)]
struct CostObserver {
    updates: Arc<Mutex<Vec<f64>>>,
}

impl SwarmObserver for CostObserver {
    fn on_phase_changed(&self, _phase: &str) {}
    fn on_agent_state_changed(&self, _work_unit_id: &str, _status: &AgentStatus, _detail: &str) {}
    fn on_tier_started(&self, _current: usize, _total: usize) {}
    fn on_merge_conflict(&self, _branch: &str, _files: &[String]) {}
    fn on_completed(&self, _result: &SwarmResult) {}
    fn on_cost_update(&self, estimate: &CostEstimate) {
        self.updates.lock().unwrap().push(estimate.estimated_usd);
    }
}

fn chain_plan() -> CompiledPlan {
    let mut plan = CompiledPlan::from_work_units(
        vec![
            unit("a", &["out/a.md"], &["out/a.md"], "MARK-A1", &[]),
            unit("b", &["out/b.md"], &["out/b.md"], "MARK-B1", &["a"]),
            unit("c", &["out/c.md"], &["out/c.md"], "MARK-C1", &["b"]),
        ],
        Some(1),
    );
    plan.execution_mode = ExecutionMode::Document;
    plan
}

async fn mount_delayed(server: &MockServer, marker: &str, file: &str, delay_ms: u64) {
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains(marker))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson(&format!("<file path=\"{file}\">
done
</file>")))
                .insert_header("content-type", "application/x-ndjson")
                .set_delay(std::time::Duration::from_millis(delay_ms)),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn wall_budget_stops_before_later_tiers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("out")).unwrap();
    let server = MockServer::start().await;
    for (m, f) in [("MARK-A1", "out/a.md"), ("MARK-B1", "out/b.md"), ("MARK-C1", "out/c.md")] {
        mount_delayed(&server, m, f, 1500).await;
    }
    let mut plan = chain_plan();
    plan.budget = Some(RunBudget {
        wall_secs: Some(1),
        usd: None,
    });

    let result = execute(
        &plan,
        &config_for(tmp.path(), &server.uri(), 1),
        None,
        None,
        &QuietObserver,
        make_obs,
    )
    .await;
    assert!(
        result.is_ok(),
        "a spent budget stops the run without an error: {:?}",
        result.err().map(|e| format!("{e:#}"))
    );
    let prompts = prompts_seen(&server).await;
    assert_eq!(count_containing(&prompts, "MARK-A1"), 1, "a ran");
    assert_eq!(count_containing(&prompts, "MARK-B1"), 0, "b must not start");
    assert_eq!(count_containing(&prompts, "MARK-C1"), 0, "c must not start");
}

#[tokio::test]
async fn cost_updates_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("out")).unwrap();
    let server = MockServer::start().await;
    for (m, f) in [("MARK-A1", "out/a.md"), ("MARK-B1", "out/b.md"), ("MARK-C1", "out/c.md")] {
        mount_delayed(&server, m, f, 0).await;
    }
    let observer = CostObserver::default();
    let updates = observer.updates.clone();

    let result = execute(
        &chain_plan(),
        &config_for(tmp.path(), &server.uri(), 1),
        None,
        None,
        &observer,
        make_obs,
    )
    .await
    .expect("run succeeds");

    let completed = result
        .manifests
        .iter()
        .filter(|m| m.status == AgentStatus::Completed)
        .count();
    assert_eq!(completed, 3);
    assert!(
        updates.lock().unwrap().len() >= completed,
        "one cost update per recorded manifest at least, got {:?}",
        updates.lock().unwrap()
    );
}
