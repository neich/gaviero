use tokio::sync::mpsc;

use crate::event::Event;

use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::types::WriteProposal;

pub(super) struct TuiWriteGateObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl WriteGateObserver for TuiWriteGateObserver {
    fn on_proposal_created(&self, proposal: &WriteProposal) {
        let _ = self.tx.send(Event::ProposalCreated(Box::new(proposal.clone())));
    }

    fn on_proposal_updated(&self, proposal_id: u64) {
        let _ = self.tx.send(Event::ProposalUpdated(proposal_id));
    }

    fn on_proposal_finalized(&self, path: &str) {
        let _ = self.tx.send(Event::ProposalFinalized(path.to_string()));
    }
}

#[allow(dead_code)]
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
        let _ = self.tx.send(Event::SwarmCompleted(Box::new(result.clone())));
    }

    fn on_coordination_started(&self, prompt: &str) {
        let _ = self.tx.send(Event::SwarmCoordinationStarted(prompt.to_string()));
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
}
