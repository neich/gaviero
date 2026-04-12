//! Claude Code subprocess backend.
//!
//! Implements [`AgentBackend`] by spawning a `claude` subprocess and mapping
//! its NDJSON [`StreamEvent`](crate::acp::protocol::StreamEvent) output
//! into the unified [`UnifiedStreamEvent`] stream.

use std::pin::Pin;

use anyhow::{Context, Result};
use futures::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::{StreamEvent, find_next_file_block, parse_file_blocks};
use crate::acp::session::{AcpSession, AgentOptions};

use super::shared::request_prompt;
use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
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
            display_name: format!("claude-code:{}", model),
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
        let file_attachments: Vec<&std::path::Path> =
            request.file_attachments.iter().map(|p| p.as_path()).collect();

        let options = AgentOptions {
            effort: request.effort.unwrap_or_else(|| "off".to_string()),
            max_tokens: request.max_tokens.unwrap_or(16384),
            auto_approve: request.auto_approve,
        };

        let session = AcpSession::spawn(
            &self.model,
            &request.workspace_root,
            &prompt,
            system_prompt,
            &allowed_tools,
            &allowed_tools,
            &options,
            &file_attachments,
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
            supports_file_blocks: true,
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
async fn drive_session(
    mut session: AcpSession,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<()> {
    let mut full_text = String::new();
    let mut file_scan_pos: usize = 0;

    loop {
        match session.next_event().await {
            Ok(Some(event)) => {
                let unified = map_acp_event(&event, &mut full_text, &mut file_scan_pos);
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
                // Process exited without ResultEvent
                if !full_text.is_empty() {
                    // Emit any remaining file blocks
                    let remaining = parse_file_blocks(&full_text[file_scan_pos..]);
                    for (path, content) in remaining {
                        let _ = tx
                            .send(Ok(UnifiedStreamEvent::FileBlock { path, content }))
                            .await;
                    }
                }
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

    // Drain remaining file blocks from accumulated text
    let remaining = parse_file_blocks(&full_text[file_scan_pos..]);
    for (path, content) in remaining {
        let _ = tx
            .send(Ok(UnifiedStreamEvent::FileBlock { path, content }))
            .await;
    }

    let _ = session.wait().await;
    Ok(())
}

/// Map a single ACP protocol event into zero or more unified events.
///
/// This is a pure function (aside from accumulating `full_text`) for testability.
pub fn map_acp_event(
    event: &StreamEvent,
    full_text: &mut String,
    file_scan_pos: &mut usize,
) -> Vec<UnifiedStreamEvent> {
    let mut out = Vec::new();

    match event {
        StreamEvent::ContentDelta(text) => {
            full_text.push_str(text);
            out.push(UnifiedStreamEvent::TextDelta(text.clone()));

            // Detect complete <file> blocks incrementally
            while let Some((path, content, end)) = find_next_file_block(full_text, *file_scan_pos) {
                *file_scan_pos = end;
                out.push(UnifiedStreamEvent::FileBlock { path, content });
            }
        }
        StreamEvent::ToolUseStart {
            tool_name,
            tool_use_id,
        } => {
            out.push(UnifiedStreamEvent::ToolCallStart {
                id: tool_use_id.clone(),
                name: tool_name.clone(),
            });
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
        StreamEvent::AssistantMessage { text, tool_uses } => {
            // If we didn't get content deltas, use the complete text
            if full_text.is_empty() && !text.is_empty() {
                *full_text = text.clone();
                // Don't emit TextDelta here — the runner already got the deltas
                // or will use full_text at completion.
            }
            // Emit ToolCallEnd for each tool use (the full input is available)
            for tu in tool_uses {
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
        } => {
            if *is_error {
                out.push(UnifiedStreamEvent::Error(result_text.clone()));
                out.push(UnifiedStreamEvent::Done(StopReason::Error));
            } else {
                out.push(UnifiedStreamEvent::Usage(TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
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
    use std::path::PathBuf;

    // Test 6: ACP → Unified event mapping (all variants)
    #[test]
    fn test_map_content_delta() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::ContentDelta("hello".into()),
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], UnifiedStreamEvent::TextDelta("hello".into()));
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_map_tool_use_start() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::ToolUseStart {
                tool_name: "Read".into(),
                tool_use_id: "t1".into(),
            },
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            UnifiedStreamEvent::ToolCallStart {
                id: "t1".into(),
                name: "Read".into(),
            }
        );
    }

    #[test]
    fn test_map_tool_input_delta() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::ToolInputDelta(r#"{"file_path":"#.into()),
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], UnifiedStreamEvent::ToolCallDelta { args_chunk, .. } if args_chunk == r#"{"file_path":"#));
    }

    #[test]
    fn test_map_result_success() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::ResultEvent {
                is_error: false,
                result_text: "ok".into(),
                duration_ms: Some(1500),
                cost_usd: Some(0.02),
            },
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], UnifiedStreamEvent::Usage(u) if u.cost_usd == Some(0.02) && u.duration_ms == Some(1500)));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    #[test]
    fn test_map_result_error() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::ResultEvent {
                is_error: true,
                result_text: "rate limit".into(),
                duration_ms: None,
                cost_usd: None,
            },
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], UnifiedStreamEvent::Error("rate limit".into()));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::Error));
    }

    #[test]
    fn test_map_system_init_filtered() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::SystemInit {
                session_id: "s1".into(),
                model: "sonnet".into(),
            },
            &mut text,
            &mut pos,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn test_map_unknown_filtered() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::Unknown(serde_json::json!({"type": "foo"})),
            &mut text,
            &mut pos,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn test_map_assistant_message_emits_tool_end() {
        let mut text = String::new();
        let mut pos = 0;
        let events = map_acp_event(
            &StreamEvent::AssistantMessage {
                text: "done".into(),
                tool_uses: vec![
                    ToolUseInfo {
                        name: "Read".into(),
                        input: serde_json::json!({"file_path": "src/lib.rs"}),
                    },
                ],
            },
            &mut text,
            &mut pos,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            UnifiedStreamEvent::ToolCallEnd { id: "Read".into() }
        );
    }

    // Test 7: File block detection in text stream
    #[test]
    fn test_file_block_detection_in_stream() {
        let mut text = String::new();
        let mut pos = 0;

        // First chunk: partial text with file block
        let chunk = "Here:\n<file path=\"src/main.rs\">\nfn main() {}\n</file>\nDone.";
        let events = map_acp_event(
            &StreamEvent::ContentDelta(chunk.into()),
            &mut text,
            &mut pos,
        );

        // Should have TextDelta + FileBlock
        assert!(events.len() >= 2);
        assert_eq!(events[0], UnifiedStreamEvent::TextDelta(chunk.into()));
        assert!(matches!(
            &events[1],
            UnifiedStreamEvent::FileBlock { path, content }
                if path == &PathBuf::from("src/main.rs") && content.contains("fn main()")
        ));
    }

    // Test 8: Partial file block across chunks
    #[test]
    fn test_partial_file_block_across_chunks() {
        let mut text = String::new();
        let mut pos = 0;

        // First chunk: opening tag only
        let events1 = map_acp_event(
            &StreamEvent::ContentDelta("<file path=\"src/lib.rs\">\nmod".into()),
            &mut text,
            &mut pos,
        );
        // No FileBlock yet — block is incomplete
        assert_eq!(events1.len(), 1); // just TextDelta
        assert!(matches!(&events1[0], UnifiedStreamEvent::TextDelta(_)));

        // Second chunk: closes the block
        let events2 = map_acp_event(
            &StreamEvent::ContentDelta(" tests;\n</file>".into()),
            &mut text,
            &mut pos,
        );
        // Now we should get TextDelta + FileBlock
        assert!(events2.len() >= 2);
        assert!(events2.iter().any(|e| matches!(
            e,
            UnifiedStreamEvent::FileBlock { path, content }
                if path == &PathBuf::from("src/lib.rs") && content.contains("mod tests;")
        )));
    }
}
