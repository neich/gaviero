//! Live check against the installed `codex` CLI (multi-agent on): a chat
//! turn whose agent spawns a sub-agent and waits for it must end on the
//! parent's own `turn/completed`, not the sub-agent's, and the sub-agent
//! must be reported as a background task that completed. Re-run after every
//! Codex upgrade: the app-server's multi-agent events are experimental.
//!
//! `cargo test -p gaviero-core --test codex_subagent_live -- --ignored --nocapture`
//! (`E2E_AGENT_MODEL` overrides the model, default `codex:gpt-5.6-sol`).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::registry::{SessionConstruction, create_session};
use gaviero_core::agent_session::{TransportContext, build_turn};
use gaviero_core::context_planner::{
    ModelSpec, PlannerMetadata, PlannerSelections, RuntimeConfig, build_provider_profile,
};
use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::types::WriteProposal;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};
use tokio::sync::Mutex as TokioMutex;

#[derive(Default)]
struct Log {
    started: Vec<String>,
    finished: Vec<(String, String)>,
    messages: Vec<(String, String)>,
    tools: Vec<String>,
}

struct Rec(Arc<Mutex<Log>>);

impl AcpObserver for Rec {
    fn on_stream_chunk(&self, _: &str) {}
    fn on_tool_call_started(&self, tool: &str) {
        self.0.lock().unwrap().tools.push(tool.to_string());
    }
    fn on_streaming_status(&self, _: &str) {}
    fn on_message_complete(&self, role: &str, content: &str) {
        let mut log = self.0.lock().unwrap();
        log.messages.push((role.to_string(), content.to_string()));
    }
    fn on_proposal_deferred(&self, _: &Path, _: Option<&str>, _: &str) {}
    fn on_background_task_started(&self, _task_id: &str, description: &str) {
        self.0.lock().unwrap().started.push(description.to_string());
    }
    fn on_background_task_finished(&self, _task_id: &str, status: &str, summary: &str) {
        let mut log = self.0.lock().unwrap();
        log.finished.push((status.to_string(), summary.to_string()));
    }
}

struct NoWrites;
impl WriteGateObserver for NoWrites {
    fn on_proposal_created(&self, _: &WriteProposal) {}
    fn on_proposal_updated(&self, _: u64) {}
    fn on_proposal_finalized(&self, _: &str) {}
}

#[tokio::test]
#[ignore = "spawns the codex CLI and calls the API"]
async fn a_codex_turn_waits_for_its_sub_agent() {
    let model = std::env::var("E2E_AGENT_MODEL").unwrap_or_else(|_| "codex:gpt-5.6-sol".into());
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let log = Arc::new(Mutex::new(Log::default()));
    let profile = build_provider_profile(&ModelSpec::parse(&model), &RuntimeConfig::default());
    let mut session = create_session(SessionConstruction {
        write_gate: Arc::new(TokioMutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoWrites),
        ))),
        observer: Box::new(Rec(log.clone())),
        model: model.clone(),
        ollama_base_url: None,
        workspace_root: dir.path().to_path_buf(),
        additional_roots: vec![],
        agent_id: "live".into(),
        conv_id: None,
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        mcp_server: None,
    });
    let turn = build_turn(
        PlannerSelections {
            memory_selections: vec![],
            graph_selections: vec![],
            skill_selections: vec![],
            file_refs: vec![],
            replay_history: None,
            metadata: PlannerMetadata {
                memory_count: 0,
                graph_token_estimate: 0,
                graph_budget: 0,
                is_first_turn: true,
                continuity_mode: None,
            },
        },
        TransportContext {
            user_message: "Spawn exactly one sub-agent (use your spawn agent tool) whose \
                           task is: reply with the word PONG. Then wait for it and reply \
                           with exactly: FINAL <its answer>. Do not run shell commands."
                .into(),
            effort: None,
            auto_approve: true,
        },
    );
    let _ = tokio::time::timeout(Duration::from_secs(240), session.send_turn(turn))
        .await
        .expect("turn finished")
        .expect("send_turn");
    session.close().await;

    let log = log.lock().unwrap();
    let transcript = format!(
        "messages: {:?}\nstarted: {:?}\nfinished: {:?}\ntools: {:?}",
        log.messages, log.started, log.finished, log.tools
    );
    assert!(
        log.started.iter().any(|d| d.starts_with("sub-agent")),
        "{transcript}"
    );
    assert!(
        log.finished.iter().any(|(s, _)| s == "completed"),
        "{transcript}"
    );
    let assistant: Vec<&String> = log
        .messages
        .iter()
        .filter(|(role, _)| role == "assistant")
        .map(|(_, content)| content)
        .collect();
    assert_eq!(assistant.len(), 1, "{transcript}");
    // Ending on the sub-agent's `turn/completed` left only the parent's
    // commentary plus the sub-agent's own text; FINAL comes after the wait.
    assert!(assistant[0].contains("FINAL"), "{transcript}");
}
