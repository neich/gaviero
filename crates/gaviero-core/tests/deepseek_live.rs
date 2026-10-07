//! Live probes against the DeepSeek chat API for the `deepseek:` provider.
//!
//! Every test is `#[ignore]`: they spend real (tiny) money and need a key. The
//! unit tests pin the request *shape*; these confirm the API accepts it — the
//! claims `plans/deepseek-parity` could not verify offline.
//!
//! ```text
//! # key from DEEPSEEK_API_KEY, or [deepseek] api_key in <root>/.gaviero/secrets.toml or ~/.gaviero/secrets.toml
//! DEEPSEEK_PROBE_ROOT=/path/to/workspace \
//!   cargo test -p gaviero-core --test deepseek_live -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use futures::StreamExt;
use serde_json::{Value, json};

use gaviero_core::agent_session::tool_agent::client::DeepseekClient;
use gaviero_core::agent_session::tool_agent::config::ApiClientConfig;
use gaviero_core::agent_session::tool_agent::{ApiClient, ApiEvent, ApiRequest};

const MODEL: &str = "deepseek-flash";

fn client() -> DeepseekClient {
    let root = std::env::var_os("DEEPSEEK_PROBE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    DeepseekClient::new(ApiClientConfig::resolve_deepseek(&root, None, None))
}

fn echo_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "echo",
            "description": "Echo the given text back.",
            "parameters": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }
        }
    })
}

/// Send one request and collect `(text, tool_calls, error)`.
async fn run(messages: Vec<Value>, tools: Vec<Value>) -> (String, Vec<(String, String)>, Option<String>) {
    let request = ApiRequest {
        model: MODEL.into(),
        messages,
        tools,
        max_tokens: None,
        reasoning_effort: Some("low".into()),
    };
    let mut stream = match client().complete(request).await {
        Ok(s) => s,
        Err(e) => return (String::new(), vec![], Some(format!("{e:#}"))),
    };
    let (mut text, mut calls, mut error) = (String::new(), Vec::new(), None);
    while let Some(event) = stream.next().await {
        match event {
            Ok(ApiEvent::Text(t)) => text.push_str(&t),
            Ok(ApiEvent::ToolCall(c)) => calls.push((c.id, c.name)),
            Ok(ApiEvent::Error(e)) => error = Some(e),
            Err(e) => error = Some(format!("{e:#}")),
            _ => {}
        }
    }
    (text, calls, error)
}

/// P1 (errata E1): a tools-bearing request whose history holds an earlier
/// *final-answer* assistant turn with `reasoning_content: ""` — the shape
/// cross-turn replay sends — must not be rejected with a 400.
#[tokio::test]
#[ignore = "live DeepSeek API; spends tokens"]
async fn replayed_turn_with_empty_reasoning_content_is_accepted() {
    let messages = vec![
        json!({ "role": "system", "content": "Answer briefly." }),
        json!({ "role": "user", "content": "Say hi." }),
        json!({ "role": "assistant", "content": "Hi.", "reasoning_content": "" }),
        json!({ "role": "user", "content": "Now say bye." }),
    ];
    let (text, _, error) = run(messages, vec![echo_tool()]).await;
    assert!(error.is_none(), "API rejected the replay shape: {error:?}");
    assert!(!text.is_empty());
}

/// P1 control: the same history *without* the field. Documents what the API
/// does today (400 or not) — informational, never fails.
#[tokio::test]
#[ignore = "live DeepSeek API; spends tokens"]
async fn replayed_turn_without_reasoning_content_control() {
    let messages = vec![
        json!({ "role": "system", "content": "Answer briefly." }),
        json!({ "role": "user", "content": "Say hi." }),
        json!({ "role": "assistant", "content": "Hi." }),
        json!({ "role": "user", "content": "Now say bye." }),
    ];
    let (_, _, error) = run(messages, vec![echo_tool()]).await;
    eprintln!("without reasoning_content: {error:?}");
}

/// P1: a tool round whose reasoning was empty, replayed with `""`, followed by
/// the tool result — the in-turn shape `assistant_tool_call_msg` builds.
#[tokio::test]
#[ignore = "live DeepSeek API; spends tokens"]
async fn tool_round_with_empty_reasoning_content_is_accepted() {
    let messages = vec![
        json!({ "role": "user", "content": "Call echo with text 'x', then tell me what it returned." }),
        json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "",
            "tool_calls": [{
                "id": "call_probe",
                "type": "function",
                "function": { "name": "echo", "arguments": "{\"text\":\"x\"}" }
            }]
        }),
        json!({ "role": "tool", "tool_call_id": "call_probe", "content": "x" }),
    ];
    let (text, _, error) = run(messages, vec![echo_tool()]).await;
    assert!(error.is_none(), "API rejected the tool-round shape: {error:?}");
    assert!(!text.is_empty());
}

/// §5.4 / issue #1244: count tool calls that come back as plain text instead
/// of `tool_calls`. Informational — prints the tally for N attempts.
#[tokio::test]
#[ignore = "live DeepSeek API; spends tokens"]
async fn plain_text_tool_call_rate() {
    let attempts = 10;
    let mut plain_text = 0;
    for _ in 0..attempts {
        let messages = vec![json!({
            "role": "user",
            "content": "Use the echo tool to echo 'ping'. Do not answer in prose."
        })];
        let (text, calls, error) = run(messages, vec![echo_tool()]).await;
        assert!(error.is_none(), "{error:?}");
        if calls.is_empty() && text.contains("echo") {
            plain_text += 1;
        }
    }
    eprintln!("{MODEL}: {plain_text}/{attempts} tool calls arrived as plain text");
}

/// P7: one tiny PNG as an `image_url` data URL in the user message.
#[tokio::test]
#[ignore = "live DeepSeek API; spends tokens"]
async fn flash_accepts_a_data_url_image() {
    // 1x1 transparent PNG.
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let messages = vec![json!({
        "role": "user",
        "content": [
            { "type": "text", "text": "How many pixels wide is this image? Answer with a number." },
            { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{PNG_B64}") } }
        ]
    })];
    let (text, _, error) = run(messages, vec![]).await;
    assert!(error.is_none(), "{error:?}");
    eprintln!("vision answer: {text}");
}
