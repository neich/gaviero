//! `<turn_annotations>` compliance probes for the providers that had none.
//!
//! `plans/provider-parity` §3.5. `tests/codex_annotation_compliance.rs` and
//! `tests/cursor_annotation_compliance.rs` cover the two config-file CLIs; dsh,
//! `deepseek:` and `ollama:` had no probe at all, so "annotations are already
//! universal" (§2.1) was an assertion about the two providers someone happened
//! to test. This file closes that gap.
//!
//! **One file, three probes** — deliberately *not* the one-file-per-provider
//! layout the two existing probes use. The harness (observer pair, write gate,
//! empty planner selections, turn construction) is ~120 lines of boilerplate
//! that would otherwise be copied three more times, and the per-provider part
//! is a four-line assertion. Divergence from that convention is intentional;
//! the assertion each probe makes is identical to the existing two.
//!
//! ## Running
//!
//! All three are `#[ignore]`d: each drives a real model turn and needs the
//! provider to be reachable.
//!
//! ```bash
//! # dsh (needs the dsh binary on PATH)
//! E2E_AGENT_MODEL=dsh:x \
//!   cargo test -p gaviero-core --test provider_annotation_compliance -- --ignored --nocapture
//!
//! # ollama (needs a running daemon; override the base URL if non-default)
//! E2E_AGENT_MODEL=ollama:llama3 E2E_OLLAMA_BASE_URL=http://127.0.0.1:11434 \
//!   cargo test -p gaviero-core --test provider_annotation_compliance -- --ignored --nocapture
//!
//! # deepseek
//! E2E_AGENT_MODEL=deepseek:deepseek-chat \
//!   cargo test -p gaviero-core --test provider_annotation_compliance -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::Mutex;

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
        // A turn can emit several `on_message_complete` calls (one per
        // assistant segment, plus a terminal system message on cancel). Keep
        // the latest non-empty content so the parser sees the reply rather
        // than a "Cancelled by user." postscript.
        if !content.trim().is_empty() {
            *self.final_text.lock().unwrap() = Some(content.to_string());
        }
    }
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
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
    }
}

/// Drive one real turn for `expected_prefix` and assert the sidecar parses.
///
/// `E2E_AGENT_MODEL` must name a spec with `expected_prefix`, so the same
/// command shape works for all three probes and a mis-set variable fails with
/// the probe's own name in the message rather than a confusing parse error.
async fn run_probe(expected_prefix: &str, ollama_base_url: Option<String>) {
    let model_spec = std::env::var("E2E_AGENT_MODEL").unwrap_or_else(|_| {
        panic!("E2E_AGENT_MODEL must point at a {expected_prefix} provider spec")
    });
    assert!(
        model_spec.starts_with(expected_prefix),
        "this probe targets `{expected_prefix}`; got {model_spec}"
    );

    let profile = build_provider_profile(&ModelSpec::parse(&model_spec), &RuntimeConfig::default());

    let recorder = Arc::new(CapturingObserver::default());
    let observer: Box<dyn AcpObserver> = Box::new(ProxyObserver {
        inner: recorder.clone(),
    });
    let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
        WriteMode::AutoAccept,
        Box::new(NoopWriteGateObserver),
    )));

    let workspace = tempfile::tempdir().expect("tempdir for the probe workspace");
    let mut session = create_session(SessionConstruction {
        write_gate,
        observer,
        model: model_spec.clone(),
        ollama_base_url,
        workspace_root: workspace.path().to_path_buf(),
        additional_roots: vec![],
        agent_id: format!("{expected_prefix}-compliance"),
        conv_id: None,
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        mcp_server: None,
    });

    let turn = build_turn(
        empty_planner_selections(),
        TransportContext {
            user_message: "Reply with one sentence acknowledging this: 'water is wet'. \
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

#[tokio::test]
#[ignore]
async fn dsh_emits_parseable_turn_annotations_sidecar() {
    run_probe("dsh:", None).await;
}

#[tokio::test]
#[ignore]
async fn deepseek_emits_parseable_turn_annotations_sidecar() {
    run_probe("deepseek:", None).await;
}

#[tokio::test]
#[ignore]
async fn ollama_emits_parseable_turn_annotations_sidecar() {
    let base = std::env::var("E2E_OLLAMA_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    run_probe("ollama:", Some(base)).await;
}
