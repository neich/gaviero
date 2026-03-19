use std::path::Path;

use crate::swarm::models::{AgentStatus, SwarmResult};
use crate::types::WriteProposal;

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
}
