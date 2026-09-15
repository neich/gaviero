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

    /// Drain the accumulated calls, parsing each arguments string into JSON via
    /// [`parse_tool_arguments`]. Empty arguments parse to `{}`; a genuine parse
    /// failure yields an `Err(message)` so the caller can surface it as an
    /// [`ApiEvent::Error`]. Draining leaves the accumulator empty so a second
    /// flush is a harmless no-op.
    fn drain(&mut self) -> Vec<std::result::Result<ToolCall, String>> {
        std::mem::take(&mut self.calls)
            .into_values()
            .map(|p| {
                let args = parse_tool_arguments(&p.name, &p.args)?;
                Ok(ToolCall {
                    id: p.id,
                    name: p.name,
                    args,
                })
            })
            .collect()
    }
}

/// Parse a tool call's `arguments` string into JSON, tolerating the sloppy
/// serialization OpenAI-compatible models routinely emit.
///
/// `serde_json` is strict about RFC 8259: a raw control character (0x00–0x1F)
/// inside a string literal is a hard error. But models emit multi-line tool
/// arguments — most often `MultiEdit` with literal newlines (and tab-indented
/// Rust) in `old_string`/`new_string` — with the control characters *unescaped*,
/// which failed the entire turn with "malformed arguments JSON". On a strict
/// parse failure we retry once with in-string control characters escaped in
/// place; the decoded string is byte-identical to what the model meant.
///
/// Returns `Err(message)` only when the arguments are unparseable even after
/// that repair, so real model mistakes still surface as [`ApiEvent::Error`].
fn parse_tool_arguments(name: &str, raw: &str) -> std::result::Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let strict_err = match serde_json::from_str(trimmed) {
        Ok(value) => return Ok(value),
        Err(e) => e,
    };
    let repaired = escape_control_chars_in_strings(trimmed);
    if repaired != trimmed
        && let Ok(value) = serde_json::from_str(&repaired)
    {
        return Ok(value);
    }
    Err(format!("tool '{name}' has malformed arguments JSON: {strict_err}"))
}

/// Escape raw control characters that sit *inside* JSON string literals.
///
/// Text outside a string is copied verbatim (there a newline or tab is legal
/// JSON whitespace, and anything else is a different failure that escaping
/// cannot fix). Already-escaped sequences such as `\n` or `\"` are preserved,
/// so a correctly serialized payload is returned unchanged.
fn escape_control_chars_in_strings(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;
    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }
        if escaped {
            escaped = false;
            out.push(ch);
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                out.push(ch);
            }
            '"' => {
                in_string = false;
                out.push(ch);
            }
            c if (c as u32) < 0x20 => match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                other => out.push_str(&format!("\\u{:04x}", other as u32)),
            },
            c => out.push(c),
        }
    }
    out
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
                let _ = tx
                    .send(Ok(ApiEvent::Usage(usage_from(usage, pricing))))
                    .await;
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
    async fn tolerates_raw_newlines_in_multiedit_arguments() {
        // Sloppy serialization seen in the wild: `arguments` carries *literal*
        // newlines inside `new_string` (raw 0x0A, not the escaped `\\n`),
        // which strict `serde_json` rejects. Before the repair fallback this
        // killed the whole turn with "tool 'MultiEdit' has malformed arguments
        // JSON: control character (\u0000-\u001F) found while parsing a string".
        let body = [
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_me","type":"function","function":{"name":"MultiEdit","arguments":"{\"file_path\":\"src/x.rs\",\"edits\":[{\"old_string\":\"a\",\"new_string\":\"line1\nline2\"}]}"}}]},"finish_reason":null}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            "",
            "data: [DONE]",
        ]
        .join("\n");

        let events = run_sse(body).await;
        assert!(
            !events.iter().any(|e| matches!(e, ApiEvent::Error(_))),
            "raw newlines in arguments must be repaired, got {events:?}"
        );
        let call = events
            .iter()
            .find_map(|e| match e {
                ApiEvent::ToolCall(c) => Some(c),
                _ => None,
            })
            .expect("expected a repaired MultiEdit tool call");
        assert_eq!(call.id, "call_me");
        assert_eq!(call.name, "MultiEdit");
        assert_eq!(call.args["file_path"], "src/x.rs");
        // The decoded string is exactly what the model meant: one real newline.
        assert_eq!(call.args["edits"][0]["new_string"], "line1\nline2");
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

    #[test]
    fn empty_arguments_parse_to_empty_object() {
        assert_eq!(parse_tool_arguments("Read", "").unwrap(), json!({}));
        assert_eq!(parse_tool_arguments("Read", "  \n").unwrap(), json!({}));
    }

    #[test]
    fn unrepairable_arguments_still_error() {
        let err = parse_tool_arguments("Read", "{not json").unwrap_err();
        assert!(err.contains("malformed arguments"), "{err}");
        // A raw newline cannot rescue text that is not a JSON document.
        assert!(parse_tool_arguments("Read", "{no\tjson\n").is_err());
    }

    #[test]
    fn clean_arguments_pass_through_untouched() {
        let raw = r#"{"file_path":"src/x.rs","edits":[{"old_string":"a\nb","new_string":"c"}]}"#;
        assert_eq!(escape_control_chars_in_strings(raw), raw);
        assert_eq!(
            parse_tool_arguments("MultiEdit", raw).unwrap()["edits"][0]["old_string"],
            "a\nb"
        );
    }

    #[test]
    fn escapes_control_chars_inside_strings_only() {
        // Raw tab/newline inside a string get escaped; JSON whitespace *outside*
        // strings is left alone; a pre-escaped `\\n` is not double-escaped.
        let raw = "{\"a\": \"x\ty\",\n \"b\": \"p\nq\", \"c\": \"keep\\nthis\"}";
        let repaired = escape_control_chars_in_strings(raw);
        assert!(repaired.contains("\\t") && repaired.contains("\\n"));
        let value: Value = serde_json::from_str(&repaired).expect("repaired parses");
        assert_eq!(value["a"], "x\ty");
        assert_eq!(value["b"], "p\nq");
        assert_eq!(value["c"], "keep\nthis");
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
