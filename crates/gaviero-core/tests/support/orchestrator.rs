//! Single-turn driver wrapping `AcpSession::spawn` with a
//! `RecordingPromptObserver`. T1.5 extends this module with the
//! multi-turn `Step` enum and `run_parallel` driver.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::Barrier;

use gaviero_core::acp::protocol::StreamEvent;
use gaviero_core::acp::session::{AcpSession, AgentOptions};

use super::env::{CapturingObserver, E2eEnv, PER_TURN_TIMEOUT, STREAM_IDLE_TIMEOUT};
use super::prompt_capture::RecordingPromptObserver;

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub session_id: Option<String>,
    pub assistant_text: String,
    pub elapsed: Duration,
}

/// Identifier opaque to the harness; the test owns the namespace.
pub type SessionId = String;
pub type BarrierId = u32;

/// One step in a `ScriptedSession`. The `User` step drives a real
/// Claude turn (with `RecordingPromptObserver` wired). `Reset` clears
/// the simulated history + drops the resume id (the test transport-
/// layer model of `/reset`). `Sleep` is a wall-clock pause.
/// `Barrier` synchronises across sessions in `run_parallel`.
/// `AssertPromptSizeMax` triggers an in-line assertion on the most
/// recent captured prompt.
#[derive(Debug, Clone)]
pub enum Step {
    User(String),
    Reset,
    Sleep(Duration),
    Barrier(BarrierId),
    AssertPromptSizeMax(usize),
}

#[derive(Clone)]
pub struct ScriptedSession {
    pub id: SessionId,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    pub step_index: usize,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub elapsed_ms: u128,
    pub prompt_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct SessionReport {
    pub id: SessionId,
    /// First-turn elapsed across the whole script (used by T1.5 probe 3
    /// for embedder cache fairness).
    pub first_turn_elapsed: Option<Duration>,
    pub records: Vec<StepRecord>,
}

/// Container passed to each session task in `run_parallel`. Bundles
/// the env, the shared prompt recorder, and the barrier map so the
/// per-session runner is `Send + 'static`.
pub struct ParallelContext {
    pub env: Arc<E2eEnv>,
    pub recorder: Arc<RecordingPromptObserver>,
    pub barriers: std::collections::HashMap<BarrierId, Arc<Barrier>>,
    pub model: String,
}

/// Drive every script in `scripts` concurrently against the supplied
/// per-session contexts (one `ParallelContext` per script). Each
/// session runs in its own `tokio::spawn`; `Barrier` steps synchronise
/// across sessions via the shared `barriers` map. Returns one
/// `SessionReport` per input script, in input order.
///
/// Concurrency invariant: scripts share the `RecordingPromptObserver`
/// in their `ParallelContext`, but each script sets its own
/// `current_turn` immediately before spawning a turn — events are
/// keyed by the explicit `turn_id` we thread through, so cross-session
/// interleaving cannot misroute events.
pub async fn run_parallel(
    scripts: Vec<ScriptedSession>,
    contexts: Vec<ParallelContext>,
) -> Result<Vec<SessionReport>> {
    if scripts.len() != contexts.len() {
        anyhow::bail!(
            "run_parallel: {} scripts vs {} contexts",
            scripts.len(),
            contexts.len()
        );
    }
    let mut handles = Vec::with_capacity(scripts.len());
    for (script, ctx) in scripts.into_iter().zip(contexts.into_iter()) {
        handles.push(tokio::spawn(async move { run_script(script, ctx).await }));
    }
    let mut reports = Vec::with_capacity(handles.len());
    for h in handles {
        reports.push(h.await.context("session task join")??);
    }
    Ok(reports)
}

async fn run_script(script: ScriptedSession, ctx: ParallelContext) -> Result<SessionReport> {
    let session_label = script.id.clone();
    let mut report = SessionReport {
        id: session_label.clone(),
        first_turn_elapsed: None,
        records: Vec::new(),
    };
    let mut resume_session_id: Option<String> = None;
    let mut step_idx: usize = 0;
    let mut turn_counter: u32 = 0;

    for step in script.steps.into_iter() {
        match step {
            Step::User(prompt) => {
                turn_counter += 1;
                let turn_id = format!("{}/t{}", session_label, turn_counter);
                let outcome = run_turn(
                    &ctx.env.repo,
                    &ctx.model,
                    &ctx.recorder,
                    &turn_id,
                    &prompt,
                    resume_session_id.clone(),
                )
                .await
                .with_context(|| format!("session {session_label} step {step_idx} (User)"))?;
                if report.first_turn_elapsed.is_none() {
                    report.first_turn_elapsed = Some(outcome.elapsed);
                }
                if resume_session_id.is_none() {
                    resume_session_id = outcome.session_id.clone();
                }
                let prompt_bytes = ctx
                    .recorder
                    .events_for_turn(&turn_id)
                    .last()
                    .map(|ev| ev.prompt.len())
                    .unwrap_or(0);
                report.records.push(StepRecord {
                    step_index: step_idx,
                    turn_id,
                    session_id: outcome.session_id,
                    elapsed_ms: outcome.elapsed.as_millis(),
                    prompt_bytes,
                });
            }
            Step::Reset => {
                resume_session_id = None;
            }
            Step::Sleep(d) => {
                tokio::time::sleep(d).await;
            }
            Step::Barrier(id) => {
                let bar = ctx
                    .barriers
                    .get(&id)
                    .with_context(|| format!("missing barrier {id} for session {session_label}"))?
                    .clone();
                bar.wait().await;
            }
            Step::AssertPromptSizeMax(max_bytes) => {
                let last_ev = ctx
                    .recorder
                    .events()
                    .into_iter()
                    .rev()
                    .find(|e| e.turn_id.starts_with(&format!("{session_label}/")));
                if let Some(ev) = last_ev {
                    if ev.prompt.len() > max_bytes {
                        anyhow::bail!(
                            "session {session_label} step {step_idx}: prompt {} B exceeds max {} B",
                            ev.prompt.len(),
                            max_bytes
                        );
                    }
                }
            }
        }
        step_idx += 1;
    }

    Ok(report)
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
