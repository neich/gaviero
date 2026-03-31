#![allow(deprecated)]
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use super::models::{AgentManifest, AgentStatus, WorkUnit};
use super::ollama::{self, OllamaBackend};
use crate::acp::client::AcpPipeline;
use crate::acp::session::AgentOptions;
use crate::memory::MemoryStore;
use crate::observer::{AcpObserver, SwarmObserver};
use crate::write_gate::WriteGatePipeline;

/// Runs a single work unit by spawning an ACP agent session.
///
/// **Deprecated:** Use [`super::backend::runner::run_backend`] with a
/// [`super::backend::AgentBackend`] trait object instead.
#[deprecated(note = "Use backend::runner::run_backend with trait objects instead")]
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
        let model = work_unit.model.as_deref().unwrap_or("sonnet");
        self.run_with_model(work_unit, observer, swarm_observer, model).await
    }

    /// Execute a work unit with an explicit model (resolved by TierRouter).
    ///
    /// `coordinator_instructions` is used as the task description when non-empty,
    /// falling back to `work_unit.description` for backward compatibility.
    pub async fn run_with_model(
        &self,
        work_unit: &WorkUnit,
        observer: Box<dyn AcpObserver>,
        swarm_observer: Option<&dyn SwarmObserver>,
        model: &str,
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

        // Use coordinator_instructions if present, else description
        let task_text = if work_unit.coordinator_instructions.is_empty() {
            &work_unit.description
        } else {
            &work_unit.coordinator_instructions
        };
        prompt_parts.push(task_text.clone());

        let full_prompt = prompt_parts.join("\n\n");

        // Create ACP pipeline and send the prompt
        let options = AgentOptions::default();
        let pipeline = AcpPipeline::new(
            self.write_gate.clone(),
            observer,
            model,
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
            branch: None,
            summary,
        })
    }

    /// Execute a work unit via Ollama (local LLM).
    ///
    /// Builds a focused prompt from coordinator_instructions + file contents,
    /// calls OllamaBackend::generate(), extracts <file> blocks from the
    /// response, and routes each through propose_write().
    pub async fn run_ollama(
        &self,
        work_unit: &WorkUnit,
        ollama_model: &str,
        ollama_base_url: &str,
        swarm_observer: Option<&dyn SwarmObserver>,
    ) -> Result<AgentManifest> {
        if let Some(obs) = swarm_observer {
            obs.on_agent_state_changed(
                &work_unit.id,
                &AgentStatus::Running,
                &format!("ollama:{}", ollama_model),
            );
        }

        let agent_id = format!("agent-{}", work_unit.id);

        // Register scope
        {
            let mut gate = self.write_gate.lock().await;
            gate.register_agent_scope(&agent_id, &work_unit.scope);
        }

        // Build focused prompt for the local model (mechanical tier template)
        let mut prompt = String::new();

        // Coordinator instructions (terse, precise)
        let task = if work_unit.coordinator_instructions.is_empty() {
            &work_unit.description
        } else {
            &work_unit.coordinator_instructions
        };
        prompt.push_str(&format!("TASK: {}\n\n", task));

        // File contents from scope
        prompt.push_str("FILES YOU MAY MODIFY:\n");
        for path in &work_unit.scope.owned_paths {
            let full_path = self.workspace_root.join(path);
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                prompt.push_str(&format!(
                    "<file path=\"{}\">\n{}\n</file>\n\n",
                    path, content
                ));
            }
        }

        prompt.push_str(
            "OUTPUT: For each file you modify, output the complete file wrapped in:\n\
             <file path=\"relative/path\">\n...complete file content...\n</file>\n\n\
             RULES:\n\
             - Output ONLY <file> blocks. No explanations, no commentary.\n\
             - Include the COMPLETE file content, not just changed sections.\n\
             - Do NOT rename files, create new files, or delete files.\n",
        );

        let system = "You are executing a precise code modification task. \
                      Follow the instructions exactly. Do not add, remove, \
                      or modify anything beyond what is specified.";

        // Call Ollama
        let backend = OllamaBackend::new(ollama_base_url, ollama_model);
        let response = backend.generate(&prompt, system).await;

        let mut modified_files = Vec::new();
        let (status, summary) = match response {
            Ok(text) => {
                // Extract <file> blocks and write to disk
                let blocks = ollama::extract_file_blocks(&text);
                for (path, content) in &blocks {
                    let path_str = path.to_string_lossy();
                    let gate = self.write_gate.lock().await;
                    if gate.is_scope_allowed(&agent_id, &path_str) {
                        drop(gate); // Release lock before I/O
                        let full_path = self.workspace_root.join(path);
                        if let Some(parent) = full_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::write(&full_path, content) {
                            tracing::warn!("Failed to write {}: {}", full_path.display(), e);
                        } else {
                            modified_files.push(full_path);
                        }
                    }
                }
                (
                    AgentStatus::Completed,
                    Some(format!("Modified {} files via Ollama", modified_files.len())),
                )
            }
            Err(e) => (
                AgentStatus::Failed(e.to_string()),
                Some(format!("Ollama error: {}", e)),
            ),
        };

        if let Some(obs) = swarm_observer {
            obs.on_agent_state_changed(&work_unit.id, &status, summary.as_deref().unwrap_or(""));
        }

        Ok(AgentManifest {
            work_unit_id: work_unit.id.clone(),
            status,
            modified_files,
            branch: None,
            summary,
        })
    }
}
