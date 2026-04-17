//! Streaming Ollama backend.
//!
//! Implements [`AgentBackend`] using Ollama's `/api/chat` endpoint with
//! streaming NDJSON. Replaces the old non-streaming `/api/generate` backend.

use std::pin::Pin;

use anyhow::{Context, Result};
use futures::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::parse_file_blocks;

use super::shared::request_prompt;
use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
};

/// Streaming Ollama backend using `/api/chat`.
pub struct OllamaStreamBackend {
    base_url: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl OllamaStreamBackend {
    pub fn new(base_url: &str, model: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        Self {
            display_name: format!("ollama:{}", model),
            base_url: base,
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl AgentBackend for OllamaStreamBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let url = format!("{}/api/chat", self.base_url);
        let prompt = request_prompt(&request);

        let mut messages = Vec::new();

        // System message
        if let Some(ref sys) = request.system_prompt {
            if !sys.is_empty() {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": sys,
                }));
            }
        }

        // User message
        messages.push(serde_json::json!({
            "role": "user",
            "content": prompt,
        }));

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });
        if let Some(max_tokens) = request.max_tokens {
            body["options"] = serde_json::json!({
                "num_predict": max_tokens,
            });
        }

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("sending request to Ollama /api/chat")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {}: {}", status, text);
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<UnifiedStreamEvent>>(64);

        tokio::spawn(async move {
            let result = drive_ollama_stream(response, tx.clone()).await;
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
            tool_use: false,
            streaming: true,
            vision: false,
            extended_thinking: false,
            max_context_tokens: 8192,
            supports_system_prompt: true,
            supports_file_blocks: true,
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Ollama health check failed")?;
        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("Ollama health check returned {}", resp.status())
        }
    }
}

/// Drive the streaming response, parsing NDJSON lines and emitting unified events.
async fn drive_ollama_stream(
    mut response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<()> {
    let mut accumulated_text = String::new();
    let mut line_buf = String::new();

    // Read chunks from the response body and split by newlines.
    // NDJSON lines may be split across chunks.
    while let Some(chunk) = response.chunk().await? {
        let text = String::from_utf8_lossy(&chunk);
        line_buf.push_str(&text);

        // Process complete lines
        while let Some(newline_pos) = line_buf.find('\n') {
            let line: String = line_buf.drain(..=newline_pos).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let events = parse_ollama_chunk(line);
            for event in events {
                if let UnifiedStreamEvent::TextDelta(ref t) = event {
                    accumulated_text.push_str(t);
                }
                if tx.send(Ok(event)).await.is_err() {
                    return Ok(()); // receiver dropped
                }
            }
        }
    }

    // Process any remaining data in the buffer (last line without trailing newline)
    let remaining_line = line_buf.trim();
    if !remaining_line.is_empty() {
        let events = parse_ollama_chunk(remaining_line);
        for event in events {
            if let UnifiedStreamEvent::TextDelta(ref t) = event {
                accumulated_text.push_str(t);
            }
            let _ = tx.send(Ok(event)).await;
        }
    }

    // Post-stream: extract <file> blocks from accumulated text
    let file_blocks = parse_file_blocks(&accumulated_text);
    for (path, content) in file_blocks {
        let _ = tx
            .send(Ok(UnifiedStreamEvent::FileBlock { path, content }))
            .await;
    }

    Ok(())
}

/// Parse a single NDJSON line from Ollama's `/api/chat` streaming response.
pub fn parse_ollama_chunk(line: &str) -> Vec<UnifiedStreamEvent> {
    let mut out = Vec::new();

    let json: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            out.push(UnifiedStreamEvent::Error(format!(
                "Ollama JSON parse error: {}",
                e
            )));
            return out;
        }
    };

    let done = json.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    if !done {
        // Streaming chunk: extract message.content
        if let Some(content) = json
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        {
            if !content.is_empty() {
                out.push(UnifiedStreamEvent::TextDelta(content.to_string()));
            }
        }

        // Thinking content (if model supports it)
        if let Some(thinking) = json
            .get("message")
            .and_then(|m| m.get("thinking"))
            .and_then(|t| t.as_str())
        {
            if !thinking.is_empty() {
                out.push(UnifiedStreamEvent::ThinkingDelta(thinking.to_string()));
            }
        }
    } else {
        // Final chunk: extract usage metrics
        let eval_count = json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let prompt_eval_count = json
            .get("prompt_eval_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total_duration_ns = json
            .get("total_duration")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        out.push(UnifiedStreamEvent::Usage(TokenUsage {
            input_tokens: prompt_eval_count,
            output_tokens: eval_count,
            cost_usd: None, // local model, no cost
            duration_ms: if total_duration_ns > 0 {
                Some(total_duration_ns / 1_000_000)
            } else {
                None
            },
        }));
        out.push(UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::path::PathBuf;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Test 11: Ollama NDJSON parsing (all variants)
    #[test]
    fn test_parse_ollama_content_chunk() {
        let line = r#"{"model":"qwen","created_at":"2025-01-01T00:00:00Z","message":{"role":"assistant","content":"Hello"},"done":false}"#;
        let events = parse_ollama_chunk(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], UnifiedStreamEvent::TextDelta("Hello".into()));
    }

    #[test]
    fn test_parse_ollama_done_chunk() {
        let line = r#"{"model":"qwen","done":true,"total_duration":500000000,"eval_count":50,"prompt_eval_count":10}"#;
        let events = parse_ollama_chunk(line);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            UnifiedStreamEvent::Usage(u) if u.output_tokens == 50 && u.input_tokens == 10 && u.duration_ms == Some(500)
        ));
        assert_eq!(events[1], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    #[test]
    fn test_parse_ollama_malformed_json() {
        let events = parse_ollama_chunk("not json at all");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], UnifiedStreamEvent::Error(_)));
    }

    #[test]
    fn test_parse_ollama_empty_content_skipped() {
        let line = r#"{"model":"qwen","message":{"role":"assistant","content":""},"done":false}"#;
        let events = parse_ollama_chunk(line);
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_ollama_thinking_chunk() {
        let line = r#"{"model":"qwen","message":{"role":"assistant","content":"","thinking":"Let me think..."},"done":false}"#;
        let events = parse_ollama_chunk(line);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            UnifiedStreamEvent::ThinkingDelta("Let me think...".into())
        );
    }

    // Test 12: Ollama capabilities
    #[test]
    fn test_ollama_capabilities() {
        let backend = OllamaStreamBackend::new("http://localhost:11434", "qwen");
        let caps = backend.capabilities();
        assert!(!caps.tool_use);
        assert!(caps.streaming);
        assert!(!caps.vision);
        assert!(caps.supports_system_prompt);
        assert!(caps.supports_file_blocks);
    }

    // Test 13: Mock HTTP streaming response
    #[tokio::test]
    async fn test_ollama_stream_via_mock_server() {
        let mock_server = MockServer::start().await;

        let ndjson_body = [
            r#"{"model":"qwen","message":{"role":"assistant","content":"Hello"},"done":false}"#,
            r#"{"model":"qwen","message":{"role":"assistant","content":" world"},"done":false}"#,
            r#"{"model":"qwen","done":true,"total_duration":100000000,"eval_count":2,"prompt_eval_count":5}"#,
        ]
        .join("\n");

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(ndjson_body)
                    .insert_header("content-type", "application/x-ndjson"),
            )
            .mount(&mock_server)
            .await;

        let backend = OllamaStreamBackend::new(&mock_server.uri(), "qwen");
        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: Some("be helpful".into()),
            workspace_root: PathBuf::from("/tmp"),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            max_tokens: None,
            auto_approve: true,
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let mut events = Vec::new();
        while let Some(ev) = stream.next().await {
            events.push(ev.unwrap());
        }

        assert!(events.len() >= 4); // 2 text + usage + done
        assert_eq!(events[0], UnifiedStreamEvent::TextDelta("Hello".into()));
        assert_eq!(events[1], UnifiedStreamEvent::TextDelta(" world".into()));
        assert!(matches!(&events[2], UnifiedStreamEvent::Usage(u) if u.output_tokens == 2));
        assert_eq!(events[3], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    // Test 14: Mock HTTP server error (500)
    #[tokio::test]
    async fn test_ollama_server_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let backend = OllamaStreamBackend::new(&mock_server.uri(), "qwen");
        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            max_tokens: None,
            auto_approve: true,
        };

        let result = backend.stream_completion(req).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("500"));
    }

    // Test 15: Mock HTTP health check
    #[tokio::test]
    async fn test_ollama_health_check_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": []})),
            )
            .mount(&mock_server)
            .await;

        let backend = OllamaStreamBackend::new(&mock_server.uri(), "qwen");
        assert!(backend.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn test_ollama_health_check_failure() {
        // Use a port that's definitely not running anything
        let backend = OllamaStreamBackend::new("http://127.0.0.1:1", "qwen");
        assert!(backend.health_check().await.is_err());
    }

    // Test 16: File block extraction from streamed Ollama response
    #[tokio::test]
    async fn test_ollama_file_block_extraction() {
        let mock_server = MockServer::start().await;

        let ndjson_body = [
            r#"{"model":"qwen","message":{"role":"assistant","content":"<file path=\"src/lib.rs\">\nmod foo;\n</file>"},"done":false}"#,
            r#"{"model":"qwen","done":true,"total_duration":50000000,"eval_count":5,"prompt_eval_count":3}"#,
        ]
        .join("\n");

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(ndjson_body)
                    .insert_header("content-type", "application/x-ndjson"),
            )
            .mount(&mock_server)
            .await;

        let backend = OllamaStreamBackend::new(&mock_server.uri(), "qwen");
        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            max_tokens: None,
            auto_approve: true,
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let mut events = Vec::new();
        while let Some(ev) = stream.next().await {
            events.push(ev.unwrap());
        }

        // Should contain a FileBlock event
        let file_blocks: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, UnifiedStreamEvent::FileBlock { .. }))
            .collect();
        assert_eq!(file_blocks.len(), 1);
        assert!(matches!(
            file_blocks[0],
            UnifiedStreamEvent::FileBlock { path, content }
                if path == &PathBuf::from("src/lib.rs") && content.contains("mod foo;")
        ));
    }
}
