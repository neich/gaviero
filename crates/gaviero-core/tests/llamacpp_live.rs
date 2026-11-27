//! Live probe of the `llamacpp:` chat path against a running llama.cpp
//! `llama-server`: `registry::create_session` → `ToolAgentSession` → the
//! server's OpenAI-compatible endpoint, with real tool calls.
//!
//! `#[ignore]`: it needs a server with a tool-calling model (`--jinja`). The
//! unit tests pin the request shape against recorded llama-server SSE; this
//! confirms a real multi-round turn — reasoning, `Read`, `Edit`, tool results
//! replayed with `reasoning_content` — edits the file. An `ollama:` session
//! would fail here: llama-server answers Ollama's `/api/chat` with 404.
//!
//! ```text
//! # server on http://127.0.0.1:8080/v1, else LLAMACPP_PROBE_URL=http://host:port/v1
//! cargo test -p gaviero-core --test llamacpp_live -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use futures::StreamExt;
use tokio::sync::Mutex;

use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::Turn;
use gaviero_core::agent_session::registry::{SessionConstruction, create_session};
use gaviero_core::context_planner::{
    ContinuityMode, ModelSpec, PlannerMetadata, RuntimeConfig, build_provider_profile,
};
use gaviero_core::observer::AcpObserver;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};

#[derive(Default)]
struct Recorder {
    chunks: StdMutex<String>,
    tools: StdMutex<Vec<String>>,
}

struct SharedRecorder(Arc<Recorder>);

impl AcpObserver for SharedRecorder {
    fn on_stream_chunk(&self, text: &str) {
        self.0.chunks.lock().unwrap().push_str(text);
    }
    fn on_tool_call_started(&self, tool_name: &str) {
        self.0.tools.lock().unwrap().push(tool_name.to_string());
    }
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

struct NoopWriteGateObserver;
impl gaviero_core::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &gaviero_core::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}

#[tokio::test]
#[ignore = "needs a running llama-server (see module doc)"]
async fn chat_turn_reads_and_edits_a_file_through_llama_server() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".gaviero")).unwrap();
    let base_url = std::env::var("LLAMACPP_PROBE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".to_string());
    // Through the settings cascade, as a user would configure it.
    std::fs::write(
        root.join(".gaviero/settings.json"),
        serde_json::json!({ "providers": { "llamacpp": { "baseUrl": base_url } } }).to_string(),
    )
    .unwrap();
    std::fs::write(root.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();

    let spec = "llamacpp:qwen3.8-27b";
    let profile = build_provider_profile(&ModelSpec::parse(spec), &RuntimeConfig::default());
    assert_eq!(profile.provider, "llamacpp");
    let recorder = Arc::new(Recorder::default());
    let mut session = create_session(SessionConstruction {
        write_gate: Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopWriteGateObserver),
        ))),
        observer: Box::new(SharedRecorder(recorder.clone())),
        model: spec.to_string(),
        ollama_base_url: None,
        workspace_root: root.to_path_buf(),
        additional_roots: vec![],
        agent_id: "llamacpp-live".to_string(),
        conv_id: None,
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        mcp_server: None,
    });
    assert_eq!(session.continuity_mode(), ContinuityMode::StatelessReplay);

    let turn = Turn {
        user_message: "calc.py has a bug: add() should return the sum. Read the file, fix it \
                       with an edit tool, then reply in one sentence."
            .to_string(),
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        skill_selections: vec![],
        replay_history: None,
        // A level the Qwen3.8 template accepts, so the field is really sent.
        effort: Some("low".to_string()),
        auto_approve: true,
        metadata: PlannerMetadata::default(),
    };
    let result = session.send_turn(turn).await;
    let chunks = recorder.chunks.lock().unwrap().clone();
    let tools = recorder.tools.lock().unwrap().clone();
    eprintln!("tools: {tools:?}\n--- stream ---\n{chunks}");
    let mut stream = result.unwrap_or_else(|e| panic!("turn failed: {e:#}\n{chunks}"));
    while stream.next().await.is_some() {}

    let after = std::fs::read_to_string(root.join("calc.py")).unwrap();
    assert!(after.contains("a + b"), "calc.py not fixed:\n{after}");
    assert!(!tools.is_empty(), "the model called no tool");
}
