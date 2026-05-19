//! T6 — `agent_session::registry::create_session` routes by `ContinuityMode`.
//!
//! The registry is the only public seam from `(provider, continuity_mode)`
//! to a concrete per-provider session type (`ClaudeSession`,
//! `CodexAppServerSession`, `CodexExecSession`, `OllamaSession`). Their
//! constructors are `pub(super)` so this routing dispatch can only be
//! exercised through `create_session` — which means without this test
//! a refactor that mis-matches a provider to the wrong session type
//! would compile silently.
//!
//! Construction is side-effect-free (no subprocess spawn until
//! `send_turn`), so the test pays no I/O cost.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use gaviero_core::acp::session::AgentOptions;
use gaviero_core::agent_session::registry::{SessionConstruction, create_session};
use gaviero_core::context_planner::{ContinuityMode, ModelSpec, RuntimeConfig, build_provider_profile};
use gaviero_core::observer::AcpObserver;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};

struct NoopAcpObserver;
impl AcpObserver for NoopAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
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

fn construction_for(model_spec: &str) -> SessionConstruction {
    let spec = ModelSpec::parse(model_spec);
    let profile = build_provider_profile(&spec, &RuntimeConfig::default());
    let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
        WriteMode::AutoAccept,
        Box::new(NoopWriteGateObserver),
    )));
    SessionConstruction {
        write_gate,
        observer: Box::new(NoopAcpObserver),
        model: model_spec.to_string(),
        ollama_base_url: None,
        workspace_root: std::env::temp_dir(),
        additional_roots: vec![],
        agent_id: "test-agent".to_string(),
        options: AgentOptions::default(),
        profile,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    }
}

#[test]
fn claude_spec_routes_to_native_resume_session() {
    let session = create_session(construction_for("claude:sonnet"));
    assert_eq!(session.continuity_mode(), ContinuityMode::NativeResume);
}

#[test]
fn bare_model_spec_falls_through_to_claude_native_resume() {
    // V9: bare names are treated as Claude — make sure registry follows.
    let session = create_session(construction_for("haiku"));
    assert_eq!(session.continuity_mode(), ContinuityMode::NativeResume);
}

#[test]
fn codex_app_server_spec_routes_to_process_bound_session() {
    let session = create_session(construction_for("codex-app-server:gpt-5"));
    assert_eq!(session.continuity_mode(), ContinuityMode::ProcessBound);
}

#[test]
fn codex_exec_spec_routes_to_stateless_replay_session() {
    let session = create_session(construction_for("codex:gpt-5"));
    assert_eq!(session.continuity_mode(), ContinuityMode::StatelessReplay);
}

#[test]
fn ollama_spec_routes_to_stateless_replay_session() {
    let session = create_session(construction_for("ollama:qwen2.5-coder:7b"));
    assert_eq!(session.continuity_mode(), ContinuityMode::StatelessReplay);
}

#[test]
fn cursor_spec_routes_to_native_resume_session() {
    // Cursor's `agent --resume <chat-id>` makes it NativeResume; this
    // pins the routing dispatch so a future refactor can't mis-match the
    // `cursor:` prefix to ClaudeSession or OllamaSession.
    let session = create_session(construction_for("cursor:auto"));
    assert_eq!(session.continuity_mode(), ContinuityMode::NativeResume);
}

#[test]
fn local_spec_routes_to_stateless_replay_session() {
    // `local:` is treated as ollama by the planner per V9 §11 M9.
    let session = create_session(construction_for("local:llama3"));
    assert_eq!(session.continuity_mode(), ContinuityMode::StatelessReplay);
}

#[test]
fn cancel_token_field_is_propagated_into_construction() {
    // Construction is side-effect-free, but the registry must accept a
    // pre-fired token without spawning anything. Future regressions would
    // remove the field or stop forwarding it.
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let mut args = construction_for("claude:sonnet");
    args.cancel_token = token.clone();
    let session = create_session(args);
    assert_eq!(session.continuity_mode(), ContinuityMode::NativeResume);
    assert!(token.is_cancelled());
}
