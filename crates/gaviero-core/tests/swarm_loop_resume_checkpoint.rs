//! A loop resumed from its artefacts runs the resumed pass, even when the
//! run is also resumed from a node checkpoint (`gaviero-cli --resume`).
//!
//! The checkpoint marks a loop body agent Completed for the last pass it
//! ran. The tier dispatch runs a loop's first pass and skips Completed
//! agents, so before the fix a `--resume` skipped the pass the artefacts said
//! to resume at and the loop moved past it: a run restarted after pass 4
//! re-ran from pass 2 (no artefact resume) or would skip pass 5.

use std::collections::HashMap;
use std::path::Path;

use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::repo_map::SpecificityConfig;
use gaviero_core::swarm::execution_state::{ExecutionState, NodeStatus};
use gaviero_core::swarm::models::{AgentStatus, WorkUnit};
use gaviero_core::swarm::pipeline::{SwarmConfig, execute};
use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode, LoopConfig, LoopUntilCondition};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[allow(deprecated)]
fn writer() -> WorkUnit {
    WorkUnit {
        id: "writer".to_string(),
        description: "write a pass".to_string(),
        scope: FileScope {
            owned_paths: vec!["out/writer-v*.md".to_string()],
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        },
        produces_json: vec![],
        produces: vec![],
        depends_on: vec![],
        backend: Default::default(),
        model: Some("ollama:qwen".to_string()),
        effort: None,
        extra: vec![],
        tier: ModelTier::Cheap,
        privacy: PrivacyLevel::Public,
        coordinator_instructions: "Write pass {{ITER}}.".to_string(),
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

/// One pass only: the tier dispatch runs it and the loop block adds none.
fn one_pass_loop() -> LoopConfig {
    LoopConfig {
        agent_ids: vec!["writer".to_string()],
        until: LoopUntilCondition::Command("exit 1".to_string()),
        max_iterations: 1,
        iter_start: 1,
        strict_judge: true,
        stability: 1,
        judge_timeout_secs: 120,
        branch_chain: Default::default(),
        consensus_mode: Default::default(),
        verdict_output_dir: Some("out".to_string()),
        irreconcilable_after: 2,
        baseline_units: Vec::new(),
    }
}

struct Quiet;
impl SwarmObserver for Quiet {
    fn on_phase_changed(&self, _phase: &str) {}
    fn on_agent_state_changed(&self, _id: &str, _status: &AgentStatus, _detail: &str) {}
    fn on_tier_started(&self, _current: usize, _total: usize) {}
    fn on_merge_conflict(&self, _branch: &str, _files: &[String]) {}
    fn on_completed(&self, _result: &gaviero_core::swarm::models::SwarmResult) {}
}
impl AcpObserver for Quiet {
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
        tool_policy: None,
        exposed_tools: None,
        extract_agent_findings: false,
        resume_from_artifacts: true,
        knowledge_invalidation: None,
        run_timeout_secs: 120,
        chat_injection: Default::default(),
        skills_emit: Default::default(),
        skill_catalog: None,
        memory_root: None,
        graph_db_path: None,
    }
}

fn ndjson_turn(file: &str) -> String {
    let body = format!("<file path=\"{file}\">\npass body\n</file>");
    let chunk = serde_json::json!({
        "model": "qwen",
        "message": { "role": "assistant", "content": body },
        "done": false
    });
    let done = serde_json::json!({
        "model": "qwen", "done": true, "total_duration": 1_000_000,
        "eval_count": 2, "prompt_eval_count": 5
    });
    format!("{chunk}\n{done}")
}

#[tokio::test]
async fn a_checkpoint_resume_still_runs_the_pass_the_artefacts_resume_at() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    std::fs::create_dir_all(workspace.join("out")).unwrap();
    // Passes 1 and 2 are on disk.
    std::fs::write(workspace.join("out/writer-v1.md"), "pass 1\n").unwrap();
    std::fs::write(workspace.join("out/writer-v2.md"), "pass 2\n").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(ndjson_turn("out/writer-v3.md"))
                .insert_header("content-type", "application/x-ndjson"),
        )
        .mount(&server)
        .await;

    let mut plan = CompiledPlan::from_work_units(vec![writer()], Some(1));
    plan.execution_mode = ExecutionMode::Document;
    plan.loop_configs = vec![one_pass_loop()];

    // The node checkpoint of the earlier run: the loop agent completed
    // (its last pass), as `gaviero-cli --resume` loads it.
    let mut checkpoint = ExecutionState::new_from_plan(&plan);
    checkpoint.set_status("writer", NodeStatus::Completed);

    let make_obs = |_id: &str| -> Box<dyn AcpObserver> { Box::new(Quiet) };
    let result = execute(
        &plan,
        &config_for(workspace, &server.uri()),
        Some(checkpoint),
        None,
        &Quiet,
        make_obs,
    )
    .await;
    assert!(result.is_ok(), "{:?}", result.err().map(|e| format!("{e:#}")));

    let prompts: Vec<String> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert_eq!(prompts.len(), 1, "the resumed pass ran exactly once");
    assert!(
        prompts[0].contains("Write pass 3."),
        "it ran as pass 3, after the two on disk"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("out/writer-v1.md")).unwrap(),
        "pass 1\n",
        "earlier passes are untouched"
    );
}
