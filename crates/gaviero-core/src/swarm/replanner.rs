//! Dynamic replanning after agent failures.
//!
//! When a node fails with `HardFailure` after exhausting retries, the
//! `Replanner` can ask Opus to revise the remaining plan. The revised plan
//! is compiled from `.gaviero` DSL text and replaces the current plan's
//! remaining nodes.
//!
//! See Phase 3 of the implementation plan.

use super::execution_state::ExecutionState;
use super::plan::CompiledPlan;

/// Decision returned by the replanner after evaluating a failure.
#[derive(Debug)]
pub enum ReplanDecision {
    /// No changes needed; continue with the current plan.
    Continue,
    /// Retry the listed work units with adjusted prompts.
    RetryFailed(Vec<String>),
    /// Replace the remaining plan with an Opus-generated revision.
    RevisePlan(CompiledPlan),
    /// Unrecoverable failure; abort with explanation.
    Abort(String),
}

/// Replanner configuration.
pub struct Replanner {
    /// Model used for replanning (typically "opus").
    pub coordinator_model: String,
    /// Maximum number of replan iterations for a single run.
    pub max_replans: u8,
}

impl Replanner {
    pub fn new(coordinator_model: impl Into<String>, max_replans: u8) -> Self {
        Self {
            coordinator_model: coordinator_model.into(),
            max_replans,
        }
    }

    /// Evaluate the current state and decide whether to replan.
    ///
    /// **Phase 3 implementation note:** This is a stub. Full implementation
    /// requires: (1) a DSL serializer (CompiledPlan → .gaviero text), (2)
    /// an Opus call with the failure context, (3) parsing the response back
    /// through `gaviero_dsl::compile()`.
    pub async fn evaluate(
        &self,
        _plan: &CompiledPlan,
        state: &ExecutionState,
        failed_ids: &[String],
    ) -> ReplanDecision {
        // Stub: log the failure and continue without replanning.
        // Full implementation will call Opus here.
        tracing::info!(
            "Replanner stub: {} failed nodes, {} total nodes, continuing without replan",
            failed_ids.len(),
            state.node_states.len(),
        );
        ReplanDecision::Continue
    }
}
