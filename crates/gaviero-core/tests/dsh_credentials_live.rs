//! Live check that gaviero's credential patch makes dsh use the key gaviero
//! injects instead of a key stored in `~/.dsh/.credentials.yaml`. Ignored by
//! default; needs dsh 0.2.1-rc.2 on PATH, a stored dsh key, and network.
//!
//! The only test in this binary, because it mutates process env.

use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::agent_client_protocol::AcpClientSession;
use gaviero_core::agent_session::registry::SessionConstruction;
use gaviero_core::agent_session::{AgentSession, Turn};
use gaviero_core::context_planner::types::ModelSpec;
use gaviero_core::context_planner::{PlannerMetadata, RuntimeConfig, build_provider_profile};
use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::swarm::backend::{StopReason, UnifiedStreamEvent};
use gaviero_core::types::{FileScope, WriteProposal};
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

struct NoopAcp;
impl AcpObserver for NoopAcp {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

struct NoopWrite;
impl WriteGateObserver for NoopWrite {
    fn on_proposal_created(&self, _proposal: &WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}

/// What one live turn produced. Error strings only; no event payload that
/// could carry a key is kept.
#[derive(Debug, Default)]
struct Outcome {
    saw_text: bool,
    saw_usage: bool,
    errors: Vec<String>,
    done: Option<StopReason>,
}

async fn run_turn(root: &Path) -> Outcome {
    let spec = ModelSpec::parse("dsh:deepseek-v4-flash");
    let profile = build_provider_profile(&spec, &RuntimeConfig::default());
    #[allow(deprecated)]
    let args = SessionConstruction {
        write_gate: std::sync::Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWrite),
        ))),
        observer: Box::new(NoopAcp),
        model: "dsh:deepseek-v4-flash".into(),
        ollama_base_url: None,
        workspace_root: root.to_path_buf(),
        additional_roots: vec![],
        agent_id: "dsh".into(),
        conv_id: None,
        options: AgentOptions {
            auto_approve: true,
            ..AgentOptions::default()
        },
        profile,
        cancel_token: CancellationToken::new(),
        mcp_server: None,
    };
    let mut session = AcpClientSession::new_with_scope(args, FileScope::default());
    let mut out = Outcome::default();
    let mut stream = match session
        .send_turn(Turn {
            user_message: "Reply with the single word pong.".into(),
            memory_selections: vec![],
            graph_selections: vec![],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: None,
            effort: Some("off".into()),
            auto_approve: true,
            metadata: PlannerMetadata::default(),
        })
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            out.errors.push(format!("send_turn: {e:#}"));
            out.done = Some(StopReason::Error);
            return out;
        }
    };
    loop {
        match tokio::time::timeout(Duration::from_secs(180), stream.next()).await {
            Ok(Some(Ok(UnifiedStreamEvent::TextDelta(_)))) => out.saw_text = true,
            Ok(Some(Ok(UnifiedStreamEvent::Usage(u)))) if u.input_tokens > 0 => {
                out.saw_usage = true
            }
            Ok(Some(Ok(UnifiedStreamEvent::Error(e)))) => out.errors.push(e),
            Ok(Some(Ok(UnifiedStreamEvent::Done(reason)))) => {
                out.done = Some(reason);
                break;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                out.errors.push(format!("stream: {e:#}"));
                break;
            }
            Ok(None) => break,
            Err(_) => panic!("dsh turn timed out after 180 s without an event; {out:?}"),
        }
    }
    drop(stream);
    Box::new(session).close().await;
    out
}

fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

#[tokio::test]
#[ignore = "needs dsh 0.2.1-rc.2, a stored ~/.dsh key, and network"]
async fn dsh_stored_key_bypassed_only_with_gaviero_patch() {
    let stored = dirs::home_dir()
        .map(|h| h.join(".dsh").join(".credentials.yaml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    if !stored.contains("DEEPSEEK_API_KEY:") {
        panic!("precondition: no stored DEEPSEEK_API_KEY ref in ~/.dsh/.credentials.yaml");
    }

    // Env wins in gaviero's key resolution, so gaviero injects this bogus key.
    unsafe {
        std::env::set_var("DEEPSEEK_API_KEY", "sk-gaviero-bogus-0000000000000000"); // pragma: allowlist secret
    }

    // Arm A: default credentialSource ("gaviero") → patch on → dsh must use
    // the bogus key and fail.
    let ws_a = tempdir();
    let a = run_turn(ws_a.path()).await;

    // Arm B: credentialSource "dsh" → no patch → dsh's stored key wins.
    let ws_b = tempdir();
    let gav = ws_b.path().join(".gaviero");
    std::fs::create_dir_all(&gav).unwrap();
    std::fs::write(
        gav.join("settings.json"),
        r#"{"providers":{"dsh":{"credentialSource":"dsh"}}}"#,
    )
    .unwrap();
    let b = run_turn(ws_b.path()).await;

    unsafe {
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    assert!(
        !a.saw_text && (!a.errors.is_empty() || a.done == Some(StopReason::Error)),
        "arm A (patch on, bogus key) should fail without text: {a:?}"
    );
    assert!(
        b.saw_text && b.done == Some(StopReason::EndTurn) && b.saw_usage,
        "arm B (no patch, stored key) should answer with usage: {b:?}"
    );
}
