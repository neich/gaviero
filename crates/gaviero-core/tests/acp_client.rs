//! ACP client (`dsh:`) against the in-tree `fake-acp-agent` binary.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::agent_client_protocol::AcpClientSession;
use gaviero_core::agent_session::registry::SessionConstruction;
use gaviero_core::agent_session::{AgentSession, Turn};
use gaviero_core::context_planner::{PlannerMetadata, ReplayPayload, RuntimeConfig, build_provider_profile};
use gaviero_core::context_planner::ledger::Role;
use gaviero_core::context_planner::types::ModelSpec;
use gaviero_core::observer::{AcpObserver, PermissionDecision, WriteGateObserver};
use gaviero_core::swarm::backend::{
    CompletionRequest, StopReason, UnifiedStreamEvent, WriteGateHandle, create_backend,
    BackendConfig,
};
use gaviero_core::types::{FileScope, WriteProposal};
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

struct NoopAcp;
impl AcpObserver for NoopAcp {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

struct RecWrite {
    paths: Arc<Mutex<Vec<PathBuf>>>,
}
impl WriteGateObserver for RecWrite {
    fn on_proposal_created(&self, proposal: &WriteProposal) {
        self.paths.lock().unwrap().push(proposal.file_path.clone());
    }
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, path: &str) {
        self.paths.lock().unwrap().push(PathBuf::from(path));
    }
}

struct DenyAcp;
impl AcpObserver for DenyAcp {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
    fn on_permission_request(
        &self,
        _tool_name: &str,
        _description: &str,
        _input: &serde_json::Value,
        respond: tokio::sync::oneshot::Sender<PermissionDecision>,
    ) {
        let _ = respond.send(PermissionDecision::deny());
    }
}

fn fake_bin() -> String {
    option_env!("CARGO_BIN_EXE_fake_acp_agent")
        .map(str::to_string)
        .or_else(|| std::env::var("CARGO_BIN_EXE_fake_acp_agent").ok())
        .unwrap_or_else(|| {
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop();
            p.pop();
            p.push("target");
            p.push("debug");
            if cfg!(windows) {
                p.push("fake_acp_agent.exe");
            } else {
                p.push("fake_acp_agent");
            }
            p.to_string_lossy().into_owned()
        })
}

fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"t\"\nversion=\"0.0.0\"\n").unwrap();
    let _ = git2::Repository::init(dir.path());
    dir
}

fn construction(
    root: &Path,
    observer: Box<dyn AcpObserver>,
    write_obs: Box<dyn WriteGateObserver>,
    auto_approve: bool,
) -> SessionConstruction {
    construction_with_gate(
        root,
        observer,
        Arc::new(TokioMutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            write_obs,
        ))),
        auto_approve,
    )
}

/// Same as [`construction`] but with the caller's own gate, so a test can pick
/// the `WriteMode` and inspect what the gate accumulated.
fn construction_with_gate(
    root: &Path,
    observer: Box<dyn AcpObserver>,
    write_gate: Arc<TokioMutex<WriteGatePipeline>>,
    auto_approve: bool,
) -> SessionConstruction {
    let spec = ModelSpec::parse("dsh:deepseek-v4-flash");
    let profile = build_provider_profile(&spec, &RuntimeConfig::default());
    #[allow(deprecated)]
    SessionConstruction {
        write_gate,
        observer,
        model: "dsh:deepseek-v4-flash".into(),
        ollama_base_url: None,
        workspace_root: root.to_path_buf(),
        additional_roots: vec![],
        agent_id: "dsh".into(),
        conv_id: None,
        options: AgentOptions {
            auto_approve,
            effort: "high".into(),
            ..AgentOptions::default()
        },
        profile,
        cancel_token: CancellationToken::new(),
        mcp_server: None,
    }
}

/// Commit the whole workspace, so HEAD exists and a clean path's turn-start
/// content is recoverable from the index.
fn commit_all(dir: &Path) {
    let repo = git2::Repository::open(dir).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
}

fn deferred_gate() -> Arc<TokioMutex<WriteGatePipeline>> {
    Arc::new(TokioMutex::new(WriteGatePipeline::new(
        WriteMode::Deferred,
        Box::new(RecWrite {
            paths: Arc::new(Mutex::new(Vec::new())),
        }),
    )))
}


fn extra(scenario: &str) -> Vec<(String, String)> {
    vec![
        ("dsh_command".into(), fake_bin()),
        ("dsh_args".into(), scenario.into()),
    ]
}

#[tokio::test]
async fn consecutive_turns_reuse_child_and_forward_session_configuration() {
    let dir = workspace();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let mut args = construction(dir.path(), Box::new(NoopAcp), Box::new(RecWrite { paths }), true);
    args.additional_roots = vec![dir.path().join("additional")];
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default()).with_extra(extra("inspect")).with_system_prompt(Some("Return exactly the requested JSON.".into()));
    for expected in [1, 2] {
        let events = drain(&mut session, true).await;
        let report = events.iter().find_map(|event| match event {
            UnifiedStreamEvent::TextDelta(text) => serde_json::from_str::<serde_json::Value>(text).ok(),
            _ => None,
        }).expect("fake inspection report");
        assert_eq!(report["turn"], expected);
        assert_eq!(report["model"]["modelId"], "deepseek-v4-flash");
        assert_eq!(report["new"]["additionalDirectories"][0], dir.path().join("additional").to_string_lossy().as_ref());
        assert_eq!(report["prompt"][0]["text"], "Return exactly the requested JSON.");
    }
    Box::new(session).close().await;
}

async fn drain_turn(
    session: &mut AcpClientSession,
    turn: Turn,
) -> Vec<UnifiedStreamEvent> {
    let mut stream = session.send_turn(turn).await.expect("send_turn");
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(ev))) => {
                let done = matches!(ev, UnifiedStreamEvent::Done(_));
                out.push(ev);
                if done {
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("stream error: {e:#}"),
            Ok(None) => break,
            Err(_) => panic!("ACP fake-agent turn timed out; got {out:?}"),
        }
    }
    out
}

fn inspect_prompt(events: &[UnifiedStreamEvent]) -> serde_json::Value {
    events
        .iter()
        .find_map(|event| match event {
            UnifiedStreamEvent::TextDelta(text) => serde_json::from_str(text).ok(),
            _ => None,
        })
        .expect("fake inspection report")
}

#[tokio::test]
async fn fresh_session_prompt_carries_host_replay_reused_session_does_not() {
    let dir = workspace();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite { paths }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default())
        .with_extra(extra("inspect"));
    let mut first = turn("new question", true);
    first.replay_history = Some(ReplayPayload {
        entries: vec![
            (Role::User, "old q".into()),
            (Role::Assistant, "old a".into()),
        ],
    });
    let report = inspect_prompt(&drain_turn(&mut session, first).await);
    let texts: Vec<&str> = report["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("old q")),
        "fresh session/new must restuff host replay: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("new question")),
        "{texts:?}"
    );

    let mut second = turn("follow up", true);
    second.replay_history = Some(ReplayPayload {
        entries: vec![
            (Role::User, "old q".into()),
            (Role::Assistant, "old a".into()),
        ],
    });
    let report = inspect_prompt(&drain_turn(&mut session, second).await);
    let texts: Vec<&str> = report["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect();
    assert!(
        texts.iter().all(|t| !t.contains("old q")),
        "reused ACP session must not restuff host replay: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("follow up")),
        "{texts:?}"
    );
    Box::new(session).close().await;
}

fn turn(msg: &str, auto_approve: bool) -> Turn {
    Turn {
        user_message: msg.into(),
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        skill_selections: vec![],
        replay_history: None,
        effort: Some("high".into()),
        auto_approve,
        metadata: PlannerMetadata::default(),
    }
}

async fn drain(
    session: &mut AcpClientSession,
    auto_approve: bool,
) -> Vec<UnifiedStreamEvent> {
    let mut stream = session
        .send_turn(turn("hello", auto_approve))
        .await
        .expect("send_turn");
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(ev))) => {
                let done = matches!(ev, UnifiedStreamEvent::Done(_));
                out.push(ev);
                if done {
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("stream error: {e:#}"),
            Ok(None) => break,
            Err(_) => panic!("ACP fake-agent turn timed out; got {out:?}"),
        }
    }
    out
}

#[tokio::test]
async fn happy_turn_maps_chunks_tools_and_usage() {
    let dir = workspace();
    let rec = Arc::new(Mutex::new(Vec::new()));
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: rec.clone(),
        }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(
        args,
        FileScope {
            owned_paths: vec!["src/".into()],
            ..FileScope::default()
        },
    )
    .with_extra(extra("happy"));
    let events = drain(&mut session, true).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::ThinkingDelta(t) if t.contains("thinking"))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::TextDelta(t) if t.contains("hello from fake acp"))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::ToolCallStart { .. })),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::Usage(u) if u.input_tokens == 11)),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(UnifiedStreamEvent::Done(StopReason::EndTurn))),
        "{events:?}"
    );
}

#[tokio::test]
async fn write_goes_through_write_gate() {
    let dir = workspace();
    let rec = Arc::new(Mutex::new(Vec::new()));
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: rec.clone(),
        }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(
        args,
        FileScope {
            owned_paths: vec!["src/".into()],
            ..FileScope::default()
        },
    )
    .with_extra(extra("write"));
    let _ = drain(&mut session, true).await;
    let paths = rec.lock().unwrap().clone();
    assert!(
        paths.iter().any(|p| p.ends_with("from_acp.rs")),
        "expected WriteProposal, got {paths:?}"
    );
    assert!(dir.path().join("src").join("from_acp.rs").is_file());
}

#[tokio::test]
async fn write_outside_scope_is_refused() {
    let dir = workspace();
    let rec = Arc::new(Mutex::new(Vec::new()));
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: rec.clone(),
        }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(
        args,
        FileScope {
            owned_paths: vec!["src/".into()],
            ..FileScope::default()
        },
    )
    .with_extra(extra("write_outside"));
    let _ = drain(&mut session, true).await;
    let paths = rec.lock().unwrap().clone();
    assert!(
        paths.is_empty(),
        "out-of-scope write must not become a proposal: {paths:?}"
    );
    assert!(!dir.path().join("secret").join("out.rs").exists());
}

#[tokio::test]
async fn permission_deny_still_completes_the_turn() {
    let dir = workspace();
    let args = construction(
        dir.path(),
        Box::new(DenyAcp),
        Box::new(RecWrite {
            paths: Arc::new(Mutex::new(Vec::new())),
        }),
        false,
    );
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default())
        .with_extra(extra("permission"));
    let events = drain(&mut session, false).await;
    assert!(
        matches!(events.last(), Some(UnifiedStreamEvent::Done(_))),
        "{events:?}"
    );
}

#[tokio::test]
async fn die_mid_turn_emits_error_and_done() {
    let dir = workspace();
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: Arc::new(Mutex::new(Vec::new())),
        }),
        true,
    );
    let mut session =
        AcpClientSession::new_with_scope(args, FileScope::default()).with_extra(extra("die_mid"));
    let events = drain(&mut session, true).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::Error(_))),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(UnifiedStreamEvent::Done(StopReason::Error))),
        "{events:?}"
    );
}

#[tokio::test]
async fn cancel_token_ends_the_turn() {
    let dir = workspace();
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: Arc::new(Mutex::new(Vec::new())),
        }),
        true,
    );
    args.cancel_token.cancel();
    let mut session =
        AcpClientSession::new_with_scope(args, FileScope::default()).with_extra(extra("happy"));
    let events = drain(&mut session, true).await;
    assert!(
        matches!(events.last(), Some(UnifiedStreamEvent::Done(StopReason::Timeout))),
        "{events:?}"
    );
}

#[tokio::test]
async fn direct_write_outside_fs_channel_emits_paths_modified() {
    let dir = workspace();
    let rec = Arc::new(Mutex::new(Vec::new()));
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: rec.clone(),
        }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default())
        .with_extra(extra("direct_write"));
    let events = drain(&mut session, true).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::PathsModified(p) if !p.is_empty())),
        "{events:?}"
    );
    // AutoAccept: the reconcile pass restores the path and the gate writes it
    // straight back, so a headless turn still leaves the agent's file on disk —
    // but it is now a finalized proposal instead of an invisible write.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("leaked.txt")).unwrap(),
        "bypassed fs channel\n"
    );
    assert!(
        rec.lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("leaked.txt")),
        "expected a proposal for the direct write, got {:?}",
        rec.lock().unwrap()
    );
}

/// The complaint this pass exists for: `dsh` edits land on disk with nothing to
/// review. With a Deferred gate (what the TUI chat sets per conversation) each
/// direct write must come back as a proposal carrying a real diff, and the tree
/// must be left at its turn-start state until a human accepts.
#[tokio::test]
async fn direct_writes_become_deferred_proposals() {
    let dir = workspace();
    std::fs::write(dir.path().join("src").join("obsolete.rs"), "fn gone() {}\n").unwrap();
    commit_all(dir.path());
    let committed = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();

    let gate = deferred_gate();
    let args = construction_with_gate(dir.path(), Box::new(NoopAcp), gate.clone(), true);
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default())
        .with_extra(extra("direct_write_multi"));
    let events = drain(&mut session, true).await;

    // The agent's bytes are off the tree: Deferred means "hold for review".
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap(),
        committed
    );
    assert!(!dir.path().join("new_file.rs").exists());
    assert!(dir.path().join("src").join("obsolete.rs").is_file());

    let pending = gate.lock().await.pending_proposals().to_vec();
    let mut names: Vec<String> = pending
        .iter()
        .map(|p| p.file_path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["Cargo.toml", "new_file.rs", "obsolete.rs"]);

    let cargo = pending
        .iter()
        .find(|p| p.file_path.ends_with("Cargo.toml"))
        .expect("proposal for the modified file");
    assert_eq!(cargo.original_content, committed);
    assert!(cargo.proposed_content.contains("9.9.9"), "{cargo:?}");
    assert!(!cargo.structural_hunks.is_empty());

    let fresh = pending
        .iter()
        .find(|p| p.file_path.ends_with("new_file.rs"))
        .expect("proposal for the created file");
    assert_eq!(fresh.original_content, "");
    assert_eq!(fresh.proposed_content, "fn fresh() {}\n");

    let gone = pending
        .iter()
        .find(|p| p.file_path.ends_with("obsolete.rs"))
        .expect("proposal for the deleted file");
    assert!(gone.is_deletion);
    assert_eq!(gone.original_content, "fn gone() {}\n");

    // Nothing is reported as modified: the gate holds every path.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::PathsModified(p) if !p.is_empty())),
        "{events:?}"
    );
}

/// A path that was already dirty at turn start has no reconstructible baseline
/// in git — the pre-turn capture is the only record of those bytes, and it must
/// be what the reviewer sees as the "before" side of the diff.
#[tokio::test]
async fn direct_write_is_restored_to_its_turn_start_dirty_content() {
    let dir = workspace();
    commit_all(dir.path());
    let dirty = "[package]\nname=\"t\"\nversion=\"0.0.0\"\n# uncommitted user line\n";
    std::fs::write(dir.path().join("Cargo.toml"), dirty).unwrap();

    let gate = deferred_gate();
    let args = construction_with_gate(dir.path(), Box::new(NoopAcp), gate.clone(), true);
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default())
        .with_extra(extra("direct_write_multi"));
    let _ = drain(&mut session, true).await;

    // Restored to the uncommitted turn-start content, not to HEAD.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap(),
        dirty
    );
    let pending = gate.lock().await.pending_proposals().to_vec();
    let cargo = pending
        .iter()
        .find(|p| p.file_path.ends_with("Cargo.toml"))
        .expect("proposal for the already-dirty file");
    assert_eq!(cargo.original_content, dirty);
    assert!(cargo.proposed_content.contains("9.9.9"), "{cargo:?}");
}

#[tokio::test]
async fn dsh_backend_happy_turn_with_fake_agent() {
    let dir = workspace();
    let backend = create_backend(&BackendConfig::Dsh {
        model: "deepseek-v4-flash".into(),
    })
    .unwrap();
    let req = CompletionRequest {
        prompt: "hi".into(),
        system_prompt: None,
        workspace_root: dir.path().to_path_buf(),
        additional_roots: vec![],
        allowed_tools: vec![],
        file_attachments: vec![],
        conversation_history: vec![],
        file_refs: vec![],
        effort: Some("high".into()),
        extra: extra("happy"),
        max_tokens: None,
        auto_approve: true,
        suppress_hooks: true,
        file_scope: FileScope::default(),
        tool_policy: None,
        exposed_tools: None,
        write_gate: Some(WriteGateHandle(Arc::new(TokioMutex::new(
            WriteGatePipeline::new(
                WriteMode::AutoAccept,
                Box::new(RecWrite {
                    paths: Arc::new(Mutex::new(Vec::new())),
                }),
            ),
        )))),
    };
    let mut stream = backend.stream_completion(req).await.unwrap();
    let mut events = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(ev))) => {
                let done = matches!(ev, UnifiedStreamEvent::Done(_));
                events.push(ev);
                if done {
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("{e:#}"),
            Ok(None) => break,
            Err(_) => panic!("backend fake turn timed out: {events:?}"),
        }
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UnifiedStreamEvent::TextDelta(t) if t.contains("hello from fake acp"))),
        "{events:?}"
    );
}

#[tokio::test]
async fn missing_api_key_surfaces_on_first_turn() {
    if std::env::var_os("DEEPSEEK_API_KEY").is_some()
        || std::env::var_os("GAVIERO_DSH_COMMAND").is_some()
    {
        return;
    }
    let dir = workspace();
    let args = construction(
        dir.path(),
        Box::new(NoopAcp),
        Box::new(RecWrite {
            paths: Arc::new(Mutex::new(Vec::new())),
        }),
        true,
    );
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default());
    match session.send_turn(turn("hi", true)).await {
        Ok(_) => panic!("expected missing DeepSeek API key to fail the first turn"),
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("DEEPSEEK_API_KEY") || msg.contains("secrets.toml"),
                "{msg}"
            );
        }
    }
}
