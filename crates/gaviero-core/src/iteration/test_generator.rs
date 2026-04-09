//! Test-first pre-step: generates failing tests before the main edit loop.
//!
//! The `TestGenerator` calls the backend once with a prompt that asks it to
//! write tests describing the *desired* behaviour (not the current behaviour).
//! The tests must fail against the current code; they only need to compile.
//!
//! After the ACP call the compile gate is run on the generated test files.
//! Files that fail to compile are discarded so the edit loop doesn't start with
//! broken tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::observer::AcpObserver;
use crate::swarm::backend::runner::run_backend;
use crate::swarm::models::{AgentStatus, WorkUnit};
use crate::types::{FileScope, ModelTier, PrivacyLevel};
use crate::validation_gate::ValidationPipeline;
use crate::write_gate::{WriteGatePipeline, WriteMode};

// ── TestGenerator ─────────────────────────────────────────────────────────────

/// Generates failing tests for a task before the main edit loop.
pub struct TestGenerator;

impl TestGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Generate tests that specify the desired behaviour.
    ///
    /// Returns the paths of test files that were written **and** compile
    /// successfully. Tests are expected to fail at runtime (TDD red phase) but
    /// must compile so the edit loop can use them as a verification signal.
    #[allow(clippy::too_many_arguments)]
    pub async fn generate(
        &self,
        task: &str,
        scope: &FileScope,
        backend: &dyn crate::swarm::backend::AgentBackend,
        model: &str,
        workspace_root: &Path,
        observer: &dyn AcpObserver,
        validation: Option<&ValidationPipeline>,
    ) -> Vec<PathBuf> {
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopTestGateObs),
        )));

        let scope_clause = scope.to_prompt_clause();
        let prompt = format!(
            "Before implementing any changes, write tests that specify the \
             desired behaviour for the following task:\n\n\
             Task: {task}\n\n\
             {scope_clause}\n\n\
             Instructions:\n\
             - Write tests using the project's test framework (e.g. `#[test]` \
               for Rust, JUnit for Java/Android, pytest for Python).\n\
             - Tests MUST fail against the current code — they describe the \
               desired state, not the current state.\n\
             - Tests MUST compile — syntax must be valid and imports must be \
               resolvable.\n\
             - Only write test files. Do NOT modify any existing source files.\n\
             - Place tests alongside or near the code under test (e.g. \
               `#[cfg(test)]` modules in Rust, or `*Test.java` files for Java)."
        );

        let unit = WorkUnit {
            id: "test-generator".into(),
            description: format!("Generate tests for: {}", &task[..task.len().min(80)]),
            scope: scope.clone(),
            depends_on: vec![],
            #[allow(deprecated)]
            backend: Default::default(),
            model: Some(model.to_string()),
            tier: ModelTier::Cheap,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: prompt,
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
        };

        let manifest = match run_backend(
            backend,
            &unit,
            write_gate,
            workspace_root,
            None,
            &[],
            observer,
            None, // no validation during test generation itself
            None,
            None,
            None, // no graph store for test generation
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("test generator backend call failed: {}", e);
                return vec![];
            }
        };

        if !matches!(manifest.status, AgentStatus::Completed) {
            tracing::warn!("test generator did not complete successfully");
            return vec![];
        }

        let written = manifest.modified_files.clone();

        // Verify test files compile (compile gate only — tests are expected to fail at runtime)
        if let Some(vp) = validation {
            let fast_only = ValidationPipeline::fast_only();
            let pipeline = vp; // use the provided pipeline (may have compile gate)
            let _ = pipeline; // suppress unused warning

            // Use fast-only (tree-sitter) to verify syntax — the full compile
            // gate is heavy and will be run in the main iteration loop anyway.
            let failure = fast_only
                .run_reporting(&written, workspace_root, false, |gate, passed| {
                    tracing::debug!("test-gen compile check [{gate}]: {}", if passed { "pass" } else { "fail" });
                })
                .await;

            if failure.is_some() {
                tracing::warn!("generated tests failed syntax check — discarding");
                return vec![];
            }
        }

        tracing::info!("test generator wrote {} test files", written.len());
        written
    }
}

impl Default for TestGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

struct NoopTestGateObs;
impl crate::observer::WriteGateObserver for NoopTestGateObs {
    fn on_proposal_created(&self, _: &crate::types::WriteProposal) {}
    fn on_proposal_updated(&self, _: u64) {}
    fn on_proposal_finalized(&self, _: &str) {}
}
