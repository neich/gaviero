//! Live probe for the `codex exec --json` event stream.
//!
//! Guards the observability contract in `agent_session/mod.rs`: a codex turn
//! must surface activity *while it runs*, not only when the final answer
//! lands. Before the `--json` migration this was structurally impossible —
//! `codex exec` puts its whole progress log on stderr and only the final
//! message on stdout, so the backend (which reads stdout) had nothing to show
//! until the process exited.
//!
//! The unit tests in `swarm/backend/codex.rs` pin the event *mapping* against
//! a captured transcript. This test pins the part they cannot: that the
//! installed codex CLI still speaks that schema. If codex changes its event
//! names, this fails while the unit tests keep passing.
//!
//! ## Running
//!
//! Marked `#[ignore]` because it requires the `codex` CLI on PATH, a working
//! login, and it spends real tokens.
//!
//! ```bash
//! cargo test -p gaviero-core --test codex_json_stream -- --ignored --nocapture
//! ```
//!
//! Override the model with `E2E_CODEX_MODEL` (default `gpt-5.5`).

use std::time::Instant;

use futures::StreamExt;

use gaviero_core::swarm::backend::codex::CodexBackend;
use gaviero_core::swarm::backend::{AgentBackend, CompletionRequest, UnifiedStreamEvent};

fn request(prompt: &str, workspace_root: std::path::PathBuf) -> CompletionRequest {
    CompletionRequest {
        prompt: prompt.to_string(),
        system_prompt: None,
        workspace_root,
        additional_roots: vec![],
        allowed_tools: vec![],
        file_attachments: vec![],
        conversation_history: vec![],
        file_refs: vec![],
        effort: None,
        extra: Vec::new(),
        max_tokens: None,
        auto_approve: false,
        suppress_hooks: true,
        file_scope: gaviero_core::types::FileScope::default(),
    }
}

#[tokio::test]
#[ignore = "requires the codex CLI, a login, and real tokens"]
async fn codex_exec_streams_tool_and_text_events_before_the_turn_ends() {
    let model = std::env::var("E2E_CODEX_MODEL").unwrap_or_else(|_| "gpt-5.5".to_string());
    let backend = CodexBackend::new(&model);
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Forces at least one shell command, so the turn has observable mid-flight
    // activity to report rather than a single final message.
    let prompt = "Run one shell command that lists the files in the current \
                  directory, then reply with the number of entries. Be brief.";

    let start = Instant::now();
    let mut stream = backend
        .stream_completion(request(prompt, workspace_root))
        .await
        .expect("codex backend should spawn");

    let mut first_activity_ms: Option<u128> = None;
    let mut tool_names: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut usage = None;
    let mut errors: Vec<String> = Vec::new();
    let mut done = None;

    while let Some(event) = stream.next().await {
        let event = event.expect("stream item");
        if first_activity_ms.is_none()
            && matches!(
                event,
                UnifiedStreamEvent::TextDelta(_)
                    | UnifiedStreamEvent::ThinkingDelta(_)
                    | UnifiedStreamEvent::ToolCallStart { .. }
            )
        {
            first_activity_ms = Some(start.elapsed().as_millis());
        }
        match event {
            UnifiedStreamEvent::TextDelta(t) => text.push_str(&t),
            UnifiedStreamEvent::ToolCallStart { name, .. } => tool_names.push(name),
            UnifiedStreamEvent::Usage(u) => usage = Some(u),
            UnifiedStreamEvent::Error(e) => errors.push(e),
            UnifiedStreamEvent::Done(reason) => {
                done = Some(reason);
                break;
            }
            _ => {}
        }
    }

    let total_ms = start.elapsed().as_millis();
    eprintln!("first activity: {first_activity_ms:?} ms, total: {total_ms} ms");
    eprintln!("tools: {tool_names:?}");
    eprintln!("usage: {usage:?}");
    eprintln!("text: {text}");

    assert!(errors.is_empty(), "stream reported errors: {errors:?}");
    assert_eq!(
        done,
        Some(gaviero_core::swarm::backend::StopReason::EndTurn)
    );

    // The core regression: activity must be observable, and it must arrive
    // strictly before the turn ends rather than all at once on exit.
    let first = first_activity_ms.expect("no text or tool event was ever emitted");
    assert!(
        first < total_ms,
        "all output arrived at the end (first={first}ms, total={total_ms}ms) — \
         the backend is not streaming"
    );

    // Tool calls are what drive the chat panel's "Using X..." indicator; the
    // pre-`--json` backend could never emit them.
    assert!(
        !tool_names.is_empty(),
        "expected at least one ToolCallStart for the shell command"
    );
    assert!(!text.trim().is_empty(), "expected visible assistant text");

    // Real token counts, not the hardcoded zeros the old backend reported.
    let usage = usage.expect("turn.completed should yield a Usage event");
    assert!(
        usage.input_tokens > 0 && usage.output_tokens > 0,
        "expected non-zero token counts, got {usage:?}"
    );
    assert!(usage.duration_ms.is_some(), "duration should be filled in");
}
