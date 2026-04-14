//! Iteration engine: wraps the backend runner in a strategy-driven outer loop.
//!
//! The `IterationEngine` owns the execution strategy (`SinglePass`, `Refine`,
//! `BestOfN`) and model-escalation logic. It calls `swarm::backend::run_backend()`
//! internally for each attempt and selects the best result.
//!
//! ## Relationship to the inner retry loop
//! `run_backend()` already contains an *inner* retry loop driven by
//! `WorkUnit::max_retries` (validation → corrective prompt → re-run).
//! `IterationEngine` is the *outer* loop: it controls how many independent
//! attempts are launched (`BestOfN`), how to escalate the model tier between
//! attempts, and whether to generate tests before editing (`test_first`).

pub mod convergence;
pub mod test_generator;

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::memory::MemoryStore;
use crate::observer::AcpObserver;
use crate::repo_map::RepoMap;
use crate::swarm::backend::AgentBackend;
use crate::swarm::backend::runner::run_backend;
use crate::swarm::board::SharedBoard;
use crate::swarm::models::{AgentManifest, AgentStatus, WorkUnit};
use crate::types::ModelTier;
use crate::validation_gate::ValidationPipeline;
use crate::write_gate::WriteGatePipeline;

use self::convergence::ConvergenceDetector;
use self::test_generator::TestGenerator;

// ── Strategy ─────────────────────────────────────────────────────────────────

/// Execution strategy for the iteration engine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// One attempt, no retry beyond the inner validation loop. Legacy behaviour.
    SinglePass,
    /// Iterate until validation passes or the budget is exhausted. **Default.**
    #[default]
    Refine,
    /// Generate N independent attempts; return the one with the most modified files.
    BestOfN { n: u32 },
}

// ── IterationConfig ───────────────────────────────────────────────────────────

/// Configuration for a single `IterationEngine` run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IterationConfig {
    /// Outer-loop strategy.
    pub strategy: Strategy,
    /// Inner-loop retries per attempt (fed into `WorkUnit::max_retries`).
    pub max_retries: u32,
    /// Number of independent attempts for `BestOfN` (ignored otherwise).
    pub max_attempts: u32,
    /// Generate failing tests before the edit loop.
    pub test_first: bool,
    /// Model used for cheap attempts (default: Haiku).
    pub cheap_model: String,
    /// Model used after escalation (default: Sonnet).
    pub expensive_model: String,
    /// Switch to expensive model after this many failed attempts.
    pub escalate_after: u32,
}

impl Default for IterationConfig {
    fn default() -> Self {
        Self {
            strategy: Strategy::Refine,
            max_retries: 5,
            max_attempts: 1,
            test_first: false,
            cheap_model: "claude-haiku-4-5-20251001".into(),
            expensive_model: "claude-sonnet-4-6".into(),
            escalate_after: 3,
        }
    }
}

// ── IterationResult ───────────────────────────────────────────────────────────

/// The outcome of an `IterationEngine::run()` call.
#[derive(Debug)]
pub struct IterationResult {
    /// The best manifest produced across all attempts.
    pub manifest: AgentManifest,
    /// Number of outer attempts actually executed.
    pub attempts_run: u32,
    /// Whether all validation gates passed on the winning attempt.
    pub all_passed: bool,
}

impl IterationResult {
    /// Convert into a single-element `SwarmResult`-compatible manifest vec.
    pub fn into_manifest(self) -> AgentManifest {
        self.manifest
    }
}

// ── IterationEngine ───────────────────────────────────────────────────────────

/// Outer-loop execution controller.
pub struct IterationEngine {
    pub config: IterationConfig,
}

impl IterationEngine {
    pub fn new(config: IterationConfig) -> Self {
        Self { config }
    }

    /// Execute the strategy loop, returning the best result across all attempts.
    ///
    /// * `SinglePass` — one attempt with `max_retries=1` in the inner loop.
    /// * `Refine` — one attempt with `max_retries` from the config.
    /// * `BestOfN { n }` — `n` independent attempts; returns the first that fully
    ///   passes validation, or the one with the most modified files if none do.
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        backend: &dyn crate::swarm::backend::AgentBackend,
        work_unit: WorkUnit,
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        workspace_root: &Path,
        memory: Option<&MemoryStore>,
        read_namespaces: &[String],
        observer: &dyn AcpObserver,
        validation: Option<&ValidationPipeline>,
        board: Option<&SharedBoard>,
        repo_map: Option<&RepoMap>,
        impact_text: Option<&str>,
        pre_fetched_memory: Option<&str>,
    ) -> IterationResult {
        let n_attempts = match &self.config.strategy {
            Strategy::SinglePass => 1,
            Strategy::Refine => 1,
            Strategy::BestOfN { n } => *n,
        };

        // Test-first: generate failing tests before the edit loop
        if self.config.test_first && !matches!(self.config.strategy, Strategy::SinglePass) {
            let generator = TestGenerator::new();
            let test_files = generator
                .generate(
                    &work_unit.coordinator_instructions,
                    &work_unit.scope,
                    backend,
                    &self.config.cheap_model,
                    workspace_root,
                    observer,
                    validation,
                )
                .await;
            if test_files.is_empty() {
                tracing::warn!("test-first: no test files generated — proceeding without tests");
            } else {
                tracing::info!("test-first: {} test files generated", test_files.len());
            }
        }

        let mut best: Option<(AgentManifest, usize)> = None;
        let mut detector = ConvergenceDetector::new();

        for attempt in 0..n_attempts {
            let unit = self.unit_for_attempt(&work_unit, attempt);

            let manifest: AgentManifest = match run_backend(
                backend,
                &unit,
                write_gate.clone(),
                workspace_root,
                memory,
                read_namespaces,
                observer,
                validation,
                board,
                repo_map,
                impact_text,
                pre_fetched_memory,
            )
            .await
            {
                Ok(m) => m,
                Err(e) => AgentManifest {
                    work_unit_id: unit.id.clone(),
                    status: AgentStatus::Failed(format!("{e}")),
                    modified_files: vec![],
                    branch: None,
                    summary: Some(format!("{e}")),
                    output: None,
                    cost_usd: 0.0,
                },
            };

            let succeeded = manifest.status == AgentStatus::Completed;
            let file_count = manifest.modified_files.len();

            // Track best by file count
            if best.as_ref().map_or(true, |(_, c)| file_count > *c) {
                best = Some((manifest.clone(), file_count));
            }

            if succeeded {
                return IterationResult {
                    manifest,
                    attempts_run: attempt + 1,
                    all_passed: true,
                };
            }

            // Stall detection (for BestOfN)
            if detector.record(&manifest.modified_files) {
                tracing::debug!(
                    attempt,
                    "iteration engine: stall detected, stopping early"
                );
                break;
            }
        }

        let (manifest, _) = best.unwrap_or_else(|| {
            (
                AgentManifest {
                    work_unit_id: work_unit.id.clone(),
                    status: AgentStatus::Failed("no attempts produced output".into()),
                    modified_files: vec![],
                    branch: None,
                    summary: None,
                    output: None,
                    cost_usd: 0.0,
                },
                0,
            )
        });

        IterationResult {
            attempts_run: n_attempts,
            all_passed: false,
            manifest,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_backend_factory<F>(
        &self,
        work_unit: WorkUnit,
        write_gate: Arc<Mutex<WriteGatePipeline>>,
        workspace_root: &Path,
        memory: Option<&MemoryStore>,
        read_namespaces: &[String],
        observer: &dyn AcpObserver,
        validation: Option<&ValidationPipeline>,
        board: Option<&SharedBoard>,
        repo_map: Option<&RepoMap>,
        impact_text: Option<&str>,
        pre_fetched_memory: Option<&str>,
        resolve_backend: F,
    ) -> IterationResult
    where
        F: Fn(&WorkUnit) -> anyhow::Result<Box<dyn AgentBackend>>,
    {
        let n_attempts = match &self.config.strategy {
            Strategy::SinglePass => 1,
            Strategy::Refine => 1,
            Strategy::BestOfN { n } => *n,
        };

        if self.config.test_first && !matches!(self.config.strategy, Strategy::SinglePass) {
            let generator = TestGenerator::new();
            let generator_unit = self.unit_for_attempt(&work_unit, 0);
            match resolve_backend(&generator_unit) {
                Ok(backend) => {
                    let model_name = generator_unit
                        .model
                        .as_deref()
                        .unwrap_or(&self.config.cheap_model);
                    let test_files = generator
                        .generate(
                            &work_unit.coordinator_instructions,
                            &work_unit.scope,
                            backend.as_ref(),
                            model_name,
                            workspace_root,
                            observer,
                            validation,
                        )
                        .await;
                    if test_files.is_empty() {
                        tracing::warn!("test-first: no test files generated — proceeding without tests");
                    } else {
                        tracing::info!("test-first: {} test files generated", test_files.len());
                    }
                }
                Err(e) => {
                    tracing::warn!("test-first: backend resolution failed: {}", e);
                }
            }
        }

        let mut best: Option<(AgentManifest, usize)> = None;
        let mut detector = ConvergenceDetector::new();

        for attempt in 0..n_attempts {
            let unit = self.unit_for_attempt(&work_unit, attempt);

            let manifest = match resolve_backend(&unit) {
                Ok(backend) => match run_backend(
                    backend.as_ref(),
                    &unit,
                    write_gate.clone(),
                    workspace_root,
                    memory,
                    read_namespaces,
                    observer,
                    validation,
                    board,
                    repo_map,
                    impact_text,
                    pre_fetched_memory,
                )
                .await
                {
                    Ok(m) => m,
                    Err(e) => AgentManifest {
                        work_unit_id: unit.id.clone(),
                        status: AgentStatus::Failed(format!("{e}")),
                        modified_files: vec![],
                        branch: None,
                        summary: Some(format!("{e}")),
                        output: None,
                        cost_usd: 0.0,
                    },
                },
                Err(e) => AgentManifest {
                    work_unit_id: unit.id.clone(),
                    status: AgentStatus::Failed(format!("{e}")),
                    modified_files: vec![],
                    branch: None,
                    summary: Some(format!("{e}")),
                    output: None,
                    cost_usd: 0.0,
                },
            };

            let succeeded = manifest.status == AgentStatus::Completed;
            let file_count = manifest.modified_files.len();

            if best.as_ref().map_or(true, |(_, c)| file_count > *c) {
                best = Some((manifest.clone(), file_count));
            }

            if succeeded {
                return IterationResult {
                    manifest,
                    attempts_run: attempt + 1,
                    all_passed: true,
                };
            }

            if detector.record(&manifest.modified_files) {
                tracing::debug!(
                    attempt,
                    "iteration engine: stall detected, stopping early"
                );
                break;
            }
        }

        let (manifest, _) = best.unwrap_or_else(|| {
            (
                AgentManifest {
                    work_unit_id: work_unit.id.clone(),
                    status: AgentStatus::Failed("no attempts produced output".into()),
                    modified_files: vec![],
                    branch: None,
                    summary: None,
                    output: None,
                    cost_usd: 0.0,
                },
                0,
            )
        });

        IterationResult {
            attempts_run: n_attempts,
            all_passed: false,
            manifest,
        }
    }

    fn unit_for_attempt(&self, work_unit: &WorkUnit, attempt: u32) -> WorkUnit {
        let mut unit = work_unit.clone();

        unit.max_retries = match &self.config.strategy {
            Strategy::SinglePass => 1,
            _ => self.config.max_retries.min(u8::MAX as u32) as u8,
        };

        if attempt >= self.config.escalate_after {
            unit.model = Some(self.config.expensive_model.clone());
            unit.tier = ModelTier::Expensive;
        } else {
            unit.model = Some(self.config.cheap_model.clone());
            unit.tier = ModelTier::Cheap;
        }

        unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::WriteGateObserver;
    use crate::swarm::backend::mock::MockBackend;
    use crate::swarm::backend::{StopReason, UnifiedStreamEvent};
    use crate::swarm::models::AgentStatus;
    use crate::types::{FileScope, PrivacyLevel, WriteProposal};
    use crate::write_gate::{WriteGatePipeline, WriteMode};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct NoopGateObs;
    impl WriteGateObserver for NoopGateObs {
        fn on_proposal_created(&self, _: &WriteProposal) {}
        fn on_proposal_updated(&self, _: u64) {}
        fn on_proposal_finalized(&self, _: &str) {}
    }

    fn make_unit(id: &str) -> WorkUnit {
        WorkUnit {
            id: id.into(),
            description: "test task".into(),
            scope: FileScope {
                owned_paths: vec![".".into()],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            #[allow(deprecated)]
            backend: Default::default(),
            model: None,
            tier: ModelTier::Cheap,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: "do something".into(),
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
        }
    }

    fn make_gate() -> Arc<Mutex<WriteGatePipeline>> {
        Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::AutoAccept,
            Box::new(NoopGateObs),
        )))
    }

    fn success_backend() -> MockBackend {
        MockBackend::new(
            "test",
            vec![
                UnifiedStreamEvent::TextDelta("task done".into()),
                UnifiedStreamEvent::Done(StopReason::EndTurn),
            ],
        )
    }

    struct NoopObserver;
    impl AcpObserver for NoopObserver {
        fn on_stream_chunk(&self, _: &str) {}
        fn on_tool_call_started(&self, _: &str) {}
        fn on_streaming_status(&self, _: &str) {}
        fn on_message_complete(&self, _: &str, _: &str) {}
        fn on_proposal_deferred(&self, _: &Path, _: Option<&str>, _: &str) {}
    }

    #[tokio::test]
    async fn single_pass_runs_once() {
        let engine = IterationEngine::new(IterationConfig {
            strategy: Strategy::SinglePass,
            ..Default::default()
        });
        let backend = success_backend();
        let result = engine
            .run(
                &backend,
                make_unit("t"),
                make_gate(),
                Path::new("/tmp"),
                None,
                &[],
                &NoopObserver,
                None,
                None,
                None,
                None,
                None,
            )
            .await;
        assert_eq!(result.attempts_run, 1);
        assert!(result.all_passed);
        assert_eq!(result.manifest.status, AgentStatus::Completed);
    }

    #[tokio::test]
    async fn best_of_n_returns_early_on_success() {
        let engine = IterationEngine::new(IterationConfig {
            strategy: Strategy::BestOfN { n: 3 },
            ..Default::default()
        });
        // MockBackend always succeeds → early return after 1 attempt
        let backend = success_backend();
        let result = engine
            .run(
                &backend,
                make_unit("t"),
                make_gate(),
                Path::new("/tmp"),
                None,
                &[],
                &NoopObserver,
                None,
                None,
                None,
                None,
                None,
            )
            .await;
        assert_eq!(result.attempts_run, 1);
        assert!(result.all_passed);
    }

    #[tokio::test]
    async fn backend_factory_re_resolves_between_attempts() {
        let engine = IterationEngine::new(IterationConfig {
            strategy: Strategy::BestOfN { n: 2 },
            escalate_after: 1,
            cheap_model: "cheap-model".into(),
            expensive_model: "expensive-model".into(),
            ..Default::default()
        });
        let seen_models = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let result = engine
            .run_with_backend_factory(
                make_unit("t"),
                make_gate(),
                Path::new("/tmp"),
                None,
                &[],
                &NoopObserver,
                None,
                None,
                None,
                None,
                None,
                {
                    let seen_models = Arc::clone(&seen_models);
                    move |unit| {
                        seen_models
                            .lock()
                            .expect("recorded models lock")
                            .push(unit.model.clone().unwrap_or_default());
                        Ok(Box::new(MockBackend::new(
                            "failing",
                            vec![
                                UnifiedStreamEvent::Error("attempt failed".into()),
                                UnifiedStreamEvent::Done(StopReason::Error),
                            ],
                        )) as Box<dyn crate::swarm::backend::AgentBackend>)
                    }
                },
            )
            .await;

        let seen = seen_models.lock().expect("recorded models lock").clone();
        assert_eq!(seen, vec!["cheap-model".to_string(), "expensive-model".to_string()]);
        assert!(!result.all_passed);
        assert_eq!(result.attempts_run, 2);
    }
}
