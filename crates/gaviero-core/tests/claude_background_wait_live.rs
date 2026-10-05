//! Live check against the installed `claude` CLI: a chat turn whose agent
//! starts a background shell command or subagent and waits for it must stay
//! open (the TUI keeps its prompt locked) until Claude reads the result and
//! answers, in both permission modes. Re-run after every Claude Code
//! upgrade: the stream shape these turns depend on is undocumented.
//!
//! `cargo test -p gaviero-core --test claude_background_wait_live -- --ignored`

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::Turn;
use gaviero_core::agent_session::registry::{SessionConstruction, create_session};
use gaviero_core::context_planner::types::ModelSpec;
use gaviero_core::context_planner::{PlannerMetadata, RuntimeConfig, build_provider_profile};
use gaviero_core::observer::{AcpObserver, PermissionDecision, WriteGateObserver};
use gaviero_core::types::WriteProposal;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Log {
    statuses: Vec<String>,
    started: Vec<String>,
    finished: Vec<(String, String)>,
    messages: Vec<(String, String)>,
    completed_at: Option<Instant>,
}

struct Rec(Arc<Mutex<Log>>);

impl AcpObserver for Rec {
    fn on_stream_chunk(&self, _: &str) {}
    fn on_tool_call_started(&self, _: &str) {}
    fn on_streaming_status(&self, status: &str) {
        self.0.lock().unwrap().statuses.push(status.to_string());
    }
    fn on_message_complete(&self, role: &str, content: &str) {
        let mut log = self.0.lock().unwrap();
        log.messages.push((role.to_string(), content.to_string()));
        log.completed_at = Some(Instant::now());
    }
    fn on_proposal_deferred(&self, _: &Path, _: Option<&str>, _: &str) {}
    fn on_background_task_started(&self, _task_id: &str, description: &str) {
        self.0.lock().unwrap().started.push(description.to_string());
    }
    fn on_background_task_finished(&self, _task_id: &str, status: &str, summary: &str) {
        self.0
            .lock()
            .unwrap()
            .finished
            .push((status.to_string(), summary.to_string()));
    }
    fn on_permission_request(
        &self,
        _: &str,
        _: &str,
        _: &serde_json::Value,
        respond: tokio::sync::oneshot::Sender<PermissionDecision>,
    ) {
        let _ = respond.send(PermissionDecision::allow());
    }
}

struct NoWrites;
impl WriteGateObserver for NoWrites {
    fn on_proposal_created(&self, _: &WriteProposal) {}
    fn on_proposal_updated(&self, _: u64) {}
    fn on_proposal_finalized(&self, _: &str) {}
}

/// Run one chat turn against the live CLI; returns the log, the turn's
/// wall time, and how long the CLI took to exit after the final message.
async fn run_turn(
    auto_approve: bool,
    tools: &[&str],
    user_message: &str,
) -> (Log, Duration, Duration) {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let log = Arc::new(Mutex::new(Log::default()));
    let model = "claude:haiku";
    let profile = build_provider_profile(&ModelSpec::parse(model), &RuntimeConfig::default());
    #[allow(deprecated)]
    let mut session = create_session(SessionConstruction {
        write_gate: Arc::new(TokioMutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoWrites),
        ))),
        observer: Box::new(Rec(log.clone())),
        model: model.into(),
        ollama_base_url: None,
        workspace_root: dir.path().to_path_buf(),
        additional_roots: vec![],
        agent_id: "live".into(),
        conv_id: None,
        options: AgentOptions {
            auto_approve,
            available_tools: Some(tools.iter().map(|t| t.to_string()).collect()),
            approved_tools: Some(tools.iter().map(|t| t.to_string()).collect()),
            suppress_hooks: true,
            ..AgentOptions::default()
        },
        profile,
        cancel_token: CancellationToken::new(),
        mcp_server: None,
    });
    let turn = Turn {
        user_message: user_message.into(),
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        skill_selections: vec![],
        replay_history: None,
        effort: None,
        auto_approve,
        metadata: PlannerMetadata::default(),
    };

    let started = Instant::now();
    let _ = tokio::time::timeout(Duration::from_secs(240), session.send_turn(turn))
        .await
        .expect("turn finished")
        .expect("send_turn");
    let returned = Instant::now();

    let log = std::mem::take(&mut *log.lock().unwrap());
    let finalize = returned - log.completed_at.expect("completed");
    (log, started.elapsed(), finalize)
}

fn transcript(log: &Log) -> String {
    format!(
        "messages: {:?}\nstarted: {:?}\nfinished: {:?}\nstatuses: {:?}",
        log.messages, log.started, log.finished, log.statuses
    )
}

async fn run(auto_approve: bool) {
    let (log, elapsed, finalize) = run_turn(
        auto_approve,
        &["Bash"],
        "Run this shell command with the Bash tool using run_in_background=true: \
         sleep 20 && echo PROBE_DONE . Then end your turn and wait for it to \
         finish (do not poll, do not sleep). When it finishes, reply with \
         exactly: FINAL <its output>.",
    )
    .await;
    let transcript = transcript(&log);
    // Closing stdin lets the CLI exit; without it the host waits out
    // PROCESS_WAIT_TIMEOUT (10 s) and kills it.
    assert!(
        finalize < Duration::from_secs(8),
        "CLI took {finalize:?} to exit after the turn\n{transcript}"
    );
    assert!(
        log.started.iter().any(|d| d.starts_with("$ sleep 20")),
        "{transcript}"
    );
    assert!(
        log.statuses
            .iter()
            .any(|s| s.starts_with("Waiting for background command: sleep 20")),
        "{transcript}"
    );
    assert_eq!(log.messages.len(), 1, "{transcript}");
    let (role, content) = &log.messages[0];
    assert_eq!(role, "assistant", "{transcript}");
    assert!(
        log.finished.iter().any(|(s, _)| s == "completed"),
        "{transcript}"
    );
    assert!(content.contains("FINAL PROBE_DONE"), "{transcript}");
    assert!(
        elapsed >= Duration::from_secs(20),
        "turn ended before the command could finish\n{transcript}"
    );
}

/// The model launches a subagent *without* `run_in_background`; Claude
/// Code 2.1.289 backgrounds it by default and the parent answers before it
/// finishes. The turn must stay open until the agent's result is read.
async fn run_agent(auto_approve: bool) {
    let (log, _, _) = run_turn(
        auto_approve,
        &["Agent", "Bash"],
        "Use the Agent tool exactly once (subagent_type general-purpose, \
         description probe-a) with this prompt: Run the Bash command `sleep 20` \
         then reply with the single word PONG. Do not pass run_in_background. \
         Do not wait for it yourself. When it has finished, reply with \
         exactly: FINAL <its reply>.",
    )
    .await;
    let transcript = transcript(&log);
    assert!(
        log.started.iter().any(|d| d.contains("probe-a")),
        "{transcript}"
    );
    assert!(
        log.finished.iter().any(|(s, _)| s == "completed"),
        "{transcript}"
    );
    // Ending at the parent's first `result` leaves only the launch turn's
    // text; `FINAL` comes from the wake-up turn that read the agent. The
    // agent's own reply is not asserted: inside an agent sandbox its Bash
    // can fail (MSYS exit 66) and it answers with the error instead of PONG.
    assert_eq!(log.messages.len(), 1, "{transcript}");
    let (role, content) = &log.messages[0];
    assert_eq!(role, "assistant", "{transcript}");
    assert!(content.contains("FINAL "), "{transcript}");
    assert!(
        log.statuses
            .iter()
            .any(|s| s.starts_with("Background agent: probe-a")),
        "{transcript}"
    );
}

#[tokio::test]
#[ignore = "spawns the claude CLI and calls the API"]
async fn interactive_turn_waits_for_a_background_command() {
    run(false).await;
}

#[tokio::test]
#[ignore = "spawns the claude CLI and calls the API"]
async fn auto_approve_turn_waits_for_a_background_command() {
    run(true).await;
}

#[tokio::test]
#[ignore = "spawns the claude CLI and calls the API"]
async fn interactive_turn_waits_for_a_default_background_agent() {
    run_agent(false).await;
}

#[tokio::test]
#[ignore = "spawns the claude CLI and calls the API"]
async fn auto_approve_turn_waits_for_a_default_background_agent() {
    run_agent(true).await;
}
