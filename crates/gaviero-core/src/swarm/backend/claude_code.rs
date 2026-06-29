//! Claude Code subprocess backend.
//!
//! Implements [`AgentBackend`] by spawning a `claude` subprocess and mapping
//! its NDJSON [`StreamEvent`](crate::acp::protocol::StreamEvent) output
//! into the unified [`UnifiedStreamEvent`] stream.

use std::pin::Pin;

use anyhow::{Context, Result};
use futures::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::StreamEvent;
use crate::acp::session::{AcpSession, AgentOptions};

use super::shared::request_prompt;
use super::{
    AgentBackend, Capabilities, CompletionRequest, RetrievalToolset, StopReason, TokenUsage,
    UnifiedStreamEvent,
};

/// Backend that spawns Claude Code as a subprocess.
pub struct ClaudeCodeBackend {
    model: String,
    display_name: String,
}

impl ClaudeCodeBackend {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            display_name: format!("claude:{}", model),
        }
    }
}

#[async_trait::async_trait]
impl AgentBackend for ClaudeCodeBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let system_prompt = request.system_prompt.as_deref().unwrap_or("");
        let prompt = request_prompt(&request);
        let allowed_tools: Vec<&str> = request.allowed_tools.iter().map(|s| s.as_str()).collect();
        let file_attachments: Vec<&std::path::Path> = request
            .file_attachments
            .iter()
            .map(|p| p.as_path())
            .collect();
        let additional_roots: Vec<&std::path::Path> = request
            .additional_roots
            .iter()
            .map(|p| p.as_path())
            .collect();

        // M6: `resume_session_id` deprecated; swarm work units are one-shot
        // (no resume), so this is always None. Allow stays until M10.
        // Tool surface defaults to the legacy hardcoded set — swarm
        // backends pre-date the workspace-driven config and don't have
        // a workspace handle here. Override via the `agent.*Tools`
        // settings is plumbed for chat sessions (see TUI side_panel).
        #[allow(deprecated)]
        let options = AgentOptions {
            effort: request.effort.unwrap_or_else(|| "off".to_string()),
            max_tokens: request.max_tokens.unwrap_or(16384),
            auto_approve: request.auto_approve,
            suppress_hooks: request.suppress_hooks,
            available_tools: None,
            approved_tools: None,
            resume_session_id: None,
            ..AgentOptions::default()
        };

        // Claude Code doesn't yet consume `extra { ... }` keys. Log them so
        // users see their DSL knobs aren't being honoured rather than wondering
        // silently. Future milestones can promote specific keys (e.g.
        // `thinking_budget`) into `AgentOptions`.
        for (k, v) in &request.extra {
            tracing::debug!(
                target: "backend.claude",
                key = %k,
                value = %v,
                "ignoring extra key (claude backend does not consume this yet)"
            );
        }

        let session = AcpSession::spawn(
            &self.model,
            &request.workspace_root,
            &prompt,
            system_prompt,
            &allowed_tools,
            &allowed_tools,
            &options,
            &file_attachments,
            &additional_roots,
        )?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<UnifiedStreamEvent>>(64);

        // Spawn a task that reads NDJSON events and maps them to unified events.
        tokio::spawn(async move {
            let result = drive_session(session, tx.clone()).await;
            if let Err(e) = result {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Error(format!("{:#}", e))))
                    .await;
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                    .await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: true,
            extended_thinking: true,
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            // Claude proposes file edits via native Write/Edit/MultiEdit
            // tool calls — never via in-band <file> markup. Setting this
            // false suppresses the file-block instruction in the system
            // prompt (see swarm::backend::shared::default_editor_system_prompt).
            supports_file_blocks: false,
            // PUSH→PULL Phase 1: the host wires the gaviero MCP server for
            // Claude (.mcp.json via config_synth), so the always-on retrieval
            // tools are live. Symbols stay off (enrichment sidecar off by default).
            retrieval: RetrievalToolset {
                graph_and_memory: true,
                symbols: false,
            },
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let output = tokio::process::Command::new("claude")
            .arg("--version")
            .output()
            .await
            .context("claude binary not found")?;
        if output.status.success() {
            Ok(())
        } else {
            anyhow::bail!("claude --version exited with {}", output.status)
        }
    }
}

/// Drive the ACP session to completion, sending unified events through the channel.
///
/// Claude proposals flow through native tool calls (`Write`/`Edit`/`MultiEdit`),
/// not through in-band file-block markup. The text stream is forwarded as
/// `TextDelta` only; no in-band parser runs here.
async fn drive_session(
    mut session: AcpSession,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<()> {
    loop {
        match session.next_event().await {
            Ok(Some(event)) => {
                let unified = map_acp_event(&event);
                for ev in unified {
                    if tx.send(Ok(ev)).await.is_err() {
                        return Ok(()); // receiver dropped
                    }
                }

                // ResultEvent signals end of stream
                if matches!(event, StreamEvent::ResultEvent { .. }) {
                    break;
                }
            }
            Ok(None) => {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)))
                    .await;
                break;
            }
            Err(e) => {
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Error(format!("{:#}", e))))
                    .await;
                let _ = tx
                    .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                    .await;
                break;
            }
        }
    }

    let _ = session.wait().await;
    Ok(())
}

/// Map a single ACP protocol event into zero or more unified events.
///
/// Pure function for testability. Claude file edits flow exclusively through
/// native tool calls (`Write`/`Edit`/`MultiEdit`); this mapper never emits
/// `UnifiedStreamEvent::FileBlock` from text content.
pub fn map_acp_event(event: &StreamEvent) -> Vec<UnifiedStreamEvent> {
    let mut out = Vec::new();

    match event {
        StreamEvent::ContentDelta(text) => {
            out.push(UnifiedStreamEvent::TextDelta(text.clone()));
        }
        StreamEvent::ToolUseStart { .. } => {
            // ACP streams `tool_use_start` before the input JSON has finished
            // arriving, so we have no args to attach. Defer the unified
            // `ToolCallStart` to `AssistantMessage`, which carries each
            // tool_use with its full input — the only point where we can emit
            // a rich summary downstream.
        }
        StreamEvent::ToolInputDelta(json) => {
            // We don't have a tool_use_id in scope for ToolInputDelta.
            // The ACP protocol emits these after ToolUseStart, but the delta
            // doesn't carry the id. We emit with empty id — the runner can
            // correlate by order.
            out.push(UnifiedStreamEvent::ToolCallDelta {
                id: String::new(),
                args_chunk: json.clone(),
            });
        }
        StreamEvent::AssistantMessage { tool_uses, .. } => {
            // `tool_uses` carries the full input JSON per call. Emit a single
            // ToolCallStart-with-args per tool plus the matching ToolCallEnd
            // so downstream observers can format a rich summary.
            for tu in tool_uses {
                out.push(UnifiedStreamEvent::ToolCallStart {
                    id: tu.name.clone(),
                    name: tu.name.clone(),
                    args: tu.input.clone(),
                });
                out.push(UnifiedStreamEvent::ToolCallEnd {
                    id: tu.name.clone(),
                });
            }
        }
        StreamEvent::ResultEvent {
            is_error,
            result_text,
            duration_ms,
            cost_usd,
            usage,
        } => {
            if *is_error {
                out.push(UnifiedStreamEvent::Error(result_text.clone()));
                out.push(UnifiedStreamEvent::Done(StopReason::Error));
            } else {
                let (input_tokens, output_tokens) = match usage {
                    Some(u) => (u.prefix_tokens(), u.output_tokens),
                    None => (0, 0),
                };
                out.push(UnifiedStreamEvent::Usage(TokenUsage {
                    input_tokens,
                    output_tokens,
                    cost_usd: *cost_usd,
                    duration_ms: *duration_ms,
                }));
                out.push(UnifiedStreamEvent::Done(StopReason::EndTurn));
            }
        }
        StreamEvent::ThinkingDelta(text) => {
            out.push(UnifiedStreamEvent::ThinkingDelta(text.clone()));
        }
        StreamEvent::SystemInit { .. } => {
            // Filtered out — not meaningful for the unified stream
        }
        StreamEvent::Unknown(_) => {
            // Filtered out — forward-compatibility passthrough
        }
        StreamEvent::PermissionRequest { .. } => {
            // Swarm agents run with auto-approve; permission requests are not expected.
            // If one arrives, it is silently ignored (the subprocess will time out and deny).
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::protocol::ToolUseInfo;

    // Test 6: ACP → Unified event mapping (all variants)
    #[test]
    fn test_map_content_delta() {
        let events = map_acp_event(&StreamEvent::ContentDelta("hello".into()));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], UnifiedStreamEvent::TextDelta("hello".into()));
    }

    #[test]
    fn test_map_tool_use_start_defers_until_assistant_message() {
        // ToolUseStart fires before input JSON has finished arriving, so
        // claude_code.rs holds emission until AssistantMessage where each
        // tool_use carries full input. ToolUseStart itself emits no events.
        let events = map_acp_event(&StreamEvent::ToolUseStart {
            tool_name: "Read".into(),
            tool_use_id: "t1".into(),
        });
        assert!(events.is_empty());
    }

    #[test]
    fn test_map_tool_input_delta() {
        let events = map_acp_event(&StreamEvent::ToolInputDelta(r#"{"file_path":"#.into()));
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], UnifiedStreamEvent::ToolCallDelta { args_chunk, .. } if args_chunk == r#"{"file_path":"#)
        );
    }

    #[test]
    fn test_map_result_success() {
        let events = map_acp_event(&StreamEvent::ResultEvent {
            is_error: false,
            result_text: "ok".into(),
            duration_ms: Some(1500),
            cost_usd: Some(0.02),
            usage: None,
        });
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], UnifiedStreamEvent::Usage(u) if u.cost_usd == Some(0.02) && u.duration_ms == Some(1500))
        );
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    #[test]
    fn test_map_result_success_propagates_usage_tokens() {
        // T1: when Claude reports a `usage` object, the swarm path must
        // surface input/output tokens (input = full prefix size) instead
        // of zero — otherwise downstream cost estimators are blind.
        let events = map_acp_event(&StreamEvent::ResultEvent {
            is_error: false,
            result_text: "ok".into(),
            duration_ms: Some(1500),
            cost_usd: Some(0.02),
            usage: Some(crate::acp::protocol::TokenUsage {
                input_tokens: 100,
                cache_creation_input_tokens: 200,
                cache_read_input_tokens: 5_000,
                output_tokens: 42,
            }),
        });
        match &events[0] {
            UnifiedStreamEvent::Usage(u) => {
                assert_eq!(u.input_tokens, 5_300, "prefix = 100+200+5000");
                assert_eq!(u.output_tokens, 42);
            }
            other => panic!("expected Usage event, got {:?}", other),
        }
    }

    #[test]
    fn test_map_result_error() {
        let events = map_acp_event(&StreamEvent::ResultEvent {
            is_error: true,
            result_text: "rate limit".into(),
            duration_ms: None,
            cost_usd: None,
            usage: None,
        });
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], UnifiedStreamEvent::Error("rate limit".into()));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::Error));
    }

    #[test]
    fn test_map_system_init_filtered() {
        let events = map_acp_event(&StreamEvent::SystemInit {
            session_id: "s1".into(),
            model: "sonnet".into(),
        });
        assert!(events.is_empty());
    }

    #[test]
    fn test_map_unknown_filtered() {
        let events = map_acp_event(&StreamEvent::Unknown(serde_json::json!({"type": "foo"})));
        assert!(events.is_empty());
    }

    #[test]
    fn test_map_assistant_message_emits_tool_end() {
        let input = serde_json::json!({"file_path": "src/lib.rs"});
        let events = map_acp_event(&StreamEvent::AssistantMessage {
            text: "done".into(),
            tool_uses: vec![ToolUseInfo {
                name: "Read".into(),
                input: input.clone(),
            }],
        });
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            UnifiedStreamEvent::ToolCallStart {
                id: "Read".into(),
                name: "Read".into(),
                args: input,
            }
        );
        assert_eq!(
            events[1],
            UnifiedStreamEvent::ToolCallEnd { id: "Read".into() }
        );
    }

    // Regression: in-band file-block markup in the assistant text stream
    // must NOT produce FileBlock events on the Claude path. Claude file
    // edits flow through native tool calls only.
    #[test]
    fn test_text_stream_does_not_emit_file_blocks() {
        let chunk = "Here:\n<example path=\"a.rs\">body</example>\nDone.";
        let events = map_acp_event(&StreamEvent::ContentDelta(chunk.into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], UnifiedStreamEvent::TextDelta(_)));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, UnifiedStreamEvent::FileBlock { .. })),
            "Claude path must never emit FileBlock from text"
        );
    }

    #[test]
    fn test_capabilities_disable_in_band_file_blocks() {
        let backend = ClaudeCodeBackend::new("sonnet");
        assert!(
            !backend.capabilities().supports_file_blocks,
            "Claude must not advertise in-band file-block support"
        );
    }
}
