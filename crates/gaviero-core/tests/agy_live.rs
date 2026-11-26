//! Live `agy acp` checks. Ignored by default; need the Antigravity CLI (`agy`)
//! on PATH, signed in, and a freshly built `gaviero-cli` (run `cargo build`
//! first): gaviero's agy hook runs `gaviero-cli --hook agy:pre-tool-use`, and
//! agy blocks every tool call when that hook command fails (verified with an
//! older `gaviero-cli`, agy 1.5.18).
//!
//! Run: `env -u GAVIERO_ROOT cargo test -p gaviero-core --test agy_live -- --ignored --test-threads=1`

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use futures::StreamExt;
use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::agent_client_protocol::AcpClientSession;
use gaviero_core::agent_session::agent_client_protocol::agy::AgyLaunchSpec;
use gaviero_core::agent_session::registry::SessionConstruction;
use gaviero_core::agent_session::{AgentSession, Turn};
use gaviero_core::context_planner::types::ModelSpec;
use gaviero_core::context_planner::{
    ContinuityHandle, PlannerMetadata, RuntimeConfig, build_provider_profile,
};
use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::swarm::backend::UnifiedStreamEvent;
use gaviero_core::types::{FileScope, WriteProposal};
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

const MODEL: &str = "agy:gemini-3.8-flash-low";

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

/// The directory holding the `gaviero-cli` this workspace build produced:
/// the nearest ancestor of the test binary that contains one.
fn fresh_cli_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe path");
    let name = if cfg!(windows) { "gaviero-cli.exe" } else { "gaviero-cli" };
    exe.ancestors()
        .find(|dir| dir.join(name).is_file())
        .map(Path::to_path_buf)
        .expect("no gaviero-cli next to the test build; run `cargo build` first")
}

static PATH_ONCE: Once = Once::new();

/// Put the freshly built `gaviero-cli` first on PATH, so the hook command
/// `ensure_agy_files` registers (bare `gaviero-cli` when the test binary has no
/// sibling) resolves to it inside agy. Checked to know `agy:pre-tool-use`.
fn use_fresh_gaviero_cli() {
    PATH_ONCE.call_once(|| {
        let dir = fresh_cli_dir();
        let cli = dir.join("gaviero-cli");
        let out = std::process::Command::new(&cli)
            .args(["--hook", "agy:pre-tool-use"])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run gaviero-cli");
        assert!(
            out.status.success(),
            "{} does not know agy:pre-tool-use; rebuild it: {}",
            cli.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let joined = std::env::join_paths(paths).expect("PATH");
        // SAFETY: called once, before any agy child is spawned; these tests
        // run with `--test-threads=1`.
        unsafe { std::env::set_var("PATH", joined) };
    });
}

fn require_agy() {
    assert!(
        AgyLaunchSpec::from_workspace_root(Path::new("."), &[]).command_resolvable(),
        "agy not on PATH (install the Antigravity CLI and sign in)"
    );
    use_fresh_gaviero_cli();
}

/// A committed git repository with `files`.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let repo = git2::Repository::init(dir.path()).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    dir
}

fn gate(mode: WriteMode, paths: Arc<Mutex<Vec<PathBuf>>>) -> Arc<TokioMutex<WriteGatePipeline>> {
    Arc::new(TokioMutex::new(WriteGatePipeline::new(
        mode,
        Box::new(RecWrite { paths }),
    )))
}

fn session(
    root: &Path,
    model: &str,
    mode: WriteMode,
    paths: Arc<Mutex<Vec<PathBuf>>>,
    options: AgentOptions,
) -> AcpClientSession {
    session_with_gate(root, model, gate(mode, paths), options)
}

fn session_with_gate(
    root: &Path,
    model: &str,
    write_gate: Arc<TokioMutex<WriteGatePipeline>>,
    options: AgentOptions,
) -> AcpClientSession {
    let profile = build_provider_profile(&ModelSpec::parse(model), &RuntimeConfig::default());
    let args = SessionConstruction {
        write_gate,
        observer: Box::new(NoopAcp),
        model: model.into(),
        ollama_base_url: None,
        workspace_root: root.to_path_buf(),
        additional_roots: vec![],
        agent_id: "agy".into(),
        conv_id: None,
        options,
        profile,
        cancel_token: CancellationToken::new(),
        mcp_server: None,
    };
    AcpClientSession::new_with_scope(args, FileScope::default())
}

fn turn(msg: &str) -> Turn {
    Turn {
        user_message: msg.into(),
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        skill_selections: vec![],
        replay_history: None,
        effort: None,
        auto_approve: false,
        metadata: PlannerMetadata::default(),
    }
}

async fn run(session: &mut AcpClientSession, msg: &str) -> Vec<UnifiedStreamEvent> {
    let mut stream = session.send_turn(turn(msg)).await.expect("agy send_turn");
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(240), stream.next()).await {
            Ok(Some(Ok(ev))) => {
                let done = matches!(ev, UnifiedStreamEvent::Done(_));
                out.push(ev);
                if done {
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("stream error: {e:#}"),
            Ok(None) => break,
            Err(_) => panic!("agy turn timed out; got {out:?}"),
        }
    }
    out
}

fn text(events: &[UnifiedStreamEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            UnifiedStreamEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs agy on PATH, signed in"]
async fn agy_live_turn_writes_through_reconcile() {
    require_agy();
    let dir = repo(&[("README.md", "# probe\n")]);
    // Deferred: agy's own write is reconciled into a pending proposal and the
    // tree is put back until a human accepts (the gate holds it; the write
    // observer is not called for a deferred proposal).
    let deferred = gate(WriteMode::Deferred, Arc::new(Mutex::new(Vec::new())));
    let mut s = session_with_gate(dir.path(), MODEL, deferred.clone(), AgentOptions::default());
    let events = run(
        &mut s,
        "Create a file hello.txt containing hi. Do not run any shell commands.",
    )
    .await;
    let modified = events.iter().any(|e| {
        matches!(e, UnifiedStreamEvent::PathsModified(paths)
            if paths.iter().any(|p| p.ends_with("hello.txt")))
    });
    let pending = deferred.lock().await.pending_proposals().to_vec();
    let proposed = pending.iter().any(|p| p.file_path.ends_with("hello.txt"));
    assert!(
        modified || proposed,
        "no Write Gate proposal for hello.txt: pending={:?} events={events:?}",
        pending.iter().map(|p| &p.file_path).collect::<Vec<_>>()
    );
    if proposed {
        assert!(
            !dir.path().join("hello.txt").exists(),
            "a deferred proposal must leave the tree at its turn-start state"
        );
    }
    assert!(
        events.iter().any(|e| matches!(e, UnifiedStreamEvent::TextDelta(_))),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, UnifiedStreamEvent::Usage(_))),
        "{events:?}"
    );
}

#[tokio::test]
#[ignore = "needs agy on PATH, signed in"]
async fn agy_live_hook_denies_sensitive_read() {
    require_agy();
    let secret = "hunter2-agy-live-7731";
    let dir = repo(&[("notes.txt", "public notes\n"), ("src/lib.rs", "fn x() {}\n")]);
    std::fs::write(dir.path().join(".env"), format!("SECRET={secret}\n")).unwrap();
    gaviero_core::agent_guard::agy::ensure_agy_files(
        dir.path(),
        &fresh_cli_dir().join("gaviero-cli").to_string_lossy(),
    )
    .unwrap();
    let mut s = session(
        dir.path(),
        MODEL,
        WriteMode::AutoAccept,
        Arc::new(Mutex::new(Vec::new())),
        AgentOptions::default(),
    );
    let events = run(&mut s, "Read the file .env and tell me its exact content.").await;
    let reply = text(&events);
    assert!(!reply.contains(secret), "the secret reached the model: {reply}");
    let audit = std::fs::read_to_string(dir.path().join(".gaviero/egress/hooks.ndjson"))
        .expect("hook audit log");
    assert!(
        audit.lines().any(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
            row["provider"] == "agy" && row["decision"] == "deny"
        }),
        "no agy deny row in the audit: {audit}"
    );
}

#[tokio::test]
#[ignore = "needs agy on PATH, signed in"]
async fn agy_live_resume_keeps_history() {
    require_agy();
    let dir = repo(&[("README.md", "# probe\n")]);
    let mut first = session(
        dir.path(),
        MODEL,
        WriteMode::AutoAccept,
        Arc::new(Mutex::new(Vec::new())),
        AgentOptions::default(),
    );
    let _ = run(
        &mut first,
        "Create a file resume.txt containing ok. Do not run any shell commands.",
    )
    .await;
    let id = match first.continuity_handle() {
        Some(ContinuityHandle::AcpSessionId(id)) => id.clone(),
        other => panic!("no ACP session id after turn 1: {other:?}"),
    };
    Box::new(first).close().await;

    #[allow(deprecated)]
    let options = AgentOptions {
        resume_session_id: Some(id.clone()),
        ..AgentOptions::default()
    };
    let mut second = session(
        dir.path(),
        MODEL,
        WriteMode::AutoAccept,
        Arc::new(Mutex::new(Vec::new())),
        options,
    );
    let events = run(
        &mut second,
        "What is the name of the file you created earlier in this conversation? Answer with the file name only.",
    )
    .await;
    assert!(text(&events).contains("resume.txt"), "{events:?}");
    assert_eq!(
        second.continuity_handle(),
        Some(&ContinuityHandle::AcpSessionId(id)),
        "the resumed session must keep its id"
    );
}

#[tokio::test]
#[ignore = "needs agy on PATH, signed in"]
async fn agy_live_rejected_model_fails() {
    require_agy();
    let dir = repo(&[("README.md", "# probe\n")]);
    let mut s = session(
        dir.path(),
        "agy:not-a-model",
        WriteMode::AutoAccept,
        Arc::new(Mutex::new(Vec::new())),
        AgentOptions::default(),
    );
    let err = match s.send_turn(turn("hi")).await {
        Ok(_) => panic!("agy:not-a-model must fail the turn"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("not offered"), "{err}");
}
