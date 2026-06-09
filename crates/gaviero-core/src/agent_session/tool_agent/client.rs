//! DeepSeek chat-completions client.
//!
//! **PR-2 (plan Units 4–5):** real SSE streaming. `delta.content` →
//! [`ApiEvent::Text`], `delta.reasoning_content` → [`ApiEvent::Reasoning`], the
//! final `usage` chunk → [`ApiEvent::Usage`] with cost, and `delta.tool_calls[]`
//! fragments are reassembled by `index` into complete [`ApiEvent::ToolCall`]s.
//! The harness's loop (PR-3) executes those tool calls; this client only
//! produces them.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use futures::Stream;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::swarm::backend::{StopReason, TokenUsage};

use super::config::{ApiClientConfig, PriceTable};
use super::{ApiClient, ApiEvent, ApiRequest, ToolCall};

/// DeepSeek client. Holds the resolved config as a `Result` so a key-resolution
/// failure at construction surfaces on the first `complete` call rather than
/// making session construction fallible.
pub struct DeepseekClient {
    cfg: std::result::Result<ApiClientConfig, String>,
    http: reqwest::Client,
}

impl DeepseekClient {
    pub fn new(cfg: Result<ApiClientConfig>) -> Self {
        Self {
            cfg: cfg.map_err(|e| format!("{e:#}")),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ApiClient for DeepseekClient {
    async fn complete(
        &self,
        request: ApiRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ApiEvent>> + Send>>> {
        let cfg = self.cfg.as_ref().map_err(|e| anyhow!(e.clone()))?;
        let url = format!("{}/chat/completions", cfg.base_url);

        let mut body = json!({
            "model": request.model,
            "messages": request.messages,
            "stream": true,
            // Required for the final SSE chunk to carry token usage.
            "stream_options": { "include_usage": true },
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.clone());
        }
        if let Some(mt) = request.max_tokens {
            body["max_tokens"] = json!(mt);
        }

        let resp = post_with_retry(&self.http, &url, cfg.api_key.expose(), &body).await?;

        let (tx, rx) = mpsc::channel::<Result<ApiEvent>>(64);
        let pricing = cfg.pricing.clone();
        tokio::spawn(async move {
            if let Err(e) = drive_sse_stream(resp, &tx, &pricing).await {
                let _ = tx.send(Ok(ApiEvent::Error(format!("{e:#}")))).await;
                let _ = tx.send(Ok(ApiEvent::Done(StopReason::Error))).await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

const MAX_API_RETRIES: u32 = 4;

/// POST with exponential backoff + jitter on 429 / 5xx (Unit 15).
async fn post_with_retry(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        let resp = http
            .post(url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .context("sending request to DeepSeek /chat/completions")?;

        if resp.status().is_success() {
            return Ok(resp);
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !is_retryable_status(status.as_u16()) || attempt >= MAX_API_RETRIES {
            anyhow::bail!("DeepSeek returned {}: {}", status, text);
        }

        let delay = retry_delay(attempt);
        tracing::warn!(
            target: "tool_agent",
            status = status.as_u16(),
            attempt,
            delay_ms = delay.as_millis(),
            "DeepSeek request retrying after transient error"
        );
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

fn is_retryable_status(code: u16) -> bool {
    code == 429 || (500..600).contains(&code)
}

fn retry_delay(attempt: u32) -> Duration {
    let base_ms = 500u64.saturating_mul(1u64 << attempt.min(6));
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % 250)
        .unwrap_or(0);
    Duration::from_millis(base_ms.saturating_add(jitter))
}

/// Reassembles `tool_calls[]` fragments streamed across SSE deltas. The API
/// sends `id` + `function.name` once and the `function.arguments` JSON string
/// in pieces, keyed by a stable `index`.
#[derive(Default)]
struct ToolCallAccumulator {
    calls: BTreeMap<u64, PartialToolCall>,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args: String,
}

impl ToolCallAccumulator {
    fn ingest(&mut self, fragments: &[Value]) {
        for frag in fragments {
            let idx = frag.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
            let entry = self.calls.entry(idx).or_default();
            if let Some(id) = frag.get("id").and_then(|x| x.as_str())
                && !id.is_empty()
            {
                entry.id = id.to_string();
            }
            if let Some(func) = frag.get("function") {
                if let Some(name) = func.get("name").and_then(|x| x.as_str())
                    && !name.is_empty()
                {
                    entry.name = name.to_string();
                }
                if let Some(args) = func.get("arguments").and_then(|x| x.as_str()) {
                    entry.args.push_str(args);
                }
            }
        }
    }

    /// Drain the accumulated calls, parsing each arguments string into JSON.
    /// Empty arguments parse to `{}`; a parse failure yields an `Err(message)`
    /// so the caller can surface it as an [`ApiEvent::Error`]. Draining leaves
    /// the accumulator empty so a second flush is a harmless no-op.
    fn drain(&mut self) -> Vec<std::result::Result<ToolCall, String>> {
        std::mem::take(&mut self.calls)
            .into_values()
            .map(|p| {
                let args = if p.args.trim().is_empty() {
                    Value::Object(Default::default())
                } else {
                    serde_json::from_str(&p.args).map_err(|e| {
                        format!("tool '{}' has malformed arguments JSON: {e}", p.name)
                    })?
                };
                Ok(ToolCall {
                    id: p.id,
                    name: p.name,
                    args,
                })
            })
            .collect()
    }
}

async fn flush_tool_calls(acc: &mut ToolCallAccumulator, tx: &mpsc::Sender<Result<ApiEvent>>) {
    for result in acc.drain() {
        let event = match result {
            Ok(call) => ApiEvent::ToolCall(call),
            Err(msg) => ApiEvent::Error(msg),
        };
        let _ = tx.send(Ok(event)).await;
    }
}

fn usage_from(usage: &Value, pricing: &PriceTable) -> TokenUsage {
    let g = |k: &str| usage.get(k).and_then(|t| t.as_u64()).unwrap_or(0);
    let prompt = g("prompt_tokens");
    let completion = g("completion_tokens");
    let cache_hit = g("prompt_cache_hit_tokens");
    let cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or_else(|| prompt.saturating_sub(cache_hit));
    TokenUsage {
        input_tokens: prompt,
        output_tokens: completion,
        cost_usd: Some(pricing.cost_usd(cache_hit, cache_miss, completion)),
        duration_ms: None,
    }
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "tool_calls" => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    }
}

/// Drive the SSE body: split into `data:` events, decode deltas, emit normalized
/// [`ApiEvent`]s. Tool-call fragments are buffered and flushed when the choice's
/// `finish_reason` arrives (or at stream end).
async fn drive_sse_stream(
    mut resp: reqwest::Response,
    tx: &mpsc::Sender<Result<ApiEvent>>,
    pricing: &PriceTable,
) -> Result<()> {
    let mut buf = String::new();
    let mut acc = ToolCallAccumulator::default();
    let mut finish = StopReason::EndTurn;

    while let Some(chunk) = resp.chunk().await? {
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline) = buf.find('\n') {
            let line: String = buf.drain(..=newline).collect();
            let line = line.trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue; // blank lines, comments, event: fields
            };
            let data = data.trim();
            if data == "[DONE]" {
                flush_tool_calls(&mut acc, tx).await;
                let _ = tx.send(Ok(ApiEvent::Done(finish))).await;
                return Ok(());
            }

            let value: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx
                        .send(Ok(ApiEvent::Error(format!("SSE JSON parse error: {e}"))))
                        .await;
                    continue;
                }
            };

            // Final chunk (with include_usage) carries usage and empty choices.
            if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
                let _ = tx.send(Ok(ApiEvent::Usage(usage_from(usage, pricing)))).await;
            }

            let Some(choice) = value.pointer("/choices/0") else {
                continue;
            };
            if let Some(delta) = choice.get("delta") {
                if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                    && !content.is_empty()
                {
                    let _ = tx.send(Ok(ApiEvent::Text(content.to_string()))).await;
                }
                if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str())
                    && !reasoning.is_empty()
                {
                    let _ = tx
                        .send(Ok(ApiEvent::Reasoning(reasoning.to_string())))
                        .await;
                }
                if let Some(fragments) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    acc.ingest(fragments);
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                finish = map_finish_reason(reason);
                // Tool-call fragments are complete once finish_reason lands.
                flush_tool_calls(&mut acc, tx).await;
            }
        }
    }

    // Stream ended without an explicit `[DONE]` line.
    flush_tool_calls(&mut acc, tx).await;
    let _ = tx.send(Ok(ApiEvent::Done(finish))).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::config::{ApiClientConfig, ApiKey, PriceTable};
    use super::super::{ApiEvent, ApiRequest, ToolCall};
    use super::*;
    use futures::StreamExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg(base: &str) -> ApiClientConfig {
        ApiClientConfig {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: ApiKey::new("test-key"),
            pricing: PriceTable::default(),
        }
    }

    async fn run_sse(server_body: String) -> Vec<ApiEvent> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(server_body),
            )
            .mount(&server)
            .await;

        let client = DeepseekClient::new(Ok(cfg(&server.uri())));
        let request = ApiRequest {
            model: "deepseek-v4-pro".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: vec![],
            max_tokens: None,
        };
        let mut stream = client.complete(request).await.unwrap();
        let mut events = Vec::new();
        while let Some(e) = stream.next().await {
            events.push(e.unwrap());
        }
        events
    }

    #[tokio::test]
    async fn streams_content_deltas_then_usage_and_done() {
        let body = [
            r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "",
            r#"data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":2,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");

        let events = run_sse(body).await;
        assert!(matches!(&events[0], ApiEvent::Text(t) if t == "Hello"));
        assert!(matches!(&events[1], ApiEvent::Text(t) if t == " world"));
        match &events[2] {
            ApiEvent::Usage(u) => {
                assert_eq!(u.input_tokens, 100);
                assert_eq!(u.output_tokens, 2);
                let expected = (80.0 * 0.07 + 20.0 * 0.56 + 2.0 * 1.68) / 1_000_000.0;
                assert!((u.cost_usd.unwrap() - expected).abs() < 1e-12);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        assert!(matches!(events[3], ApiEvent::Done(StopReason::EndTurn)));
    }

    #[tokio::test]
    async fn streams_reasoning_before_content() {
        let body = [
            r#"data: {"choices":[{"index":0,"delta":{"reasoning_content":"Let me think"},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{"content":"Answer"},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "",
            "data: [DONE]",
        ]
        .join("\n");

        let events = run_sse(body).await;
        assert!(matches!(&events[0], ApiEvent::Reasoning(t) if t == "Let me think"));
        assert!(matches!(&events[1], ApiEvent::Text(t) if t == "Answer"));
        assert!(matches!(events[2], ApiEvent::Done(StopReason::EndTurn)));
    }

    #[tokio::test]
    async fn assembles_tool_call_across_fragments() {
        // `function.arguments` arrives split across two deltas and must be
        // concatenated by `index` before JSON-parsing.
        let body = [
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":"}}]},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/x.rs\"}"}}]},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            "",
            "data: [DONE]",
        ]
        .join("\n");

        let events = run_sse(body).await;
        let calls: Vec<&ToolCall> = events
            .iter()
            .filter_map(|e| match e {
                ApiEvent::ToolCall(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "Read");
        assert_eq!(calls[0].args["file_path"], "src/x.rs");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ApiEvent::Done(StopReason::ToolUse)))
        );
    }

    #[tokio::test]
    async fn malformed_tool_arguments_emit_error() {
        let body = [
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"Read","arguments":"{not json"}}]},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            "",
            "data: [DONE]",
        ]
        .join("\n");

        let events = run_sse(body).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ApiEvent::Error(m) if m.contains("malformed arguments"))),
            "expected a malformed-arguments error, got {events:?}"
        );
    }

    #[tokio::test]
    async fn http_error_surfaces() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let client = DeepseekClient::new(Ok(cfg(&server.uri())));
        let request = ApiRequest {
            model: "x".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: None,
        };
        let result = client.complete(request).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("401"));
    }

    #[tokio::test]
    async fn retries_429_then_streams() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        let ok_body = [
            r#"data: {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(ok_body),
            )
            .mount(&server)
            .await;

        let client = DeepseekClient::new(Ok(cfg(&server.uri())));
        let request = ApiRequest {
            model: "x".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: vec![],
            max_tokens: None,
        };
        let mut stream = client.complete(request).await.unwrap();
        let mut saw_text = false;
        while let Some(e) = stream.next().await {
            if matches!(e.unwrap(), ApiEvent::Text(_)) {
                saw_text = true;
            }
        }
        assert!(saw_text);
    }

    #[tokio::test]
    async fn missing_key_config_errors_on_complete() {
        let client = DeepseekClient::new(Err(anyhow!("no key")));
        let request = ApiRequest {
            model: "x".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: None,
        };
        assert!(client.complete(request).await.is_err());
    }
}
