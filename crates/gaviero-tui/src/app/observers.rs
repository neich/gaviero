use std::sync::Arc;

use tokio::sync::mpsc;

use crate::event::Event;

use gaviero_core::history::HistoryRecorder;
use gaviero_core::observer::{AcpObserver, ToolCallOutcome, ToolOutputOutcome, WriteGateObserver};
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

pub(super) struct TuiAcpObserver {
    pub tx: mpsc::UnboundedSender<Event>,
    pub conv_id: String,
    /// The turn this observer was built for. History capture is keyed by it,
    /// never by the conversation.
    pub turn_id: String,
    pub history: Arc<HistoryRecorder>,
}

impl AcpObserver for TuiAcpObserver {
    fn on_stream_chunk(&self, text: &str) {
        let _ = self.tx.send(Event::StreamChunk {
            conv_id: self.conv_id.clone(),
            text: text.to_string(),
        });
    }

    fn on_tool_call_started(&self, tool_name: &str) {
        // `tool_name` is the provider's one-line summary. Reserve the call's
        // history slot now; the record is written when the result arrives,
        // or as summary-only when the turn ends without one.
        self.history.tool_started(&self.turn_id, tool_name);
        let _ = self.tx.send(Event::ToolCallStarted {
            conv_id: self.conv_id.clone(),
            tool_name: tool_name.to_string(),
        });
    }

    fn on_tool_call_completed(&self, outcome: &ToolCallOutcome<'_>) {
        let output = match outcome.output {
            Some(ToolOutputOutcome::Full { content, is_error }) => {
                gaviero_core::history::ToolOutput::Full {
                    content: content.to_string(),
                    is_error,
                }
            }
            Some(ToolOutputOutcome::Summary(text)) => gaviero_core::history::ToolOutput::Summary {
                text: text.to_string(),
            },
            None => gaviero_core::history::ToolOutput::None,
        };
        self.history.tool_completed(
            &self.turn_id,
            gaviero_core::history::tool_call_record(
                outcome.name,
                outcome.tool_use_id.map(str::to_string),
                outcome
                    .duration
                    .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
                outcome.input.cloned(),
                output,
                outcome.summary.map(str::to_string),
            ),
        );
    }

    fn on_streaming_status(&self, status: &str) {
        let _ = self.tx.send(Event::StreamingStatus {
            conv_id: self.conv_id.clone(),
            status: status.to_string(),
        });
    }

    fn on_background_task_started(&self, task_id: &str, description: &str) {
        let _ = self.tx.send(Event::BackgroundTaskStarted {
            conv_id: self.conv_id.clone(),
            task_id: task_id.to_string(),
            description: description.to_string(),
        });
    }

    fn on_background_task_finished(&self, task_id: &str, status: &str, summary: &str) {
        let _ = self.tx.send(Event::BackgroundTaskFinished {
            conv_id: self.conv_id.clone(),
            task_id: task_id.to_string(),
            status: status.to_string(),
            summary: summary.to_string(),
        });
    }

    fn on_context_window(&self, tokens: u64) {
        let _ = self.tx.send(Event::ContextWindow {
            conv_id: self.conv_id.clone(),
            tokens,
        });
    }

    fn on_context_compacted(
        &self,
        trigger: &str,
        pre_tokens: Option<u64>,
        post_tokens: Option<u64>,
    ) {
        let _ = self.tx.send(Event::ContextCompacted {
            conv_id: self.conv_id.clone(),
            trigger: trigger.to_string(),
            pre_tokens,
            post_tokens,
        });
    }

    fn on_permission_request(
        &self,
        tool_name: &str,
        description: &str,
        input: &serde_json::Value,
        respond: tokio::sync::oneshot::Sender<gaviero_core::observer::PermissionDecision>,
    ) {
        let _ = self.tx.send(Event::PermissionRequest {
            conv_id: self.conv_id.clone(),
            tool_name: tool_name.to_string(),
            description: description.to_string(),
            input: input.clone(),
            respond,
        });
    }

    fn on_message_complete(&self, role: &str, content: &str) {
        if role == "assistant" {
            self.history.note_assistant_output(&self.turn_id, content);
        }
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
        self.history.note_usage(
            &self.turn_id,
            gaviero_core::history::ProviderUsage {
                input_tokens: usage.input_tokens,
                cache_creation_input_tokens: usage.cache_creation_input_tokens,
                cache_read_input_tokens: usage.cache_read_input_tokens,
                output_tokens: usage.output_tokens,
            },
        );
        let _ = self.tx.send(Event::TurnTokenUsage {
            conv_id: self.conv_id.clone(),
            usage: usage.clone(),
        });
    }

    fn on_turn_cost_usd(&self, cost: f64) {
        let _ = self.tx.send(Event::TurnCostUpdate {
            conv_id: self.conv_id.clone(),
            cost_usd: cost,
        });
    }

    fn on_tool_agent_edits(&self, paths: &[std::path::PathBuf]) {
        let _ = self.tx.send(Event::ToolAgentEditsPending {
            paths: paths.to_vec(),
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
    fn on_write_enqueued(&self, _kind: &str) {
        let _ = self.tx.send(Event::MemoryWriteEnqueued);
    }
    fn on_write_committed(&self, _kind: &str, _result: &gaviero_core::memory::WriteResult) {
        let _ = self.tx.send(Event::MemoryWriteCommitted);
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
    fn on_manifest_persisted(&self, turn_id: &str, _session_id: &str) {
        let _ = self.tx.send(Event::MemoryManifestPersisted {
            turn_id: turn_id.to_string(),
        });
    }
}

/// A5: forwards read-only MCP tool calls into the TUI event loop, and
/// records each call's verbatim request and response in the history log.
/// The history write is direct (not via the event loop), so the status-bar
/// event keeps its small shape.
pub(super) struct TuiMcpObserver {
    pub tx: mpsc::UnboundedSender<Event>,
    pub history: Arc<HistoryRecorder>,
}

impl gaviero_core::mcp::McpToolCallObserver for TuiMcpObserver {
    fn on_tool_call(&self, entry: &gaviero_core::mcp::McpCallLogEntry) {
        self.history.record_mcp_call(entry);
        let _ = self.tx.send(Event::McpToolCall {
            tool_name: entry.tool_name.clone(),
            duration_ms: entry.duration.as_millis() as u64,
            error: entry.error.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::history::{Attribution, HistoryKind, ToolOutput, TurnStart};
    use gaviero_core::mcp::McpToolCallObserver;

    fn recorder() -> (tempfile::TempDir, Arc<HistoryRecorder>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        (dir, HistoryRecorder::with_path_and_cap(path, 1 << 20))
    }

    fn start() -> TurnStart {
        TurnStart {
            provider: "claude".into(),
            model: "claude:sonnet".into(),
            conv_title: None,
            workspace_root: "C:/w".into(),
            prompt: "p".into(),
            prompt_bytes: 0,
            prompt_truncated: false,
            input_tokens_est: None,
            estimator: None,
        }
    }

    #[test]
    fn mcp_observer_records_verbatim_io_and_still_emits_the_status_event() {
        let (_dir, history) = recorder();
        history.begin_turn("c1", "c1-1", start(), false);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer = TuiMcpObserver {
            tx,
            history: history.clone(),
        };
        let entry = gaviero_core::mcp::McpCallLogEntry {
            tool_name: "memory_search".into(),
            input: serde_json::json!({"query": "estimator", "limit": 5}),
            output: serde_json::json!({"results": [{"id": 41, "text": "words×1.3"}]}),
            duration: std::time::Duration::from_millis(4),
            error: None,
            first_tool_call_initiated: true,
            session_id: None,
            turn: None,
        };
        observer.on_tool_call(&entry);

        assert!(matches!(rx.try_recv(), Ok(Event::McpToolCall { .. })));
        let out = gaviero_core::history::read_records(history.path(), false);
        let mcp = out
            .records
            .iter()
            .find_map(|e| match &e.record.payload {
                HistoryKind::McpCall(m) => Some((e.record.turn_id.clone(), m.clone())),
                _ => None,
            })
            .expect("an mcp_call record");
        assert_eq!(mcp.0.as_deref(), Some("c1-1"));
        assert_eq!(mcp.1.input, entry.input);
        assert_eq!(mcp.1.output, entry.output);
        assert_eq!(mcp.1.attribution, Attribution::TurnInferred);
    }

    #[test]
    fn acp_observer_pairs_tool_start_and_completion_into_one_record() {
        let (_dir, history) = recorder();
        history.begin_turn("c1", "c1-1", start(), false);
        let (tx, _rx) = mpsc::unbounded_channel();
        let observer = TuiAcpObserver {
            tx,
            conv_id: "c1".into(),
            turn_id: "c1-1".into(),
            history: history.clone(),
        };
        let input = serde_json::json!({"command": "ls"});
        observer.on_tool_call_started("Bash: ls");
        observer.on_tool_call_completed(&ToolCallOutcome {
            name: "Bash",
            tool_use_id: Some("toolu_9"),
            summary: Some("Bash: ls"),
            input: Some(&input),
            output: Some(ToolOutputOutcome::Full {
                content: "Cargo.toml",
                is_error: false,
            }),
            duration: Some(std::time::Duration::from_millis(12)),
        });
        observer.on_message_complete("assistant", "done");
        history.end_turn("c1-1", gaviero_core::history::TurnEnd::new(false, None, 0));

        let turns = gaviero_core::history::group_turns(
            gaviero_core::history::read_records(history.path(), false).records,
        );
        let calls: Vec<_> = turns[0]
            .records
            .iter()
            .filter_map(|e| match &e.record.payload {
                HistoryKind::ToolCall(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1, "start + completion must not duplicate");
        assert_eq!(calls[0].tool_use_id.as_deref(), Some("toolu_9"));
        assert_eq!(calls[0].duration_ms, Some(12));
        assert_eq!(calls[0].input.as_ref(), Some(&input));
        assert_eq!(
            calls[0].output,
            ToolOutput::Full {
                content: "Cargo.toml".into(),
                is_error: false
            }
        );
        assert_eq!(turns[0].summary.out_tokens_est, 1);
    }
}
