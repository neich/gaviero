//! Swarm backend for `dsh:` — one ACP session per `stream_completion`.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::acp::session::AgentOptions;
use crate::agent_session::agent_client_protocol::AcpClientSession;
use crate::agent_session::registry::SessionConstruction;
use crate::agent_session::{AgentSession, TransportContext, build_turn};
use crate::context_planner::{PlannerMetadata, PlannerSelections};
use crate::observer::AcpObserver;
use crate::write_gate::{WriteGatePipeline, WriteMode};

use super::{
    AgentBackend, Capabilities, CompletionRequest, RetrievalToolset,
    UnifiedStreamEvent,
};

pub struct DshBackend {
    display_name: String,
}

impl DshBackend {
    pub fn new(model: &str) -> Self {
        Self {
            display_name: format!("dsh:{model}"),
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
            max_context_tokens: 128_000,
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
impl AgentBackend for DshBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let write_gate = request
            .write_gate
            .map(|h| h.0)
            .unwrap_or_else(|| {
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
            agent_id: "dsh".into(),
            conv_id: None,
            options,
            profile,
            cancel_token: CancellationToken::new(),
        };
        let mut session =
            AcpClientSession::new_with_scope(args, request.file_scope.clone()).with_extra(extra).with_system_prompt(request.system_prompt);
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
        let spec = crate::agent_session::agent_client_protocol::dsh::DshLaunchSpec::from_workspace_root(
            Path::new("."),
            &[],
        );
        if spec.command_resolvable() {
            Ok(())
        } else {
            anyhow::bail!(
                "dsh command `{}` not found on PATH (install @deepseek-ai/dsh)",
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
        let b = DshBackend::new("deepseek-v4-flash");
        assert_eq!(b.name(), "dsh:deepseek-v4-flash");
        assert!(b.capabilities().tool_use);
        assert!(!b.capabilities().supports_file_blocks);
    }
}
