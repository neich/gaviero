use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::FileScope;

/// A unit of work for an agent in the swarm.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkUnit {
    /// Unique identifier for this work unit.
    pub id: String,
    /// Human-readable description of the task.
    pub description: String,
    /// File scope defining which paths this agent can write to.
    pub scope: FileScope,
    /// IDs of work units that must complete before this one starts.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Which agent backend to use.
    #[serde(default)]
    pub backend: AgentBackend,
    /// Optional model override (e.g. "opus", "sonnet").
    pub model: Option<String>,
}

/// The backend used to execute an agent's work.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    #[default]
    ClaudeCode,
    Codex,
    Custom(String),
}

/// Runtime status of an agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

/// Manifest produced by an agent after completing its work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Which work unit this manifest is for.
    pub work_unit_id: String,
    /// Final status.
    pub status: AgentStatus,
    /// Files that were modified.
    pub modified_files: Vec<PathBuf>,
    /// Optional branch name (used in M3b with git worktrees).
    pub branch: Option<String>,
    /// Optional summary of changes.
    pub summary: Option<String>,
}

/// Overall result of a swarm execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwarmResult {
    /// Manifests from all agents, in execution order.
    pub manifests: Vec<AgentManifest>,
    /// Merge results (populated in M3b).
    pub merge_results: Vec<MergeResult>,
    /// Whether the overall execution succeeded.
    pub success: bool,
}

/// Result of merging an agent's changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeResult {
    pub work_unit_id: String,
    pub success: bool,
    pub conflicts: Vec<MergeConflict>,
}

/// A merge conflict between agent branches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeConflict {
    pub file: PathBuf,
    pub resolved: bool,
    pub resolution_method: Option<String>,
}
