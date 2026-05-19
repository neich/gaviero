//! Transport-side types for provider sessions (V9 §11 M5).
//!
//! This module is the transport boundary for the provider-session refactor:
//! it lands `Turn` (V9 §4), `AgentSession` (V9 §4), and the `build_turn`
//! conversion from the planner's [`crate::context_planner::PlannerSelections`]
//! to a transport payload.
//!
//! **Type ownership (V9 §0 rule 12, §4):**
//! * M1 owns planner-side types (`PlannerSelections`, `SessionLedger`, ...).
//! * M5 owns transport-side types (this module). `Turn` is a *thin lossless
//!   lift* of `PlannerSelections` — `build_turn` below is the conversion and
//!   is covered by a round-trip unit test per V9 §11 M5 acceptance.
//!
//! **Legacy shim (M5 only).** The real per-provider `AgentSession` impls land
//! in later milestones:
//! * M6 — Claude (`agent_session/claude.rs`)
//! * M8 — Codex (`agent_session/codex.rs`)
//! * M9 — Ollama (`agent_session/ollama.rs`)
//!
//! M5 ships [`LegacyAgentSession`], a thin wrapper around the existing
//! [`crate::acp::AcpPipeline`] so the chat path can exercise the trait
//! surface today without a per-provider rewrite. The shim's
//! `send_turn` reconstructs the legacy inputs from a `Turn` and calls
//! the existing pipeline; V9 §0 rule 6 forbids deleting the legacy path
//! before M10 parity is proven.
//!
//! ## Adding a new CLI agent provider with Claude-equivalent behavior
//!
//! Every provider must surface the same three contracts the user sees in
//! Claude: visible reasoning/tool activity, file edits routed through the
//! Write Gate review flow, and post-turn `<turn_annotations>` memory
//! extraction. Two integration patterns are supported; pick one — don't
//! invent a third observer wiring path.
//!
//! ### Pattern A — one-shot subprocess (no native session state)
//!
//! Implement `swarm::backend::AgentBackend` in
//! `crates/gaviero-core/src/swarm/backend/<name>.rs`. Reuse
//! `AcpPipeline::send_prompt` → `swarm::backend::executor::complete_to_write_gate`,
//! which drives the observer (reasoning, tool starts, status, token usage,
//! `on_message_complete`) and routes `UnifiedStreamEvent::FileBlock`
//! events through the Write Gate. Wrap the backend in `LegacyAgentSession`
//! inside `registry::create_session`. Used today by `codex:` (exec) and
//! `ollama:`/`local:`.
//!
//! ### Pattern B — process-bound, native session protocol
//!
//! Implement `AgentSession` directly so `send_turn` returns a
//! `Stream<Item = Result<UnifiedStreamEvent>>`. Register the model-spec
//! prefix in `registry::create_session`; the registry wraps the result in
//! `ObservedStreamSession`, which drains the stream and calls the same
//! observer callbacks Pattern A uses. Used today by `codex-app-server:`.
//!
//! ### Provider checklist
//!
//! 1. Emit `UnifiedStreamEvent::TextDelta` for visible reply text and
//!    `ThinkingDelta` for reasoning. Pattern B's wrapper frames reasoning
//!    with `<think>` / `</think>` automatically; Pattern A's executor does
//!    the same.
//! 2. Emit `ToolCallStart` for every tool/command launch — drives the chat
//!    panel's "Using X..." indicator.
//! 3. Include `swarm::backend::shared::default_editor_system_prompt` in the
//!    developer/system instructions so the model is taught the `<file>`
//!    block convention (when `Capabilities::supports_file_blocks`) and the
//!    `<turn_annotations>` sidecar (always). Both Pattern A and Pattern B
//!    rely on the same system prompt.
//! 4. File edits flow through one of two channels and no others: the
//!    backend's native tool-call channel (only when the prompt teaches
//!    tool use, not `<file>`) — e.g. Claude's Write/Edit/MultiEdit — OR
//!    in-band `<file path="…">…</file>` blocks parsed by the wrapper /
//!    executor. Backends that advertise `supports_file_blocks=false` must
//!    not emit `FileBlock`; `complete_to_write_gate` debug-asserts this
//!    invariant.
//! 5. Emit `Usage` once per turn so the TUI status bar's token counter
//!    updates.
//! 6. Terminate with `Done`. The wrapper / executor calls
//!    `on_message_complete` immediately after, which is what
//!    `controller.rs::Event::MessageComplete` keys off to invoke
//!    `enqueue_post_turn` for memory extraction.
//!
//! No additional observer plumbing is needed. Permission requests are not
//! supported through `UnifiedStreamEvent`; providers that need on-request
//! approvals must run with `approvalPolicy: "never"` until a future event
//! variant is added.

pub mod claude;
pub mod codex_app_server;
pub mod codex_exec;
pub mod cursor;
pub mod ollama;
pub mod registry;

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;

use crate::acp::client::AcpPipeline;
use crate::acp::session::AgentOptions;
use crate::context_planner::ledger::Role;
use crate::context_planner::{
    ContinuityHandle, ContinuityMode, FileAttachment, GraphSelection, MemorySelection,
    PlannerMetadata, PlannerSelections, ProviderProfile, ReplayPayload,
};
use crate::observer::AcpObserver;
use crate::swarm::backend::UnifiedStreamEvent;
use crate::write_gate::WriteGatePipeline;

// ── Transport payload (V9 §4) ──────────────────────────────────

/// Transport-facing fields that aren't part of `PlannerSelections`.
///
/// Split out from `Turn` so `build_turn`'s contract is "take everything the
/// planner emitted + these runtime knobs → Turn". Keeps the lossless-lift
/// invariant visible in the signature.
#[derive(Debug, Clone)]
pub struct TransportContext {
    /// The new user message this turn dispatches.
    pub user_message: String,
    /// Provider-specific reasoning effort ("low"/"medium"/"high"/"xhigh"/"max"/"off"/"auto").
    /// `None`, `"off"`, or `"auto"` = provider/model default.
    pub effort: Option<String>,
    /// Whether the user pre-approved writes for this turn.
    pub auto_approve: bool,
}

/// V9 §4 `Turn`: the transport payload a session consumes.
///
/// **Lossless lift of `PlannerSelections`.** The five shared fields
/// (`memory_selections`, `graph_selections`, `file_refs`, `replay_history`,
/// `metadata`) must appear unchanged after `build_turn` — V9 §11 M5 requires
/// a unit test that round-trips representative values and asserts field-by-
/// field equality. See [`build_turn`] and `tests::lossless_round_trip`.
#[derive(Debug, Clone)]
pub struct Turn {
    pub user_message: String,
    pub memory_selections: Vec<MemorySelection>,
    pub graph_selections: Vec<GraphSelection>,
    pub file_refs: Vec<FileAttachment>,
    pub replay_history: Option<ReplayPayload>,
    pub effort: Option<String>,
    pub auto_approve: bool,
    pub metadata: PlannerMetadata,
}

/// Lift `PlannerSelections` + `TransportContext` into a transport `Turn`.
///
/// **Contract (V9 §4):** this conversion is lossless over every shared
/// field. `build_turn` takes ownership of `sel` and moves the five shared
/// collections into the resulting `Turn` unchanged. No transport code
/// reads `PlannerSelections` directly — it reads `Turn`.
///
/// Unit test in this module's `tests` submodule pins the round-trip
/// equality; do not simplify this function without updating the test.
pub fn build_turn(sel: PlannerSelections, ctx: TransportContext) -> Turn {
    Turn {
        user_message: ctx.user_message,
        memory_selections: sel.memory_selections,
        graph_selections: sel.graph_selections,
        file_refs: sel.file_refs,
        replay_history: sel.replay_history,
        effort: ctx.effort,
        auto_approve: ctx.auto_approve,
        metadata: sel.metadata,
    }
}

// ── AgentSession trait (V9 §4) ────────────────────────────────

/// Provider-agnostic session trait consumed by chat (M5), swarm (M7), and
/// compaction (M9). Replaces the direct-call + observer coupling in
/// `AcpPipeline` once per-provider implementations land in M6/M8/M9.
///
/// M5 ships this trait alongside the [`LegacyAgentSession`] shim so
/// the chat path can call through it immediately. Per-provider impls
/// (`ClaudeSession`, `CodexSession`, `OllamaSession`) follow one milestone
/// at a time — V9 §0 rule 6, §11 M5 forbidden shortcut "do not migrate
/// all providers at once".
#[async_trait::async_trait]
pub trait AgentSession: Send + Sync {
    /// Dispatch a turn and return a stream of normalized events.
    ///
    /// The M5 shim returns an empty stream because the observer pattern
    /// (wired by the caller) already carries all events; M6+ per-provider
    /// implementations return real streams and the caller migrates to
    /// consuming them directly.
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;

    /// Provider continuity mode (from `ProviderProfile.continuity_mode`).
    fn continuity_mode(&self) -> ContinuityMode;

    /// Current continuity handle if the session holds one.
    ///
    /// M5 shim always returns `None` (the chat caller tracks the handle in
    /// `SessionLedger::continuity_handle`). M6 wires this to the live
    /// Claude session id so the ledger can read from the session directly.
    fn continuity_handle(&self) -> Option<&ContinuityHandle>;

    /// Tear down the session. M5 shim is a no-op; M6+ per-provider impls
    /// may release subprocess handles or tokens here.
    async fn close(self: Box<Self>);
}

// ── Legacy shim (M5 only) ─────────────────────────────────────

/// M5 shim implementation of [`AgentSession`] that wraps the existing
/// [`AcpPipeline`] so the chat path can call through the new trait without
/// a per-provider rewrite. Each call to `send_turn` reconstructs the legacy
/// input tuple (`enriched_prompt, file_refs, history, attachments`) from
/// the `Turn` and calls [`AcpPipeline::send_prompt`] unchanged.
///
/// The shim uses the shared renderer helpers from
/// `swarm::backend::shared` to collapse `graph_selections` + `memory_selections`
/// back into the legacy enriched-prompt string format — byte-identical to
/// the M2/M3 chat output.
///
/// **Lifecycle: M5 introduces, M6 replaces for Claude, M8 for Codex,
/// M9 for Ollama, M10 deletes.**
pub struct LegacyAgentSession {
    pipeline: AcpPipeline,
    profile: ProviderProfile,
    /// Mirror of the caller-tracked continuity handle. Exposed via
    /// [`AgentSession::continuity_handle`] so M6's real `ClaudeSession`
    /// can drop this field without changing the public API.
    handle: Option<ContinuityHandle>,
}

impl LegacyAgentSession {
    /// Construct a shim session. Callers typically go through
    /// [`registry::create_session`] rather than calling this directly.
    // M6: reads deprecated `resume_session_id`; allow stays until M10.
    #[allow(deprecated)]
    pub fn new(
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        observer: Box<dyn AcpObserver>,
        model: String,
        ollama_base_url: Option<String>,
        workspace_root: PathBuf,
        additional_roots: Vec<PathBuf>,
        agent_id: String,
        options: AgentOptions,
        profile: ProviderProfile,
    ) -> Self {
        let handle: Option<ContinuityHandle> = options
            .resume_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|id| ContinuityHandle::ClaudeSessionId(id.to_string()));
        let pipeline = AcpPipeline::new(
            write_gate,
            observer,
            model,
            ollama_base_url,
            workspace_root,
            additional_roots,
            agent_id,
            options,
        );
        Self {
            pipeline,
            profile,
            handle,
        }
    }
}

#[async_trait::async_trait]
impl AgentSession for LegacyAgentSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        // Reconstruct the legacy enriched prompt from the Turn's structured
        // selections. `render_graph_block` + `render_memory_block` live in
        // `swarm::backend::shared` (introduced in M3). The user message is
        // placed FIRST so it survives Claude's default 2000-line Read
        // truncation when the assembled blob is spilled to a tempfile on
        // bootstrap-heavy first turns (mirrors `ClaudeSession::run_claude_turn`).
        let mut parts: Vec<String> = Vec::new();
        parts.push(turn.user_message.clone());
        if let Some(block) =
            crate::swarm::backend::shared::render_graph_block(&turn.graph_selections)
        {
            parts.push(block);
        }
        if let Some(block) =
            crate::swarm::backend::shared::render_memory_block(&turn.memory_selections)
        {
            parts.push(block);
        }
        let enriched_prompt = parts.join("\n\n");

        // Lift `replay_history` back into the legacy `Vec<(String, String)>`
        // shape. Unused today (chat uses Claude `--resume` for continuity)
        // but kept on the transport so M9 (Ollama) has somewhere to put
        // real client-side replay.
        let history: Vec<(String, String)> = turn
            .replay_history
            .map(|p| {
                p.entries
                    .into_iter()
                    .map(|(r, c)| (role_to_string(r), c))
                    .collect()
            })
            .unwrap_or_default();

        // Split FileAttachment back into the two legacy inputs:
        // * `file_refs: &[(String, String)]` — path + contents (text).
        // * `file_attachments: &[PathBuf]` — path only (images, documents
        //   routed via Claude's --file flag).
        let mut file_refs: Vec<(String, String)> = Vec::new();
        let mut file_attachments: Vec<PathBuf> = Vec::new();
        for f in turn.file_refs {
            match f.content {
                Some(text) => file_refs.push((f.path.to_string_lossy().into_owned(), text)),
                None => file_attachments.push(f.path),
            }
        }

        self.pipeline
            .send_prompt(&enriched_prompt, &file_refs, &history, &file_attachments)
            .await?;

        // M5 shim returns an empty stream — all events flow through the
        // observer injected at construction. M6 (Claude) + M8/M9 return
        // real streams and callers migrate.
        Ok(Box::pin(futures::stream::empty()))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.profile.continuity_mode
    }

    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        self.handle.as_ref()
    }

    async fn close(self: Box<Self>) {
        // AcpPipeline has no explicit close today; subprocesses are spawned
        // per send_prompt and torn down when that future resolves. M6's
        // real ClaudeSession wires a proper close path.
    }
}

fn role_to_string(r: Role) -> String {
    match r {
        Role::User => "user".to_string(),
        Role::Assistant => "assistant".to_string(),
        Role::System => "system".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::types::{GraphSelectionKind, PlannerMetadata};
    use std::path::PathBuf;

    fn sample_selections() -> PlannerSelections {
        PlannerSelections {
            memory_selections: vec![MemorySelection {
                id: Some(42),
                namespace: Some("ws".to_string()),
                scope_label: Some("workspace".to_string()),
                score: Some(3.14),
                trust: None,
                content: "remember this".to_string(),
                source_hash: None,
                updated_at: Some("2026-04-14T00:00:00Z".to_string()),
            }],
            graph_selections: vec![GraphSelection {
                path: Some(PathBuf::from("src/lib.rs")),
                kind: GraphSelectionKind::FullContent,
                token_estimate: 500,
                content: "  OWN src/lib.rs".to_string(),
                rank_score: Some(0.6),
                confidence: Some(crate::repo_map::GraphConfidence::High),
                symbols: vec![],
                content_digest: None,
            }],
            file_refs: vec![FileAttachment {
                path: PathBuf::from("Cargo.toml"),
                content: Some("[package]\nname = \"x\"\n".to_string()),
            }],
            replay_history: Some(ReplayPayload {
                entries: vec![(Role::User, "hi".to_string())],
            }),
            metadata: PlannerMetadata {
                memory_count: 1,
                graph_token_estimate: 500,
                graph_budget: 8000,
                is_first_turn: true,
                continuity_mode: Some(ContinuityMode::NativeResume),
            },
        }
    }

    #[test]
    fn m5_build_turn_is_lossless_over_shared_fields() {
        // V9 §4 contract + §11 M5 acceptance: every bit in PlannerSelections
        // must appear unchanged in the resulting Turn.
        let sel = sample_selections();
        let snapshot = sel.clone();
        let ctx = TransportContext {
            user_message: "do the thing".to_string(),
            effort: Some("medium".to_string()),
            auto_approve: true,
        };
        let turn = build_turn(sel, ctx.clone());

        // Shared fields: field-by-field equality.
        assert_eq!(turn.memory_selections, snapshot.memory_selections);
        assert_eq!(turn.graph_selections.len(), snapshot.graph_selections.len());
        // GraphSelection contains `GraphConfidence` which lacks PartialEq by
        // default — compare the non-confidence fields explicitly.
        for (a, b) in turn
            .graph_selections
            .iter()
            .zip(snapshot.graph_selections.iter())
        {
            assert_eq!(a.path, b.path);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.token_estimate, b.token_estimate);
            assert_eq!(a.content, b.content);
            assert_eq!(a.rank_score, b.rank_score);
            assert_eq!(a.symbols, b.symbols);
            assert_eq!(a.content_digest, b.content_digest);
        }
        assert_eq!(turn.file_refs, snapshot.file_refs);
        assert_eq!(turn.replay_history, snapshot.replay_history);
        assert_eq!(turn.metadata, snapshot.metadata);

        // Transport fields: lifted from context.
        assert_eq!(turn.user_message, ctx.user_message);
        assert_eq!(turn.effort, ctx.effort);
        assert_eq!(turn.auto_approve, ctx.auto_approve);
    }

    #[test]
    fn m5_build_turn_consumes_selections_by_move() {
        // Compile-time contract: `build_turn` takes ownership so downstream
        // code can't accidentally hold a second source of truth for the
        // same data. Not a runtime assertion — this test simply won't
        // compile if the signature loses its owning parameter.
        let sel = sample_selections();
        let ctx = TransportContext {
            user_message: "m".to_string(),
            effort: None,
            auto_approve: false,
        };
        let _turn = build_turn(sel, ctx);
        // `sel` cannot be used here — moved.
    }
}
