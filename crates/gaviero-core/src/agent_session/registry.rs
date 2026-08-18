//! Transport-session registry.
//!
//! This façade selects a concrete [`AgentSession`] for a provider profile.
//! Claude and Cursor own their native-resume transports, Codex chat uses the
//! app-server protocol, DeepSeek uses its in-process tool agent, and Ollama
//! retains stateless replay.
//!
//! Standard `codex:` profiles remain `StatelessReplay` at the planner layer
//! because swarm still uses `codex exec`. At this chat-only construction seam,
//! both stateless `codex:` and explicit process-bound `codex-app-server:`
//! profiles use [`CodexAppServerSession`]. The app-server session renders replay
//! history when the profile is stateless.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::{Stream, StreamExt};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::protocol::find_next_file_block;
use crate::acp::session::AgentOptions;
use crate::context_planner::{ContinuityMode, ProviderProfile};
use crate::observer::AcpObserver;
use crate::swarm::backend::{StopReason, UnifiedStreamEvent};
use crate::write_gate::WriteGatePipeline;

use super::claude::ClaudeSession;
use super::codex_app_server::CodexAppServerSession;
use super::cursor::CursorSession;
use super::ollama::OllamaSession;
use super::tool_agent::ToolAgentSession;
use super::{AgentSession, Turn};

pub struct SessionConstruction {
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    pub observer: Box<dyn AcpObserver>,
    pub model: String,
    pub ollama_base_url: Option<String>,
    pub workspace_root: PathBuf,
    pub additional_roots: Vec<PathBuf>,
    pub agent_id: String,
    pub conv_id: Option<String>,
    pub options: AgentOptions,
    pub profile: ProviderProfile,
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
        let mut file_scan_pos = 0usize;
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
                    let summary =
                        crate::acp::client::format_tool_summary(&name, &args, &self.workspace_root);
                    self.observer.on_tool_call_started(&summary);
                    self.observer
                        .on_streaming_status(&format!("Using {name}..."));
                }
                UnifiedStreamEvent::ToolCallDelta { .. }
                | UnifiedStreamEvent::ToolCallEnd { .. }
                | UnifiedStreamEvent::PathsModified(_) => {}
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
                UnifiedStreamEvent::Error(message) => {
                    error = Some(message);
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

        if let Some(message) = error {
            anyhow::bail!(message);
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
        Ok(Box::pin(futures::stream::empty()))
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

fn create_observed_codex_session(args: SessionConstruction) -> Box<dyn AgentSession> {
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
        inner: Box::new(CodexAppServerSession::new(inner_args, observer.clone())),
        observer,
        write_gate,
        workspace_root,
        agent_id,
        conv_id,
        scan_text_file_blocks: false,
    })
}

pub fn create_session(args: SessionConstruction) -> Box<dyn AgentSession> {
    match args.profile.continuity_mode {
        ContinuityMode::NativeResume => {
            if args.profile.provider == "cursor" {
                Box::new(CursorSession::new(args))
            } else {
                Box::new(ClaudeSession::new(args))
            }
        }
        ContinuityMode::ProcessBound => create_observed_codex_session(args),
        ContinuityMode::StatelessReplay => {
            if args.profile.provider == "codex" {
                create_observed_codex_session(args)
            } else if args.profile.provider == "deepseek" {
                Box::new(ToolAgentSession::new(args))
            } else {
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

        fn on_proposal_deferred(
            &self,
            path: &Path,
            _old_content: Option<&str>,
            _new_content: &str,
        ) {
            self.events
                .lock()
                .unwrap()
                .push(format!("deferred:{}", path.display()));
        }

        fn on_turn_token_usage(&self, usage: &crate::acp::protocol::TokenUsage) {
            self.events.lock().unwrap().push(format!(
                "usage:{}/{}",
                usage.input_tokens, usage.output_tokens
            ));
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
            Ok(Box::pin(stream::iter(std::mem::take(&mut self.events))))
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
            metadata: PlannerMetadata::default(),
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
            agent_id: "test-agent".to_string(),
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
    async fn dispatches_stream_events_through_observer() {
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta("hi ".to_string())),
            Ok(UnifiedStreamEvent::ThinkingDelta("plan".to_string())),
            Ok(UnifiedStreamEvent::ToolCallStart {
                id: "1".to_string(),
                name: "Bash".to_string(),
                args: serde_json::json!({ "command": "ls -la" }),
            }),
            Ok(UnifiedStreamEvent::TextDelta("done".to_string())),
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

        session.send_turn(empty_turn()).await.unwrap();

        let recorded = recording.snapshot();
        assert!(recorded.iter().any(|event| event == "chunk:<think>\n"));
        assert!(recorded.iter().any(|event| event == "chunk:plan"));
        assert!(recorded.iter().any(|event| event == "chunk:\n</think>\n"));
        assert!(recorded.iter().any(|event| event == "chunk:hi "));
        assert!(recorded.iter().any(|event| event == "chunk:done"));
        assert!(recorded.iter().any(|event| event == "tool:Bash: ls -la"));
        assert!(recorded.iter().any(|event| event == "status:Using Bash..."));
        assert!(recorded.iter().any(|event| event == "usage:10/5"));
        assert!(
            recorded
                .iter()
                .any(|event| event == "complete:assistant:hi done")
        );
    }

    #[tokio::test]
    async fn inline_file_block_parser_remains_available_for_legacy_backends() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_root = temp_dir.path().to_path_buf();
        let text = concat!(
            "Here is the change:\n",
            "<file path=\"foo.txt\">hello world</",
            "file>\nDone."
        );
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta(text.to_string())),
            Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)),
        ];
        let recording = Arc::new(RecordingObserver::default());
        let write_gate = make_write_gate(WriteMode::Interactive);
        let mut session = make_wrapper(
            ScriptedSession { events },
            recording,
            write_gate.clone(),
            workspace_root.clone(),
            true,
        );

        session.send_turn(empty_turn()).await.unwrap();

        let gate = write_gate.lock().await;
        let active = gate.active_proposal_ids();
        assert_eq!(active.len(), 1);
        let proposal = gate.get_proposal(active[0]).unwrap();
        assert_eq!(proposal.file_path, workspace_root.join("foo.txt"));
        assert_eq!(proposal.proposed_content, "hello world");
        assert_eq!(proposal.original_content, "");
    }

    #[tokio::test]
    async fn disabling_file_block_scan_treats_markers_as_plain_text() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_root = temp_dir.path().to_path_buf();
        let text = concat!(
            "Quoted example: <file path=\"foo.txt\">hello world</",
            "file>"
        );
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta(text.to_string())),
            Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)),
        ];
        let write_gate = make_write_gate(WriteMode::Interactive);
        let mut session = make_wrapper(
            ScriptedSession { events },
            Arc::new(RecordingObserver::default()),
            write_gate.clone(),
            workspace_root,
            false,
        );

        session.send_turn(empty_turn()).await.unwrap();
        assert!(write_gate.lock().await.active_proposal_ids().is_empty());
    }

    #[tokio::test]
    async fn stream_error_surfaces_after_message_complete() {
        let events = vec![
            Ok(UnifiedStreamEvent::TextDelta("partial".to_string())),
            Ok(UnifiedStreamEvent::Error("backend exploded".to_string())),
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

        let error = match session.send_turn(empty_turn()).await {
            Ok(_) => panic!("expected stream error"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("backend exploded"));
        assert!(
            recording
                .snapshot()
                .iter()
                .any(|event| event == "complete:assistant:partial")
        );
    }
}
