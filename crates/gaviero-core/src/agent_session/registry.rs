//! Transport-session registry (V9 §11 M5).
//!
//! Small façade that picks an [`AgentSession`] implementation for a given
//! [`ProviderProfile`]. In M5 all providers resolve to [`LegacyAgentSession`];
//! later milestones swap entries per provider:
//!
//! * M6 — Claude returns `ClaudeSession`.
//! * M8 — Codex `app-server` returns `CodexSession`.
//! * M9 — Ollama returns `OllamaSession`.
//!
//! Keeping this in its own module lets the chat/swarm callers construct a
//! session by profile without knowing the transport implementation —
//! exactly what V9 §3 architectural principle 9 ("planner-side and
//! transport-side types in separate modules with a named conversion
//! boundary") prescribes.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::{Stream, StreamExt};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::protocol::find_next_file_block;
use crate::acp::session::AgentOptions;
use crate::context_planner::ProviderProfile;
use crate::observer::AcpObserver;
use crate::swarm::backend::{StopReason, UnifiedStreamEvent};
use crate::write_gate::WriteGatePipeline;

use crate::context_planner::ContinuityMode;

use super::claude::ClaudeSession;
use super::codex_app_server::CodexAppServerSession;
use super::codex_exec::CodexExecSession;
use super::cursor::CursorSession;
use super::ollama::OllamaSession;
use super::{AgentSession, Turn};

/// Inputs the shim needs. Named struct (not positional args) so adding a
/// per-session field is additive; new providers drop the ones they don't
/// consume.
pub struct SessionConstruction {
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    pub observer: Box<dyn AcpObserver>,
    pub model: String,
    pub ollama_base_url: Option<String>,
    pub workspace_root: PathBuf,
    /// Sibling workspace folders (workspace-mode multi-folder). The session
    /// forwards each as a `--add-dir` flag to the underlying CLI so the model
    /// can read/write across folders, not only the primary cwd. Empty in
    /// single-folder mode and per-agent swarm worktrees. The primary
    /// `workspace_root` is *not* duplicated here — callers pass only the
    /// sibling folders.
    pub additional_roots: Vec<PathBuf>,
    pub agent_id: String,
    /// Conversation that owns this session. Stamped onto every `WriteProposal`
    /// the session emits so the gate can drain / mode-switch per conversation
    /// when multiple providers run in parallel. `None` for non-conversational
    /// callers (CLI batch ops, internal tooling).
    pub conv_id: Option<String>,
    pub options: AgentOptions,
    pub profile: ProviderProfile,
    /// Cancellation signal owned by the host (e.g. the TUI). When fired, the
    /// session must kill any subprocess, run revert / cleanup paths, and
    /// return — no more tool calls, no more file edits. Defaulted via
    /// [`CancellationToken::new`] when the caller does not need cancel.
    pub cancel_token: CancellationToken,
}

struct NoopAcpObserver;

impl AcpObserver for NoopAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old_content: Option<&str>, _new_content: &str) {}
}

/// Drives a stream-returning session through Gaviero's existing observer and
/// Write Gate surfaces.
///
/// Claude sessions call the observer directly because Claude's native tool
/// stream owns the review flow. Codex app-server returns normalized stream
/// events instead, so the registry wraps it here to keep the chat caller
/// provider-agnostic: reasoning appears in chat, tool starts update status,
/// in-band `<file>` blocks become review proposals, and completed assistant
/// text flows through the same memory sidecar handling as Claude.
struct ObservedStreamSession {
    inner: Box<dyn AgentSession>,
    observer: Arc<dyn AcpObserver>,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    workspace_root: PathBuf,
    agent_id: String,
    conv_id: Option<String>,
    scan_text_file_blocks: bool,
}

impl ObservedStreamSession {
    async fn consume_stream(
        &self,
        mut stream: Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>,
    ) -> Result<()> {
        let mut assistant_text = String::new();
        let mut in_thinking = false;
        let mut file_scan_pos: usize = 0;
        let mut error: Option<String> = None;

        while let Some(event_result) = stream.next().await {
            let event = match event_result {
                Ok(event) => event,
                Err(e) => {
                    error = Some(format!("{e:#}"));
                    break;
                }
            };

            match event {
                UnifiedStreamEvent::TextDelta(text) => {
                    if in_thinking {
                        self.observer.on_stream_chunk("\n</think>\n");
                        in_thinking = false;
                    }

                    self.observer.on_stream_chunk(&text);
                    assistant_text.push_str(&text);

                    if self.scan_text_file_blocks {
                        while let Some((path, content, end)) =
                            find_next_file_block(&assistant_text, file_scan_pos)
                        {
                            file_scan_pos = end;
                            crate::acp::client::propose_write(
                                &self.write_gate,
                                self.observer.as_ref(),
                                &self.workspace_root,
                                &self.agent_id,
                                self.conv_id.as_deref(),
                                &path,
                                &content,
                            )
                            .await?;
                        }
                    }
                }
                UnifiedStreamEvent::ThinkingDelta(text) => {
                    if !in_thinking {
                        self.observer.on_stream_chunk("<think>\n");
                        in_thinking = true;
                    }
                    self.observer.on_stream_chunk(&text);
                }
                UnifiedStreamEvent::ToolCallStart { name, args, .. } => {
                    let summary = crate::acp::client::format_tool_summary(
                        &name,
                        &args,
                        &self.workspace_root,
                    );
                    self.observer.on_tool_call_started(&summary);
                    self.observer
                        .on_streaming_status(&format!("Using {}...", name));
                }
                UnifiedStreamEvent::ToolCallDelta { .. } | UnifiedStreamEvent::ToolCallEnd { .. } => {
                }
                UnifiedStreamEvent::FileBlock { path, content } => {
                    crate::acp::client::propose_write(
                        &self.write_gate,
                        self.observer.as_ref(),
                        &self.workspace_root,
                        &self.agent_id,
                        self.conv_id.as_deref(),
                        &path,
                        &content,
                    )
                    .await?;
                }
                UnifiedStreamEvent::Usage(usage) => {
                    self.observer
                        .on_turn_token_usage(&crate::acp::protocol::TokenUsage {
                            input_tokens: usage.input_tokens,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            output_tokens: usage.output_tokens,
                        });
                }
                UnifiedStreamEvent::Error(msg) => {
                    error = Some(msg);
                }
                UnifiedStreamEvent::Done(reason) => {
                    if matches!(reason, StopReason::Error) && error.is_none() {
                        error = Some("agent turn failed".to_string());
                    }
                    break;
                }
            }
        }

        if in_thinking {
            self.observer.on_stream_chunk("\n</think>\n");
        }

        self.observer
            .on_message_complete("assistant", &assistant_text);

        if let Some(msg) = error {
            anyhow::bail!(msg);
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl AgentSession for ObservedStreamSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let stream = self.inner.send_turn(turn).await?;
        self.consume_stream(stream).await?;
        Ok(Box::pin(futures::stream::empty::<Result<UnifiedStreamEvent>>()))
    }

    fn continuity_mode(&self) -> ContinuityMode {
        self.inner.continuity_mode()
    }

    fn continuity_handle(&self) -> Option<&crate::context_planner::ContinuityHandle> {
        self.inner.continuity_handle()
    }

    async fn close(self: Box<Self>) {
        let ObservedStreamSession { inner, .. } = *self;
        inner.close().await;
    }
}

/// Pick a transport session for the given profile.
///
/// Matches on `ContinuityMode` + `provider` so each provider gets an
/// independent session type with a named deletion target for M10 cleanup.
///
/// M5: introduced; all providers resolved to `LegacyAgentSession`.
/// M6: `NativeResume` (Claude) returns `ClaudeSession`.
/// M8: `ProcessBound` (Codex app-server) returns `CodexAppServerSession`;
///     `StatelessReplay` Codex exec returns `CodexExecSession`.
/// M9: `StatelessReplay` Ollama (and local:) returns `OllamaSession`.
pub fn create_session(args: SessionConstruction) -> Box<dyn AgentSession> {
    match args.profile.continuity_mode {
        ContinuityMode::NativeResume => {
            if args.profile.provider == "cursor" {
                // Cursor's `agent --resume <chat-id>` is the native-resume
                // mechanism; the chat session captures the chat id from the
                // `system.init` event and feeds it back via `options.resume_session_id`.
                Box::new(CursorSession::new(args))
            } else {
                // M6: Claude — per-provider session owns the subprocess lifecycle.
                Box::new(ClaudeSession::new(args))
            }
        }
        ContinuityMode::ProcessBound => {
            // M8: Codex app-server returns normalized stream events. Drive
            // those through the observer + Write Gate here so chat callers
            // get Claude-like streaming, review, and memory behavior without
            // provider-specific code.
            let SessionConstruction {
                write_gate,
                observer,
                model,
                ollama_base_url,
                workspace_root,
                additional_roots,
                agent_id,
                conv_id,
                options,
                profile,
                cancel_token,
            } = args;

            let observer: Arc<dyn AcpObserver> = Arc::from(observer);
            let inner_args = SessionConstruction {
                write_gate: write_gate.clone(),
                observer: Box::new(NoopAcpObserver),
                model,
                ollama_base_url,
                workspace_root: workspace_root.clone(),
                additional_roots,
                agent_id: agent_id.clone(),
                conv_id: conv_id.clone(),
                options,
                profile,
                cancel_token,
            };

            Box::new(ObservedStreamSession {
                inner: Box::new(CodexAppServerSession::new(inner_args)),
                observer,
                write_gate,
                workspace_root,
                agent_id,
                conv_id,
                scan_text_file_blocks: true,
            })
        }
        ContinuityMode::StatelessReplay => {
            if args.profile.provider == "codex" {
                // M8: `codex exec` — named type distinct from Ollama so the
                // registry can route them independently and M10 has separate
                // deletion targets.
                Box::new(CodexExecSession::new(args))
            } else {
                // M9: Ollama (and future StatelessReplay providers) get a
                // bounded session that applies compaction before forwarding
                // to the Ollama backend.
                Box::new(OllamaSession::new(args))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::ContinuityHandle;
    use crate::context_planner::types::PlannerMetadata;
    use crate::swarm::backend::TokenUsage as BackendTokenUsage;
    use crate::types::WriteProposal;
    use crate::write_gate::{WriteGatePipeline, WriteMode};
    use futures::stream;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct RecordingObserver {
        events: StdMutex<Vec<String>>,
    }

    impl RecordingObserver {
        fn snapshot(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AcpObserver for RecordingObserver {
        fn on_stream_chunk(&self, text: &str) {
            self.events.lock().unwrap().push(format!("chunk:{text}"));
        }
        fn on_tool_call_started(&self, name: &str) {
            self.events.lock().unwrap().push(format!("tool:{name}"));
        }
        fn on_streaming_status(&self, status: &str) {
            self.events.lock().unwrap().push(format!("status:{status}"));
        }
        fn on_message_complete(&self, role: &str, content: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("complete:{role}:{content}"));
        }
        fn on_proposal_deferred(&self, path: &Path, _old: Option<&str>, _new: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("deferred:{}", path.display()));
        }
        fn on_turn_token_usage(&self, usage: &crate::acp::protocol::TokenUsage) {
            self.events
                .lock()
                .unwrap()
                .push(format!("usage:{}/{}", usage.input_tokens, usage.output_tokens));
        }
    }

    struct NoopWriteGateObserver;
    impl crate::observer::WriteGateObserver for NoopWriteGateObserver {
        fn on_proposal_created(&self, _proposal: &WriteProposal) {}
        fn on_proposal_updated(&self, _proposal_id: u64) {}
        fn on_proposal_finalized(&self, _path: &str) {}
    }

    struct ScriptedSession {
        events: Vec<Result<UnifiedStreamEvent>>,
    }

    #[async_trait::async_trait]
    impl AgentSession for ScriptedSession {
        async fn send_turn(
            &mut self,
            _turn: Turn,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
            let events = std::mem::take(&mut self.events);
            Ok(Box::pin(stream::iter(events)))
        }

        fn continuity_mode(&self) -> ContinuityMode {
            ContinuityMode::ProcessBound
        }

        fn continuity_handle(&self) -> Option<&ContinuityHandle> {
            None
        }

        async fn close(self: Box<Self>) {}
    }

    fn empty_turn() -> Turn {
        Turn {
            user_message: String::new(),
            memory_selections: vec![],
            graph_selections: vec![],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: None,
            effort: None,
            auto_approve: false,
            metadata: PlannerMetadata {
                memory_count: 0,
                graph_token_estimate: 0,
                graph_budget: 0,
                is_first_turn: false,
                continuity_mode: None,
            },
        }
    }

    fn make_wrapper(
        inner: ScriptedSession,
        recording: Arc<RecordingObserver>,
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        workspace_root: PathBuf,
        scan_text_file_blocks: bool,
    ) -> ObservedStreamSession {
        let observer: Arc<dyn AcpObserver> = recording;
        ObservedStreamSession {
            inner: Box::new(inner),
            observer,
            write_gate,
            workspace_root,
            agent_id: "test-agent".into(),
            conv_id: None,
            scan_text_file_blocks,
        }
    }

    fn make_write_gate(mode: WriteMode) -> Arc<Mutex<WriteGatePipeline>> {
        Arc::new(Mutex::new(WriteGatePipeline::new(
            mode,
            Box::new(NoopWriteGateObserver),
        )))
    }

    #[tokio::test]
    async fn dispatches_stream_events_through_acp_observer_and_fires_message_complete() {
        // Provider parity acceptance: every UnifiedStreamEvent variant that a
        // Claude-equivalent CLI emits must surface through AcpObserver in the
        // same order Claude's native path uses, and on_message_complete must
        // fire at the end — that callback is what controller.rs Event::
        // MessageComplete → enqueue_post_turn keys off for memory extraction.
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta("hi ".into())),
            Ok(UnifiedStreamEvent::ThinkingDelta("plan".into())),
            Ok(UnifiedStreamEvent::ToolCallStart {
                id: "1".into(),
                name: "Bash".into(),
                args: serde_json::json!({ "command": "ls -la" }),
            }),
            Ok(UnifiedStreamEvent::TextDelta("done".into())),
            Ok(UnifiedStreamEvent::Usage(BackendTokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cost_usd: None,
                duration_ms: None,
            })),
            Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)),
        ];
        let recording = Arc::new(RecordingObserver::default());
        let mut session = make_wrapper(
            ScriptedSession { events },
            recording.clone(),
            make_write_gate(WriteMode::AutoAccept),
            std::env::temp_dir(),
            false,
        );

        let _ = session.send_turn(empty_turn()).await.expect("send_turn ok");

        let recorded = recording.snapshot();
        // The wrapper wraps ThinkingDelta in <think>...</think> framing the
        // chat panel renders as a reasoning region. Verify the framing and
        // that the inner delta is between the open and close tags.
        let think_open = recorded.iter().position(|e| e == "chunk:<think>\n");
        let think_inner = recorded.iter().position(|e| e == "chunk:plan");
        let think_close = recorded.iter().position(|e| e == "chunk:\n</think>\n");
        assert!(think_open.is_some(), "missing <think> open: {recorded:?}");
        assert!(think_inner.is_some(), "missing reasoning delta: {recorded:?}");
        assert!(think_close.is_some(), "missing </think> close: {recorded:?}");
        assert!(think_open < think_inner && think_inner < think_close);

        // TextDelta surfaces unchanged.
        assert!(recorded.iter().any(|e| e == "chunk:hi "));
        assert!(recorded.iter().any(|e| e == "chunk:done"));

        // ToolCallStart triggers a rich tool summary + Using... status (drives
        // the streaming indicator in the chat panel header). The summary uses
        // `format_tool_summary` so the Bash command shows the actual argv.
        assert!(
            recorded.iter().any(|e| e == "tool:Bash: ls -la"),
            "expected rich tool summary, recorded={recorded:?}"
        );
        assert!(recorded.iter().any(|e| e == "status:Using Bash..."));

        // Usage event drives the token counter in the status bar.
        assert!(recorded.iter().any(|e| e == "usage:10/5"));

        // Final on_message_complete carries the concatenated visible text
        // (no <think> framing leaks into the message body; reasoning lives
        // only in the observer stream).
        assert!(
            recorded
                .iter()
                .any(|e| e == "complete:assistant:hi done"),
            "expected final message_complete; got {recorded:?}"
        );
    }

    #[tokio::test]
    async fn inline_file_block_in_text_creates_write_gate_proposal() {
        // The in-band <file> parser is the writable channel for providers
        // whose native stream can't carry tool calls (Codex exec / Codex
        // app-server / Ollama). When the wrapper sees a TextDelta whose
        // cumulative text contains a complete <file path="..."> ... </file>
        // block, it must call propose_write so the chat path's normal
        // batch-review flow opens.
        let tmpdir = TempDir::new().expect("tempdir");
        let workspace_root = tmpdir.path().to_path_buf();

        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta(
                "Here is the change:\n<file path=\"foo.txt\">hello world</file>\nDone.".into(),
            )),
            Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)),
        ];
        let recording = Arc::new(RecordingObserver::default());
        let wg = make_write_gate(WriteMode::Interactive);
        let mut session = make_wrapper(
            ScriptedSession { events },
            recording.clone(),
            wg.clone(),
            workspace_root.clone(),
            true,
        );

        let _ = session.send_turn(empty_turn()).await.expect("send_turn ok");

        let gate = wg.lock().await;
        let active = gate.active_proposal_ids();
        assert_eq!(
            active.len(),
            1,
            "expected exactly one proposal for the inline <file> block"
        );
        let proposal = gate
            .get_proposal(active[0])
            .expect("proposal id active but not retrievable");
        assert_eq!(proposal.file_path, workspace_root.join("foo.txt"));
        assert_eq!(proposal.proposed_content, "hello world");
        assert_eq!(proposal.original_content, "");
    }

    #[tokio::test]
    async fn stream_error_surfaces_as_send_turn_error_after_message_complete() {
        // If the provider stream emits Error, the wrapper still calls
        // on_message_complete with whatever text accumulated, then bails out
        // of send_turn with the error. This keeps the chat UI consistent: the
        // partial reply is visible (and gets a memory pass via the message-
        // complete path), and the error is surfaced separately in the chat
        // panel via the caller's side_panel.rs error branch.
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta("partial".into())),
            Ok(UnifiedStreamEvent::Error("backend exploded".into())),
            Ok(UnifiedStreamEvent::Done(StopReason::Error)),
        ];
        let recording = Arc::new(RecordingObserver::default());
        let mut session = make_wrapper(
            ScriptedSession { events },
            recording.clone(),
            make_write_gate(WriteMode::AutoAccept),
            std::env::temp_dir(),
            false,
        );

        // Can't use expect_err — the Ok variant is Pin<Box<dyn Stream>>,
        // which doesn't implement Debug. Match instead.
        let err = match session.send_turn(empty_turn()).await {
            Ok(_) => panic!("expected send_turn to surface the stream error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("backend exploded"),
            "error message should propagate; got: {err}"
        );

        let recorded = recording.snapshot();
        assert!(
            recorded
                .iter()
                .any(|e| e == "complete:assistant:partial"),
            "on_message_complete must fire with partial text before bailing; got: {recorded:?}"
        );
    }
}