//! Cursor `<turn_annotations>` compliance probe.
//!
//! Drives one real Cursor turn through `registry::create_session` and asserts
//! the assistant text ends with a parseable `<turn_annotations>` sidecar.
//! Mirrors `tests/codex_annotation_compliance.rs`.
//!
//! ## Running
//!
//! Marked `#[ignore]` because it requires:
//!   - the `agent` (Cursor CLI) binary on PATH (or its alias `cursor-agent`),
//!   - a logged-in account (`agent whoami` reports `Logged in as …`),
//!   - `E2E_AGENT_MODEL` set to a cursor provider spec, e.g. `cursor:auto`.
//!
//! ```bash
//! E2E_AGENT_MODEL=cursor:auto \
//!   cargo test -p gaviero-core --test cursor_annotation_compliance -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::Mutex;

use gaviero_core::acp::protocol::TokenUsage;
use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::registry::{SessionConstruction, create_session};
use gaviero_core::agent_session::{TransportContext, build_turn};
use gaviero_core::context_planner::{
    ModelSpec, PlannerMetadata, PlannerSelections, RuntimeConfig, build_provider_profile,
};
use gaviero_core::memory::annotations::parse_and_strip;
use gaviero_core::observer::AcpObserver;
use gaviero_core::types::WriteProposal;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};

#[derive(Default)]
struct CapturingObserver {
    final_text: StdMutex<Option<String>>,
}

impl AcpObserver for CapturingObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, content: &str) {
        // The Cursor session can emit multiple `on_message_complete` calls
        // (one per assistant segment + a terminal system message on
        // cancel). Keep only the latest non-empty assistant content so
        // the sidecar parser sees the actual reply, not a "Cancelled by
        // user." postscript.
        if !content.trim().is_empty() {
            *self.final_text.lock().unwrap() = Some(content.to_string());
        }
    }
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
    fn on_turn_token_usage(&self, _usage: &TokenUsage) {}
    fn on_cursor_session_started(&self, _session_id: &str) {}
}

struct ProxyObserver {
    inner: Arc<CapturingObserver>,
}

impl AcpObserver for ProxyObserver {
    fn on_stream_chunk(&self, text: &str) {
        self.inner.on_stream_chunk(text);
    }
    fn on_tool_call_started(&self, tool_name: &str) {
        self.inner.on_tool_call_started(tool_name);
    }
    fn on_streaming_status(&self, status: &str) {
        self.inner.on_streaming_status(status);
    }
    fn on_message_complete(&self, role: &str, content: &str) {
        self.inner.on_message_complete(role, content);
    }
    fn on_proposal_deferred(&self, path: &Path, old: Option<&str>, new: &str) {
        self.inner.on_proposal_deferred(path, old, new);
    }
    fn on_turn_token_usage(&self, usage: &TokenUsage) {
        self.inner.on_turn_token_usage(usage);
    }
    fn on_cursor_session_started(&self, session_id: &str) {
        self.inner.on_cursor_session_started(session_id);
    }
}

struct NoopWriteGateObserver;
impl gaviero_core::observer::WriteGateObserver for NoopWriteGateObserver {
    fn on_proposal_created(&self, _proposal: &WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}

fn empty_planner_selections() -> PlannerSelections {
    PlannerSelections {
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        replay_history: None,
        metadata: PlannerMetadata {
            memory_count: 0,
            graph_token_estimate: 0,
            graph_budget: 0,
            is_first_turn: true,
            continuity_mode: None,
        },
    }
}

#[tokio::test]
#[ignore]
async fn cursor_emits_parseable_turn_annotations_sidecar() {
    let model_spec = std::env::var("E2E_AGENT_MODEL")
        .expect("E2E_AGENT_MODEL must point at a cursor provider, e.g. cursor:auto");
    assert!(
        model_spec.starts_with("cursor:"),
        "this probe targets the Cursor CLI; got {model_spec}"
    );

    let spec = ModelSpec::parse(&model_spec);
    let profile = build_provider_profile(&spec, &RuntimeConfig::default());

    let recorder = Arc::new(CapturingObserver::default());
    let observer_for_construction: Box<dyn AcpObserver> = Box::new(ProxyObserver {
        inner: recorder.clone(),
    });
    let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
        WriteMode::AutoAccept,
        Box::new(NoopWriteGateObserver),
    )));

    let workspace = tempfile::tempdir().expect("tempdir for cursor workspace");
    let mut session = create_session(SessionConstruction {
        write_gate,
        observer: observer_for_construction,
        model: model_spec.clone(),
        ollama_base_url: None,
        workspace_root: workspace.path().to_path_buf(),
        additional_roots: vec![],
        agent_id: "cursor-compliance".into(),
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    });

    let turn = build_turn(
        empty_planner_selections(),
        TransportContext {
            user_message:
                "Reply with one sentence acknowledging this: 'water is wet'. \
                 Then end your reply with the required <turn_annotations> JSON sidecar."
                    .into(),
            effort: None,
            auto_approve: false,
        },
    );

    let _ = session.send_turn(turn).await.expect("send_turn ok");
    session.close().await;

    let final_text = recorder
        .final_text
        .lock()
        .unwrap()
        .clone()
        .expect("on_message_complete must fire before send_turn returns");

    let parsed = parse_and_strip(&final_text);
    assert!(
        parsed.annotations.is_some(),
        "{model_spec} did not emit a parseable <turn_annotations> sidecar. \
         parse_error={:?}. Final assistant text was:\n{final_text}",
        parsed.parse_error,
    );
    assert!(
        parsed.parse_error.is_none(),
        "{model_spec} emitted a malformed <turn_annotations> sidecar: {:?}. \
         Final assistant text was:\n{final_text}",
        parsed.parse_error,
    );
}
