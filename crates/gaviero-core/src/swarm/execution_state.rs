//! Runtime execution state for a `CompiledPlan`.
//!
//! `ExecutionState` is the mutable counterpart to the immutable `CompiledPlan`.
//! It tracks per-node progress (status, attempt count, cost, timing) and
//! can be serialized to disk for checkpoint/resume support.
//!
//! ## Checkpoint format
//! State is saved to `.gaviero/state/{plan_hash}.json` after each node
//! completes. On `--resume`, the saved state is loaded and completed nodes
//! are skipped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::models::AgentManifest;
use super::plan::CompiledPlan;

// ── Node status ──────────────────────────────────────────────

/// Lifecycle state of a single plan node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NodeStatus {
    /// Not yet started; waiting for dependencies.
    #[default]
    Pending,
    /// Dependencies incomplete (at least one is still running/pending).
    Blocked,
    /// All dependencies satisfied; waiting for a semaphore permit.
    Ready,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished but with validation warnings (output exists, flagged).
    SoftFailure,
    /// Finished with an unrecoverable error or exhausted all retries.
    HardFailure,
    /// Skipped because its `TriggerRule` conditions were not met.
    Skipped,
}

impl NodeStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::SoftFailure | Self::HardFailure | Self::Skipped
        )
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed | Self::SoftFailure)
    }
}

// ── Per-node state ───────────────────────────────────────────

/// Runtime state for a single work unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    pub status: NodeStatus,
    /// Number of attempts made (0 = not started, 1 = first attempt complete).
    pub attempt: u8,
    /// The manifest produced by the last agent run, if any.
    pub result: Option<AgentManifest>,
    /// Validation error messages accumulated across attempts.
    pub validation_issues: Vec<String>,
    /// Estimated cost in USD for this node.
    pub cost_usd: f64,
    // Timing — skipped during serialization (Instant is not serializable)
    #[serde(skip)]
    pub started_at: Option<Instant>,
    #[serde(skip)]
    pub completed_at: Option<Instant>,
}

impl Default for NodeState {
    fn default() -> Self {
        Self {
            status: NodeStatus::Pending,
            attempt: 0,
            result: None,
            validation_issues: Vec::new(),
            cost_usd: 0.0,
            started_at: None,
            completed_at: None,
        }
    }
}

// ── ExecutionState ───────────────────────────────────────────

/// Mutable runtime state for an entire plan execution.
///
/// Keys are `work_unit.id` strings (not `NodeIndex`) so the state is stable
/// across serialization/deserialization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionState {
    /// Per-node state, keyed by `work_unit.id`.
    pub node_states: HashMap<String, NodeState>,
    /// Accumulated cost estimate across all nodes.
    pub cost_estimate_usd: f64,
}

impl ExecutionState {
    /// Create a fresh state for the given plan (all nodes `Pending`).
    pub fn new_from_plan(plan: &CompiledPlan) -> Self {
        let node_states = plan
            .graph
            .node_weights()
            .map(|n| (n.work_unit.id.clone(), NodeState::default()))
            .collect();
        Self {
            node_states,
            cost_estimate_usd: 0.0,
        }
    }

    /// Returns `true` when every node is in a terminal state.
    pub fn all_terminal(&self) -> bool {
        self.node_states.values().all(|s| s.status.is_terminal())
    }

    /// Returns the status of a node by work-unit id.
    pub fn status(&self, id: &str) -> NodeStatus {
        self.node_states
            .get(id)
            .map(|s| s.status.clone())
            .unwrap_or(NodeStatus::Pending)
    }

    /// Update node status.
    pub fn set_status(&mut self, id: &str, status: NodeStatus) {
        self.node_states.entry(id.to_string()).or_default().status = status;
    }

    /// Record the result of a completed agent run.
    ///
    /// Cost is read from `manifest.cost_usd` — no separate parameter needed.
    pub fn record_result(&mut self, id: &str, manifest: AgentManifest) {
        let state = self.node_states.entry(id.to_string()).or_default();
        state.attempt += 1;
        let cost = manifest.cost_usd;
        state.cost_usd += cost;
        state.completed_at = Some(Instant::now());

        let status = match &manifest.status {
            super::models::AgentStatus::Completed => NodeStatus::Completed,
            super::models::AgentStatus::Failed(_) => NodeStatus::HardFailure,
            _ => NodeStatus::HardFailure,
        };
        state.status = status;
        state.result = Some(manifest);
        self.cost_estimate_usd += cost;
    }

    // ── Checkpoint I/O ───────────────────────────────────────

    /// Path to the checkpoint file for a given plan hash.
    pub fn checkpoint_path(plan_hash: &str) -> PathBuf {
        PathBuf::from(".gaviero").join("state").join(format!("{}.json", plan_hash))
    }

    /// Serialize state to `.gaviero/state/{hash}.json`.
    pub fn save(&self, plan_hash: &str) -> anyhow::Result<()> {
        let path = Self::checkpoint_path(plan_hash);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Load state from `.gaviero/state/{hash}.json`.
    /// Returns `Ok(None)` if the file does not exist.
    pub fn load(plan_hash: &str) -> anyhow::Result<Option<Self>> {
        let path = Self::checkpoint_path(plan_hash);
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(&path)?;
        let state = serde_json::from_str(&json)?;
        Ok(Some(state))
    }
}
