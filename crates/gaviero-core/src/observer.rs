use std::path::Path;

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

    // ── Cost tracking ───────────────────────────────────────────────

    /// Called with updated cost estimates during execution.
    fn on_cost_update(&self, _estimate: &CostEstimate) {}
}
