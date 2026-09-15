//! Live `dsh --profile acp` checks. Ignored by default; need `@deepseek-ai/dsh`
//! on PATH and `DEEPSEEK_API_KEY`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use futures::StreamExt;
use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::agent_client_protocol::AcpClientSession;
use gaviero_core::agent_session::registry::SessionConstruction;
use gaviero_core::agent_session::{AgentSession, Turn};
use gaviero_core::context_planner::{PlannerMetadata, RuntimeConfig, build_provider_profile};
use gaviero_core::context_planner::types::ModelSpec;
use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::swarm::backend::UnifiedStreamEvent;
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

fn dsh_acp_on_path() -> bool {
    gaviero_core::agent_session::agent_client_protocol::dsh::DshLaunchSpec::from_workspace_root(
        Path::new("."),
        &[],
    )
    .command_resolvable()
}

#[tokio::test]
#[ignore = "needs dsh + DEEPSEEK_API_KEY"]
async fn dsh_acp_smoke() {
    assert!(
        dsh_acp_on_path(),
        "dsh not on PATH (npm i -g @deepseek-ai/dsh)"
    );
    std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY");
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
        workspace_root: std::env::current_dir().unwrap(),
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
    let mut stream = session
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
        .expect("dsh ACP spawn");
    let mut saw_text = false;
    let mut saw_done = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(120), stream.next()).await {
            Ok(Some(Ok(UnifiedStreamEvent::TextDelta(_)))) => saw_text = true,
            Ok(Some(Ok(UnifiedStreamEvent::Done(_)))) => {
                saw_done = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => panic!("{e:#}"),
            Ok(None) => break,
            Err(_) => panic!("dsh_acp_smoke timed out"),
        }
    }
    assert!(saw_text && saw_done, "saw_text={saw_text} saw_done={saw_done}");
}

#[test]
#[ignore = "needs dsh + gaviero-cli"]
fn dsh_nested_reach() {
    let status = Command::new("cargo")
        .args([
            "run",
            "-p",
            "gaviero-cli",
            "--quiet",
            "--",
            "--repo",
            ".",
            "--mcp-reach-probe",
            "--reach-providers",
            "dsh",
        ])
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => panic!("dsh_nested_reach: gaviero-cli --mcp-reach-probe --reach-providers dsh failed: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs dsh + DEEPSEEK_API_KEY"]
async fn dsh_cache_hit_ratio() {
    // ACP usage_update as mapped today has input/output tokens only.
    // Live dsh-acp does not expose a cache-hit field we can pin.
    panic!("unverifiable over ACP");
}
