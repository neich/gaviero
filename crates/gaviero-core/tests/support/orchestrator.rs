//! Single-turn driver wrapping `AcpSession::spawn` with a
//! `RecordingPromptObserver`. T1.5 will extend this module with a
//! multi-turn `Step` enum and `run_parallel` driver.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use gaviero_core::acp::protocol::StreamEvent;
use gaviero_core::acp::session::{AcpSession, AgentOptions};

use super::env::{
    CapturingObserver, PER_TURN_TIMEOUT, STREAM_IDLE_TIMEOUT,
};
use super::prompt_capture::RecordingPromptObserver;

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub session_id: Option<String>,
    pub assistant_text: String,
    pub elapsed: Duration,
}

/// Drive a single Claude turn against `cwd`. Wires the
/// `RecordingPromptObserver` so the captured `PromptEvent` is keyed by
/// `turn_id`. `resume_session_id` mirrors `AgentOptions::resume_session_id`
/// — `None` for cold turns, `Some(id)` for continuations.
///
/// Allows the deprecated `resume_session_id` field for the duration of
/// T1: this is the test transport-layer model of `/reset` (drop resume
/// id + history) and matches `run_one_claude_turn`'s pattern.
#[allow(deprecated)]
pub async fn run_turn(
    cwd: &Path,
    model: &str,
    recorder: &Arc<RecordingPromptObserver>,
    turn_id: &str,
    user_prompt: &str,
    resume_session_id: Option<String>,
) -> Result<TurnOutcome> {
    recorder.set_current_turn(turn_id);

    let observer = CapturingObserver::new();

    let options = AgentOptions {
        effort: "off".to_string(),
        max_tokens: 0,
        auto_approve: true,
        available_tools: Some(vec![]),
        approved_tools: Some(vec![]),
        resume_session_id,
        prompt_observer: Some(recorder.clone() as _),
        turn_id: Some(turn_id.to_string()),
        ..AgentOptions::default()
    };

    let started = Instant::now();
    let mut session = AcpSession::spawn(
        model,
        cwd,
        user_prompt,
        "",
        &[],
        &[],
        &options,
        &[],
    )
    .context("spawning AcpSession")?;

    use gaviero_core::observer::AcpObserver as _;
    let deadline = Instant::now() + PER_TURN_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            session.kill();
            anyhow::bail!("turn timed out after {:?}", PER_TURN_TIMEOUT);
        }
        let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, session.next_event()).await;
        match next {
            Err(_elapsed) => {
                if session.try_wait_exited() {
                    break;
                }
                continue;
            }
            Ok(Ok(Some(event))) => match event {
                StreamEvent::ContentDelta(text) => observer.on_stream_chunk(&text),
                StreamEvent::AssistantMessage { text, .. } => {
                    if !text.is_empty() {
                        observer.on_stream_chunk(&text);
                    }
                }
                StreamEvent::ResultEvent {
                    is_error,
                    result_text,
                    ..
                } => {
                    let role = if is_error { "system" } else { "assistant" };
                    let snap = observer.snapshot();
                    let final_text = if !snap.streamed.is_empty() {
                        snap.streamed
                    } else {
                        result_text
                    };
                    observer.on_message_complete(role, &final_text);
                    break;
                }
                StreamEvent::SystemInit { session_id, .. } => {
                    if !session_id.is_empty() {
                        observer.on_claude_session_started(&session_id);
                    }
                }
                StreamEvent::PermissionRequest { request_id, .. } => {
                    session.respond_permission(false, &request_id);
                }
                StreamEvent::ToolUseStart { tool_name, .. } => {
                    observer.on_tool_call_started(&tool_name);
                }
                _ => {}
            },
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                anyhow::bail!("AcpSession event error: {e}");
            }
        }
    }

    let _ = tokio::time::timeout(Duration::from_secs(10), session.wait()).await;

    let snap = observer.snapshot();
    let assistant_text = snap
        .messages
        .iter()
        .find(|(role, _)| role == "assistant")
        .map(|(_, c)| c.clone())
        .unwrap_or(snap.streamed);

    Ok(TurnOutcome {
        session_id: snap.claude_session_id,
        assistant_text,
        elapsed: started.elapsed(),
    })
}
