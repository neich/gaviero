//! Swarm backend for agy: — one ACP session per stream_completion.
//!
//! Mirrors [`super::dsh::DshBackend`]: each unit gets a fresh
//! `AcpClientSession` over `agy acp`. agy edits files itself (never
//! `fs/write_text_file`), so the session's git dirty-set reconcile turns those
//! edits into Write Gate proposals.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::session::AgentOptions;
use crate::agent_session::agent_client_protocol::AcpClientSession;
use crate::agent_session::agent_client_protocol::agy::AgyLaunchSpec;
use crate::agent_session::registry::SessionConstruction;
use crate::agent_session::{AgentSession, TransportContext, build_turn};
use crate::context_planner::{PlannerMetadata, PlannerSelections};
use crate::observer::AcpObserver;
use crate::write_gate::{WriteGatePipeline, WriteMode};

use super::{AgentBackend, Capabilities, CompletionRequest, RetrievalToolset, UnifiedStreamEvent};

/// Context window agy reports on every session (`usage_update.size`,
/// agy 1.5.18).
pub const AGY_CONTEXT_WINDOW: usize = 1_048_576;

/// The model a fresh `agy acp` session starts on (`models.currentModelId`,
/// agy 1.5.18). Used when model discovery cannot ask the CLI.
pub const AGY_DEFAULT_MODEL: &str = "gemini-4-carbon-medium";

pub struct AgyBackend {
    display_name: String,
}

impl AgyBackend {
    pub fn new(model: &str) -> Self {
        Self {
            display_name: format!("agy:{model}"),
        }
    }

    fn capabilities_for(exposed: Option<&[String]>) -> Capabilities {
        let retrieval = match exposed {
            Some(tools) => RetrievalToolset::from_exposed(tools),
            None => RetrievalToolset {
                graph_and_memory: true,
                symbols: false,
                exposed: Vec::new(),
            },
        };
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
            extended_thinking: true,
            max_context_tokens: AGY_CONTEXT_WINDOW,
            supports_system_prompt: true,
            supports_file_blocks: false,
            retrieval,
        }
    }
}

struct NoopObserver;
impl AcpObserver for NoopObserver {
    fn on_stream_chunk(&self, _text: &str) {}
    fn on_tool_call_started(&self, _tool_name: &str) {}
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, _role: &str, _content: &str) {}
    fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
}

struct NoopWrite;
impl crate::observer::WriteGateObserver for NoopWrite {
    fn on_proposal_created(&self, _proposal: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _proposal_id: u64) {}
    fn on_proposal_finalized(&self, _path: &str) {}
}

#[async_trait::async_trait]
impl AgentBackend for AgyBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let write_gate = request.write_gate.map(|h| h.0).unwrap_or_else(|| {
            Arc::new(Mutex::new(WriteGatePipeline::new(
                WriteMode::RejectAll,
                Box::new(NoopWrite),
            )))
        });
        let extra = request.extra.clone();
        let spec = crate::context_planner::ModelSpec::parse(&self.display_name);
        let profile = crate::context_planner::build_provider_profile(
            &spec,
            &crate::context_planner::RuntimeConfig::default(),
        );
        #[allow(deprecated)]
        let options = AgentOptions {
            effort: request.effort.clone().unwrap_or_else(|| "off".into()),
            auto_approve: request.auto_approve,
            exposed_tools: request.exposed_tools.clone(),
            ..AgentOptions::default()
        };
        let args = SessionConstruction {
            write_gate,
            observer: Box::new(NoopObserver),
            model: self.display_name.clone(),
            ollama_base_url: None,
            workspace_root: request.workspace_root.clone(),
            additional_roots: request.additional_roots.clone(),
            agent_id: "agy".into(),
            conv_id: None,
            options,
            profile,
            cancel_token: CancellationToken::new(),
            mcp_server: None,
        };
        let mut session = AcpClientSession::new_with_scope(args, request.file_scope.clone())
            .with_extra(extra)
            .with_system_prompt(request.system_prompt);
        let turn = build_turn(
            PlannerSelections {
                memory_selections: vec![],
                graph_selections: vec![],
                file_refs: vec![],
                skill_selections: vec![],
                replay_history: None,
                metadata: PlannerMetadata::default(),
            },
            TransportContext {
                user_message: request.prompt,
                effort: request.effort,
                auto_approve: request.auto_approve,
            },
        );
        session.send_turn(turn).await
    }

    fn capabilities(&self) -> Capabilities {
        Self::capabilities_for(None)
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let spec = AgyLaunchSpec::from_workspace_root(Path::new("."), &[]);
        if spec.command_resolvable() {
            Ok(())
        } else {
            anyhow::bail!(
                "agy command `{}` not found on PATH (install the Antigravity CLI)",
                spec.command
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_includes_model() {
        let b = AgyBackend::new("gemini-3.8-flash-low");
        assert_eq!(b.name(), "agy:gemini-3.8-flash-low");
        assert!(b.capabilities().tool_use);
        assert!(!b.capabilities().supports_file_blocks);
        assert_eq!(b.capabilities().max_context_tokens, AGY_CONTEXT_WINDOW);
    }
}
