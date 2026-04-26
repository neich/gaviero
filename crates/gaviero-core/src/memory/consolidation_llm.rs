//! Minimal LLM abstraction for memory consolidation / extraction.
//!
//! The writer task invokes `ConsolidationLlm::complete` when processing
//! `TurnComplete` messages (Phase 4 — per-turn extractor). Keeping the trait
//! narrow (single async `complete` method) lets Phase 1 land without pulling
//! in streaming, cancellation, or tool plumbing — those can be added later
//! without churn on the writer task itself.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::swarm::backend::{AgentBackend, CompletionRequest, executor};

/// Narrow async completion interface used by memory consolidation paths.
///
/// Deliberately does **not** expose streaming or cancellation: the extractor
/// needs one JSON blob back, nothing else. Add capability to the trait only
/// when a concrete consumer requires it (see Phase 1 notes re: `CancellationToken`).
#[async_trait]
pub trait ConsolidationLlm: Send + Sync {
    /// Run the prompt to completion and return the full assistant text.
    async fn complete(&self, prompt: String) -> Result<String>;
}

/// `ConsolidationLlm` implementation backed by any `AgentBackend`. The
/// concrete backend is normally chosen by `TierRouter` at the "cheap /
/// mechanical" tier (Haiku / Ollama / Codex-mini — not pinned here).
pub struct BackendConsolidationLlm {
    backend: Arc<dyn AgentBackend>,
    workspace_root: PathBuf,
}

impl BackendConsolidationLlm {
    pub fn new(backend: Arc<dyn AgentBackend>, workspace_root: PathBuf) -> Self {
        Self {
            backend,
            workspace_root,
        }
    }
}

#[async_trait]
impl ConsolidationLlm for BackendConsolidationLlm {
    async fn complete(&self, prompt: String) -> Result<String> {
        let request = CompletionRequest {
            prompt,
            system_prompt: None,
            workspace_root: self.workspace_root.clone(),
            allowed_tools: Vec::new(),
            file_attachments: Vec::new(),
            conversation_history: Vec::new(),
            file_refs: Vec::new(),
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        };
        let outcome = executor::complete_to_text(self.backend.as_ref(), request, None).await?;
        Ok(outcome.text)
    }
}

/// Fallback implementation that always returns an error. Useful when no
/// backend is available yet but the writer task still needs to be
/// constructable (tests, headless bootstrap).
pub struct NoopConsolidationLlm;

#[async_trait]
impl ConsolidationLlm for NoopConsolidationLlm {
    async fn complete(&self, _prompt: String) -> Result<String> {
        anyhow::bail!("ConsolidationLlm is not configured")
    }
}
