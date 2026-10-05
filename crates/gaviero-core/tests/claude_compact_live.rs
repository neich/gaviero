//! Live checks against the installed `claude` CLI for `/compact`: a chat
//! turn whose message is a slash command must reach Claude as the bare
//! command — wrapped in `<user_message>` with context blocks, Claude reads
//! it as prose (verified on Claude Code 2.1.289). Re-run after every Claude
//! Code upgrade.
//!
//! `cargo test -p gaviero-core --test claude_compact_live -- --ignored --nocapture`
//!
//! The multi-turn test resumes a session, so it needs a normal terminal:
//! inside the agent sandbox a `claude` spawned from a test binary never
//! persists its session (every `--resume` fails "No conversation found"),
//! while the same command from a shell does.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    session_id: Option<String>,
    compactions: Vec<(String, Option<u64>, Option<u64>)>,
    windows: Vec<u64>,
    messages: Vec<(String, String)>,
}

struct Rec(Arc<Mutex<Log>>);

impl AcpObserver for Rec {
    fn on_stream_chunk(&self, _: &str) {}
    fn on_tool_call_started(&self, _: &str) {}
    fn on_streaming_status(&self, _: &str) {}
    fn on_message_complete(&self, role: &str, content: &str) {
        let mut log = self.0.lock().unwrap();
        log.messages.push((role.to_string(), content.to_string()));
    }
    fn on_proposal_deferred(&self, _: &Path, _: Option<&str>, _: &str) {}
    fn on_claude_session_started(&self, session_id: &str) {
        self.0.lock().unwrap().session_id = Some(session_id.to_string());
    }
    fn on_context_window(&self, tokens: u64) {
        self.0.lock().unwrap().windows.push(tokens);
    }
    fn on_context_compacted(&self, trigger: &str, pre: Option<u64>, post: Option<u64>) {
        let mut log = self.0.lock().unwrap();
        log.compactions.push((trigger.to_string(), pre, post));
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

/// One chat turn on a fresh `ClaudeSession`, resuming `resume` when given —
/// how the TUI drives Claude (a new session object per turn).
async fn turn(dir: &Path, log: &Arc<Mutex<Log>>, resume: Option<String>, message: &str) {
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
        workspace_root: dir.to_path_buf(),
        additional_roots: vec![],
        agent_id: "live".into(),
        conv_id: None,
        options: AgentOptions {
            resume_session_id: resume,
            ..AgentOptions::default()
        },
        profile,
        cancel_token: CancellationToken::new(),
        mcp_server: None,
    });
    let turn = Turn {
        user_message: message.into(),
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        skill_selections: vec![],
        replay_history: None,
        effort: None,
        auto_approve: false,
        metadata: PlannerMetadata::default(),
    };
    let _ = tokio::time::timeout(Duration::from_secs(180), session.send_turn(turn))
        .await
        .expect("turn finished")
        .expect("send_turn");
}

/// Runs inside the agent sandbox: on a fresh session the CLI itself answers
/// a bare `/compact` with "Not enough messages to compact."; a wrapped one
/// gets prose from the model instead.
#[tokio::test]
#[ignore = "spawns the claude CLI and calls the API"]
async fn a_slash_command_reaches_claude_as_a_command() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let log = Arc::new(Mutex::new(Log::default()));
    turn(dir.path(), &log, None, "/compact Keep file paths verbatim.").await;
    {
        let log = log.lock().unwrap();
        let (role, content) = log.messages.last().expect("a reply");
        assert_eq!(role, "assistant", "{:?}", log.messages);
        assert_eq!(content.trim(), "Not enough messages to compact.");
        // A CLI-local command makes no model call, so no `modelUsage`.
        assert!(log.windows.is_empty(), "{:?}", log.windows);
    }

    // A model turn reports the model's real window.
    turn(dir.path(), &log, None, "Reply only OK.").await;
    let log = log.lock().unwrap();
    assert!(
        !log.windows.is_empty() && log.windows.iter().all(|&w| w >= 100_000),
        "Claude reports the model's window: {:?}",
        log.windows
    );
}

/// Needs a normal terminal (see the module doc).
#[tokio::test]
#[ignore = "spawns the claude CLI, calls the API, needs session persistence"]
async fn compact_shrinks_a_resumed_session_and_keeps_its_facts() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let log = Arc::new(Mutex::new(Log::default()));

    turn(
        dir.path(),
        &log,
        None,
        "Note for later: the build target we discussed is gaviero-tui and the \
         failing test is a_default_background_agent_holds_the_turn. Reply only OK.",
    )
    .await;
    let session = log.lock().unwrap().session_id.clone().expect("session id");
    turn(
        dir.path(),
        &log,
        Some(session.clone()),
        "/compact Keep the build target and test name verbatim.",
    )
    .await;
    turn(
        dir.path(),
        &log,
        Some(session),
        "Which build target and which failing test did we note? One line.",
    )
    .await;

    let log = log.lock().unwrap();
    let transcript = format!(
        "compactions: {:?}\nwindows: {:?}\nmessages: {:?}",
        log.compactions, log.windows, log.messages
    );
    assert_eq!(log.compactions.len(), 1, "{transcript}");
    let (trigger, pre, post) = &log.compactions[0];
    assert_eq!(trigger, "manual", "{transcript}");
    assert!(
        matches!((pre, post), (Some(pre), Some(post)) if post < pre),
        "{transcript}"
    );
    let (_, last) = log.messages.last().expect("answer");
    assert!(last.contains("gaviero-tui"), "{transcript}");
    assert!(
        last.contains("a_default_background_agent_holds_the_turn"),
        "{transcript}"
    );
}
