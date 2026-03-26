use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::{FileScope, ModelTier, PrivacyLevel};

/// A unit of work for an agent in the swarm.
///
/// All fields except `id` have serde defaults or aliases to tolerate
/// varying JSON shapes from LLM coordinators.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkUnit {
    /// Unique identifier for this work unit.
    #[serde(alias = "name")]
    pub id: String,
    /// Human-readable description of the task.
    #[serde(default, alias = "task", alias = "title", alias = "summary")]
    pub description: String,
    /// File scope defining which paths this agent can write to.
    #[serde(default)]
    pub scope: FileScope,
    /// IDs of work units that must complete before this one starts.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Which agent backend to use.
    #[serde(default)]
    pub backend: AgentBackend,
    /// Per-unit model override. When `Some(_)`, bypasses `TierRouter` for
    /// this unit — the agent runs on the specified model directly.
    /// When `None`, the `TierRouter` resolves the model from `tier`.
    #[serde(default)]
    pub model: Option<String>,

    // ── Tier routing fields ──────────────────────────────────────

    /// Model tier assigned by the coordinator.
    #[serde(default)]
    pub tier: ModelTier,
    /// Privacy classification — routing constraint.
    #[serde(default)]
    pub privacy: PrivacyLevel,
    /// Coordinator's decomposed instructions for this subtask.
    #[serde(default)]
    pub coordinator_instructions: String,
    /// Context budget hint (estimated tokens for this unit).
    #[serde(default)]
    pub estimated_tokens: u32,
    /// Max retries before escalation (default: 1).
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,
    /// Tier to escalate to on failure.
    #[serde(default)]
    pub escalation_tier: Option<ModelTier>,
}

fn default_max_retries() -> u8 {
    1
}

/// The backend used to execute an agent's work.
///
/// **Deprecated:** Use [`super::backend::BackendConfig`] and the [`super::backend::AgentBackend`]
/// trait instead. This enum is retained for serde backward compatibility.
#[deprecated(note = "Use backend::BackendConfig and the AgentBackend trait instead")]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    #[default]
    ClaudeCode,
    Codex,
    GeminiCli,
    Ollama {
        model: String,
        base_url: String,
    },
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
