use tokio::sync::mpsc;

use crate::event::Event;

use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::types::WriteProposal;

pub(super) struct TuiWriteGateObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl WriteGateObserver for TuiWriteGateObserver {
    fn on_proposal_created(&self, proposal: &WriteProposal) {
        let _ = self
            .tx
            .send(Event::ProposalCreated(Box::new(proposal.clone())));
    }

    fn on_proposal_updated(&self, proposal_id: u64) {
        let _ = self.tx.send(Event::ProposalUpdated(proposal_id));
    }

    fn on_proposal_finalized(&self, path: &str) {
        let _ = self.tx.send(Event::ProposalFinalized(path.to_string()));
    }
}

pub(super) struct TuiSwarmObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl gaviero_core::observer::SwarmObserver for TuiSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        let _ = self.tx.send(Event::SwarmPhaseChanged(phase.to_string()));
    }

    fn on_agent_state_changed(
        &self,
        id: &str,
        status: &gaviero_core::swarm::models::AgentStatus,
        detail: &str,
    ) {
        let _ = self.tx.send(Event::SwarmAgentStateChanged {
            id: id.to_string(),
            status: status.clone(),
            detail: detail.to_string(),
        });
    }

    fn on_tier_started(&self, current: usize, total: usize) {
        let _ = self.tx.send(Event::SwarmTierStarted { current, total });
    }

    fn on_merge_conflict(&self, branch: &str, files: &[String]) {
        let _ = self.tx.send(Event::SwarmMergeConflict {
            branch: branch.to_string(),
            files: files.to_vec(),
        });
    }

    fn on_completed(&self, result: &gaviero_core::swarm::models::SwarmResult) {
        let _ = self
            .tx
            .send(Event::SwarmCompleted(Box::new(result.clone())));
    }

    fn on_coordination_started(&self, prompt: &str) {
        let _ = self
            .tx
            .send(Event::SwarmCoordinationStarted(prompt.to_string()));
    }

    fn on_coordination_complete(&self, dag: &gaviero_core::swarm::coordinator::TaskDAG) {
        let _ = self.tx.send(Event::SwarmCoordinationComplete {
            unit_count: dag.units.len(),
            summary: dag.plan_summary.clone(),
        });
    }

    fn on_tier_dispatch(&self, unit_id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        let _ = self.tx.send(Event::SwarmTierDispatch {
            unit_id: unit_id.to_string(),
            tier,
            backend: backend.to_string(),
        });
    }

    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        let _ = self.tx.send(Event::SwarmCostUpdate(estimate.clone()));
    }
}

pub(super) struct TuiAcpObserver {
    pub tx: mpsc::UnboundedSender<Event>,
    pub conv_id: String,
}

impl AcpObserver for TuiAcpObserver {
    fn on_stream_chunk(&self, text: &str) {
        let _ = self.tx.send(Event::StreamChunk {
            conv_id: self.conv_id.clone(),
            text: text.to_string(),
        });
    }

    fn on_tool_call_started(&self, tool_name: &str) {
        let _ = self.tx.send(Event::ToolCallStarted {
            conv_id: self.conv_id.clone(),
            tool_name: tool_name.to_string(),
        });
    }

    fn on_streaming_status(&self, status: &str) {
        let _ = self.tx.send(Event::StreamingStatus {
            conv_id: self.conv_id.clone(),
            status: status.to_string(),
        });
    }

    fn on_permission_request(
        &self,
        tool_name: &str,
        description: &str,
        respond: tokio::sync::oneshot::Sender<bool>,
    ) {
        let _ = self.tx.send(Event::PermissionRequest {
            conv_id: self.conv_id.clone(),
            tool_name: tool_name.to_string(),
            description: description.to_string(),
            respond,
        });
    }

    fn on_message_complete(&self, role: &str, content: &str) {
        let _ = self.tx.send(Event::MessageComplete {
            conv_id: self.conv_id.clone(),
            role: role.to_string(),
            content: content.to_string(),
        });
    }

    fn on_proposal_deferred(
        &self,
        path: &std::path::Path,
        old_content: Option<&str>,
        new_content: &str,
    ) {
        let old_lines = old_content.map(|s| s.lines().count()).unwrap_or(0);
        let new_lines = new_content.lines().count();
        let additions = new_lines.saturating_sub(old_lines);
        let deletions = old_lines.saturating_sub(new_lines);
        let _ = self.tx.send(Event::FileProposalDeferred {
            conv_id: self.conv_id.clone(),
            path: path.to_path_buf(),
            additions,
            deletions,
        });
    }

    fn on_claude_session_started(&self, session_id: &str) {
        let _ = self.tx.send(Event::ClaudeSessionStarted {
            conv_id: self.conv_id.clone(),
            session_id: session_id.to_string(),
        });
    }

    fn on_cursor_session_started(&self, session_id: &str) {
        let _ = self.tx.send(Event::CursorSessionStarted {
            conv_id: self.conv_id.clone(),
            session_id: session_id.to_string(),
        });
    }

    fn on_memory_injected(&self, summary: &gaviero_core::observer::ChatInjectionSummary) {
        let _ = self.tx.send(Event::ChatMemoryInjected {
            conv_id: self.conv_id.clone(),
            items_injected: summary.items_injected,
            pool_size: summary.pool_size,
            tokens_used: summary.tokens_used,
            token_budget: summary.token_budget,
        });
    }

    fn on_turn_token_usage(&self, usage: &gaviero_core::acp::protocol::TokenUsage) {
        let _ = self.tx.send(Event::TurnTokenUsage {
            conv_id: self.conv_id.clone(),
            usage: usage.clone(),
        });
    }
}

/// A4: forwards `MemoryObserver` callbacks from the writer task to the
/// TUI event loop. Fires on every write the writer task processes, so
/// the memory panel's "Recently Written" section can refresh in real
/// time. Debouncing / rate-limiting lives on the panel side.
pub(super) struct TuiMemoryObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl gaviero_core::memory::MemoryObserver for TuiMemoryObserver {
    fn on_write_enqueued(&self, kind: &str) {
        let _ = self.tx.send(Event::MemoryWriteEnqueued {
            kind: kind.to_string(),
        });
    }
    fn on_write_committed(&self, kind: &str, _result: &gaviero_core::memory::WriteResult) {
        let _ = self.tx.send(Event::MemoryWriteCommitted {
            kind: kind.to_string(),
        });
    }
    fn on_write_failed(&self, kind: &str, error: &str) {
        let _ = self.tx.send(Event::MemoryWriteFailed {
            kind: kind.to_string(),
            error: error.to_string(),
        });
    }
}

/// A4: forwards `ManifestObserver::on_manifest_persisted` to the TUI
/// event loop so the panel's "Injected Now" section can re-query the
/// just-landed manifest without polling.
pub(super) struct TuiManifestObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl gaviero_core::memory::observer::ManifestObserver for TuiManifestObserver {
    fn on_manifest_persisted(&self, turn_id: &str, session_id: &str) {
        let _ = self.tx.send(Event::MemoryManifestPersisted {
            turn_id: turn_id.to_string(),
            session_id: session_id.to_string(),
        });
    }
}

/// A5: forwards read-only MCP tool calls into the TUI event loop.
pub(super) struct TuiMcpObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl gaviero_core::mcp::McpToolCallObserver for TuiMcpObserver {
    fn on_tool_call(&self, entry: &gaviero_core::mcp::McpCallLogEntry) {
        let _ = self.tx.send(Event::McpToolCall {
            tool_name: entry.tool_name.clone(),
            duration_ms: entry.duration.as_millis() as u64,
            error: entry.error.clone(),
        });
    }
}
