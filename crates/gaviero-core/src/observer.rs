use std::path::Path;
use std::time::Instant;

use crate::swarm::coordinator::TaskDAG;
use crate::swarm::models::{AgentStatus, SwarmResult};
use crate::swarm::verify::{CostEstimate, EscalationRecord, VerificationStep};
use crate::types::{ModelTier, WriteProposal};

/// Observer trait for write gate lifecycle events.
/// The TUI implements this to receive notifications and update the UI.
pub trait WriteGateObserver: Send + Sync {
    /// Called when a new proposal is created and ready for review.
    fn on_proposal_created(&self, proposal: &WriteProposal);

    /// Called when a proposal's hunk statuses have been updated.
    fn on_proposal_updated(&self, proposal_id: u64);

    /// Called when a proposal has been finalized and written to disk.
    fn on_proposal_finalized(&self, path: &str);
}

/// Observer trait for ACP (Agent Communication Protocol) events.
/// The TUI implements this to display streaming agent responses.
pub trait AcpObserver: Send + Sync {
    /// Called when a text chunk arrives from the streaming response.
    fn on_stream_chunk(&self, text: &str);

    /// Called when the agent starts executing a tool (enriched summary with details).
    fn on_tool_call_started(&self, tool_name: &str);

    /// Called to update the streaming status label (shown in the spinner).
    fn on_streaming_status(&self, status: &str);

    /// Called when a message (user/assistant/system) is complete.
    fn on_message_complete(&self, role: &str, content: &str);

    /// Called when a file proposal is deferred (batch review mode).
    /// The TUI uses this to show a compact inline diff summary during streaming.
    fn on_proposal_deferred(&self, path: &Path, old_content: Option<&str>, new_content: &str);

    /// Called when the agent subprocess needs user approval to run a tool.
    ///
    /// The pipeline blocks until the `respond` sender is used:
    /// - `respond.send(true)` → allow the tool
    /// - `respond.send(false)` (or drop) → deny the tool
    ///
    /// The default implementation auto-allows (preserves existing non-TUI behaviour).
    fn on_permission_request(
        &self,
        _tool_name: &str,
        _description: &str,
        respond: tokio::sync::oneshot::Sender<bool>,
    ) {
        let _ = respond.send(true);
    }

    // ── Inline validation (default no-op) ──────────────────────────

    /// Called after each validation gate runs on a modified file.
    fn on_validation_result(&self, _gate: &str, _passed: bool, _message: Option<&str>) {}

    /// Called when a validation failure triggers an agent retry.
    fn on_validation_retry(&self, _attempt: u8, _max_retries: u8) {}

    /// Called once per subprocess with the Claude session id captured from
    /// the `SystemInit` event. Consumers can persist this and pass it back
    /// via `AgentOptions::resume_session_id` on the next turn so Claude
    /// retains model context. Default no-op for observers that don't care.
    fn on_claude_session_started(&self, _session_id: &str) {}

    /// Called once per Cursor turn with the chat / thread id captured
    /// from the `system.init` event. Default no-op — phase 1 wires this
    /// up for ledger persistence; a follow-up milestone reads it back
    /// to pass `--resume <id>` so the Cursor session can be promoted to
    /// `NativeResume`.
    fn on_cursor_session_started(&self, _session_id: &str) {}

    /// Fired once per chat turn immediately after the prompt assembler has
    /// decided which memories to inject. Carries a summary snapshot
    /// (`items_injected`, `tokens_used`, `token_budget`) for UI / logging.
    /// The full `ChatInjection` lives on the writer path (S4 manifest).
    ///
    /// Default no-op — observers that don't care about injection telemetry
    /// pay nothing.
    fn on_memory_injected(&self, _summary: &ChatInjectionSummary) {}

    /// Fired once per chat turn when the provider reports authoritative
    /// token usage for the turn. For Claude this is parsed from the
    /// `usage` object on the `result` NDJSON event; other providers will
    /// fire it from their equivalent (e.g. Codex's `turn/completed
    /// tokenUsage`). `usage.prefix_tokens()` is the actual context size the
    /// model was conditioned on — use this in place of char-count
    /// heuristics for context-window indicators.
    ///
    /// Default no-op so observers without a usage display pay nothing.
    fn on_turn_token_usage(&self, _usage: &crate::acp::protocol::TokenUsage) {}

    /// Fired when an Option-B write tool snapshots a path mid-turn so the host
    /// can stash pre-turn content before the file watcher fires.
    fn on_tool_agent_edit_captured(&self, _path: &Path, _pre_turn_content: Option<&str>) {}

    /// Fired after a successful in-process tool-agent turn that wrote files
    /// to disk (Option B). Carries each touched path and its pre-turn content
    /// (`None` = file did not exist) so the host can open external-change
    /// review and revert on reject.
    fn on_tool_agent_edits(&self, _edits: &[ToolAgentEdit]) {}

    /// Fired once per in-process tool-agent turn with the accumulated USD
    /// cost (sum of per-round API usage). Drives the chat status-bar cost
    /// indicator for DeepSeek and future API providers.
    fn on_turn_cost_usd(&self, _cost: f64) {}
}

/// One file touched by the in-process tool-agent harness this turn.
#[derive(Debug, Clone)]
pub struct ToolAgentEdit {
    pub path: std::path::PathBuf,
    /// Pre-turn on-disk content. `None` when the file did not exist.
    pub pre_turn_content: Option<String>,
}

/// Lightweight summary of a chat memory injection decision. Handed to
/// observers via `on_memory_injected`. Full per-candidate detail lives on
/// the S4 manifest writer path.
#[derive(Debug, Clone)]
pub struct ChatInjectionSummary {
    pub items_injected: usize,
    pub pool_size: usize,
    pub tokens_used: usize,
    pub token_budget: usize,
}

/// Observer trait fired once per `AcpSession::spawn` call, after the
/// argv-vs-tempfile spill decision has been computed but before the
/// subprocess is launched. Captures the exact prompt + system-prompt
/// bytes the runtime would otherwise drop on `AcpSession::drop`
/// (because the spilled tempfile is owned by `_prompt_tempfile`).
///
/// Production callers leave `AgentOptions::prompt_observer` as `None`
/// and pay nothing (single `Option::is_some` check). Tests opt in by
/// setting the field and reading captured events.
///
/// Errors from the observer are not surfaced. The hook is
/// fire-and-forget; the subprocess spawn proceeds regardless.
pub trait PromptObserver: Send + Sync {
    fn on_prompt(&self, ev: PromptEvent);
}

/// Event payload emitted by [`PromptObserver::on_prompt`].
///
/// `prompt` carries the exact bytes that would land in
/// `.gaviero/tmp/prompt-*.md` when `used_tempfile == true`, or the
/// argv contents when `used_tempfile == false`. `system_prompt` is the
/// `--append-system-prompt` argv contents (always passed via argv,
/// never spilled).
#[derive(Debug, Clone)]
pub struct PromptEvent {
    pub turn_id: String,
    pub resume_session_id: Option<String>,
    pub prompt: String,
    pub system_prompt: String,
    pub used_tempfile: bool,
    pub argv_threshold: usize,
    pub captured_at: Instant,
}

/// Observer trait for swarm orchestration events.
/// The TUI dashboard and CLI implement this for progress reporting.
pub trait SwarmObserver: Send + Sync {
    /// Called when the swarm pipeline phase changes (e.g. "validating", "running", "merging").
    fn on_phase_changed(&self, phase: &str);

    /// Called when an agent's status changes.
    fn on_agent_state_changed(&self, work_unit_id: &str, status: &AgentStatus, detail: &str);

    /// Called when a new execution tier starts.
    fn on_tier_started(&self, current: usize, total: usize);

    /// Called when a merge conflict is detected.
    fn on_merge_conflict(&self, branch: &str, files: &[String]);

    /// Called when the swarm execution completes.
    fn on_completed(&self, result: &SwarmResult);

    // ── Coordination lifecycle (default no-op for backward compat) ──

    /// Called when the coordinator starts planning.
    fn on_coordination_started(&self, _prompt: &str) {}

    /// Called when the coordinator finishes producing a TaskDAG.
    fn on_coordination_complete(&self, _dag: &TaskDAG) {}

    /// Called when a work unit is dispatched to a specific tier/backend.
    fn on_tier_dispatch(&self, _unit_id: &str, _tier: ModelTier, _backend: &str) {}

    /// Called when a work unit is escalated to a higher tier.
    fn on_escalation(&self, _record: &EscalationRecord) {}

    // ── Verification lifecycle ──────────────────────────────────────

    /// Called when verification starts.
    fn on_verification_started(&self, _strategy: &str) {}

    /// Called when a verification step starts.
    fn on_verification_step_started(&self, _step: &VerificationStep) {}

    /// Called when a verification step completes.
    fn on_verification_step_complete(&self, _step: &VerificationStep, _passed: bool) {}

    /// Called when verification completes.
    fn on_verification_complete(&self, _passed: bool) {}

    // ── Loop lifecycle ──────────────────────────────────────────────

    /// Called when a loop iteration is about to start.
    ///
    /// `current` is 1-based; `max` is the configured `max_iterations`.
    fn on_loop_iteration_started(&self, _current: u32, _max: u32, _agents: &[String]) {}

    /// Called after each loop condition evaluation.
    ///
    /// `passed` is the raw judge/verify result; `consecutive` and `stability`
    /// reflect the running PASS streak and its required target.
    fn on_loop_verdict(&self, _passed: bool, _consecutive: u32, _stability: u32) {}

    // ── Cost tracking ───────────────────────────────────────────────

    /// Called with updated cost estimates during execution.
    fn on_cost_update(&self, _estimate: &CostEstimate) {}
}
