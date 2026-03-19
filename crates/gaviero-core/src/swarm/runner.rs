use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use super::models::{AgentManifest, AgentStatus, WorkUnit};
use crate::acp::client::AcpPipeline;
use crate::acp::session::AgentOptions;
use crate::memory::MemoryStore;
use crate::observer::{AcpObserver, SwarmObserver};
use crate::write_gate::WriteGatePipeline;

/// Runs a single work unit by spawning an ACP agent session.
///
/// In M3a this writes directly to the workspace (no git worktree).
pub struct AgentRunner {
    write_gate: Arc<Mutex<WriteGatePipeline>>,
    workspace_root: PathBuf,
    memory: Option<Arc<MemoryStore>>,
    /// Namespaces to search for memory context.
    read_namespaces: Vec<String>,
}

impl AgentRunner {
    pub fn new(
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        workspace_root: PathBuf,
        memory: Option<Arc<MemoryStore>>,
    ) -> Self {
        Self {
            write_gate,
            workspace_root,
            memory,
            read_namespaces: vec!["default".to_string()],
        }
    }

    /// Set the namespaces to search when reading memory context.
    pub fn with_read_namespaces(mut self, namespaces: Vec<String>) -> Self {
        self.read_namespaces = namespaces;
        self
    }

    /// Execute a single work unit and return an agent manifest.
    pub async fn run(
        &self,
        work_unit: &WorkUnit,
        observer: Box<dyn AcpObserver>,
        swarm_observer: Option<&dyn SwarmObserver>,
    ) -> Result<AgentManifest> {
        let agent_id = format!("agent-{}", work_unit.id);

        if let Some(obs) = swarm_observer {
            obs.on_agent_state_changed(&work_unit.id, &AgentStatus::Running, "starting");
        }

        // Register the agent's file scope with the write gate
        {
            let mut gate = self.write_gate.lock().await;
            gate.register_agent_scope(&agent_id, &work_unit.scope);
        }

        // Build prompt with scope clause and optional memory context
        let mut prompt_parts = Vec::new();

        // Add memory context if available (searches across all read namespaces)
        if let Some(ref memory) = self.memory {
            let ctx = memory.search_context(&self.read_namespaces, &work_unit.description, 5).await;
            if !ctx.is_empty() {
                prompt_parts.push(ctx);
            }
        }

        // Add scope clause
        let scope_clause = work_unit.scope.to_prompt_clause();
        if !scope_clause.is_empty() {
            prompt_parts.push(format!("[File scope]:\n{}", scope_clause));
        }

        // Add task description
        prompt_parts.push(work_unit.description.clone());

        let full_prompt = prompt_parts.join("\n\n");

        // Determine model
        let model = work_unit.model.as_deref().unwrap_or("sonnet").to_string();

        // Create ACP pipeline and send the prompt
        let options = AgentOptions::default();
        let pipeline = AcpPipeline::new(
            self.write_gate.clone(),
            observer,
            &model,
            self.workspace_root.clone(),
            &agent_id,
            options,
        );

        let result = pipeline.send_prompt(&full_prompt, &[], &[], &[]).await;

        // Collect modified files from the write gate
        let modified_files = {
            let gate = self.write_gate.lock().await;
            gate.active_proposal_ids()
                .iter()
                .filter_map(|id| {
                    gate.get_proposal(*id)
                        .map(|p| p.file_path.clone())
                })
                .collect::<Vec<PathBuf>>()
        };

        let (status, summary) = match result {
            Ok(()) => (
                AgentStatus::Completed,
                Some(format!("Modified {} files", modified_files.len())),
            ),
            Err(e) => (
                AgentStatus::Failed(e.to_string()),
                Some(e.to_string()),
            ),
        };

        if let Some(obs) = swarm_observer {
            obs.on_agent_state_changed(&work_unit.id, &status, summary.as_deref().unwrap_or(""));
        }

        Ok(AgentManifest {
            work_unit_id: work_unit.id.clone(),
            status,
            modified_files,
            branch: None, // No git worktree in M3a
            summary,
        })
    }
}
