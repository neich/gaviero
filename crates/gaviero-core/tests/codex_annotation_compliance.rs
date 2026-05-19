//! WU3 — Codex `<turn_annotations>` compliance probe.
//!
//! Drives one real Codex turn through `registry::create_session` and asserts
//! the assistant text ends with a parseable `<turn_annotations>` sidecar.
//! If this test fails for a given Codex provider, the fix is prompt
//! engineering in `swarm/backend/shared.rs::default_editor_system_prompt`
//! (not a Gaviero architectural change) — the registry+observer+memory
//! plumbing is independently verified by
//! `agent_session::registry::tests::dispatches_stream_events_through_acp_observer_and_fires_message_complete`.
//!
//! ## Running
//!
//! Marked `#[ignore]` because it requires:
//!   - the `codex` CLI on PATH,
//!   - a model accessible to that CLI,
//!   - `E2E_AGENT_MODEL` set to a codex provider spec (e.g.
//!     `codex-app-server:gpt-5`, `codex:gpt-5`).
//!
//! ```bash
//! E2E_AGENT_MODEL=codex-app-server:gpt-5 \
//!   cargo test -p gaviero-core --test codex_annotation_compliance -- --ignored --nocapture
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

/// Records `on_message_complete` content so the test can parse the final
/// assistant text for the `<turn_annotations>` sidecar.
#[derive(Default)]
struct CapturingObserver {
    streamed: StdMutex<String>,
    final_text: StdMutex<Option<String>>,
}

impl AcpObserver for CapturingObserver {
    fn on_stream_chunk(&self, text: &str) {
        self.streamed.lock().unwrap().push_str(text);
    }
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, content: &str) {
        *self.final_text.lock().unwrap() = Some(content.to_string());
    }
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
    fn on_turn_token_usage(&self, _usage: &TokenUsage) {}
}

/// Forwarder so we can hold an `Arc<CapturingObserver>` for read-back while
/// SessionConstruction takes ownership via `Box<dyn AcpObserver>`.
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
async fn codex_emits_parseable_turn_annotations_sidecar() {
    let model_spec = std::env::var("E2E_AGENT_MODEL")
        .expect("E2E_AGENT_MODEL must point at a codex provider, e.g. codex-app-server:gpt-5");
    assert!(
        model_spec.starts_with("codex:") || model_spec.starts_with("codex-app-server:"),
        "this probe targets codex providers; got {model_spec}"
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

    let workspace = std::env::temp_dir();
    let mut session = create_session(SessionConstruction {
        write_gate,
        observer: observer_for_construction,
        model: model_spec.clone(),
        ollama_base_url: None,
        workspace_root: workspace,
        additional_roots: vec![],
        agent_id: "codex-compliance".into(),
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    });

    // The system prompt already teaches the <turn_annotations> convention
    // (default_editor_system_prompt). The user message just gives Codex
    // something innocuous to acknowledge — the sidecar must show up on
    // every reply regardless.
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

    // ObservedStreamSession consumes the inner stream and returns an empty
    // one — discard explicitly so #[warn(unused_must_use)] doesn't fire.
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
