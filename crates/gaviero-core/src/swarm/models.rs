#![allow(deprecated)]
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
    /// Reasoning/effort level requested by the DSL `client { effort ... }`
    /// field. Provider-neutral string; each backend maps it into its own knob
    /// (Claude `effort`, Codex `model_reasoning_effort`, etc.) and ignores
    /// values it doesn't recognise. `None` means "use the backend's default".
    #[serde(default)]
    pub effort: Option<String>,
    /// Provider-specific key/value pairs from the DSL `client { extra { ... } }`
    /// block. Backends consume keys they understand and ignore the rest.
    /// Order is preserved (the DSL allows duplicate-key detection only at
    /// compile time; by the time a `WorkUnit` is constructed all keys are
    /// unique).
    #[serde(default)]
    pub extra: Vec<(String, String)>,

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

    // ── Memory routing fields (from DSL `memory {}` block) ───────
    /// Memory namespaces to read from for this agent's context.
    /// `None` means inherit `SwarmConfig.read_namespaces`.
    #[serde(default)]
    pub read_namespaces: Option<Vec<String>>,

    /// Memory namespace to write this agent's results into.
    /// `None` means inherit `SwarmConfig.write_namespace`.
    #[serde(default)]
    pub write_namespace: Option<String>,

    /// Importance weight for memories written by this agent (0.0–1.0).
    /// `None` → memory store default (0.5).
    #[serde(default)]
    pub memory_importance: Option<f32>,

    /// Relative paths (from workspace root) whose hashes are checked before
    /// this agent runs; stale entries are invalidated automatically.
    #[serde(default)]
    pub staleness_sources: Vec<String>,

    // ── Explicit memory control (from DSL `memory {}` extensions) ──
    /// Custom semantic search query for memory context.
    /// When `Some`, replaces `description` as the search query in `build_prompt`.
    #[serde(default)]
    pub memory_read_query: Option<String>,

    /// Custom search result limit. `None` → default (5).
    #[serde(default)]
    pub memory_read_limit: Option<usize>,

    /// Template for memory write content. `None` → auto-generated summary.
    /// Supports `{{SUMMARY}}`, `{{FILES}}`, `{{AGENT}}`, `{{DESCRIPTION}}`.
    #[serde(default)]
    pub memory_write_content: Option<String>,

    // ── Graph / impact fields (from DSL `scope` and `context` blocks) ──
    /// When true, automatically expand `read_only_paths` with blast-radius files.
    #[serde(default)]
    pub impact_scope: bool,

    /// Files whose callers should be queried from the code knowledge graph.
    #[serde(default)]
    pub context_callers_of: Vec<String>,

    /// Paths whose test files should be queried from the code knowledge graph.
    #[serde(default)]
    pub context_tests_for: Vec<String>,

    /// BFS depth limit for context graph queries (default: 2).
    #[serde(default = "default_context_depth")]
    pub context_depth: u32,

    /// Extra tools this agent may invoke, beyond the runner's default
    /// read-only set (`Read, Glob, Grep, Write, Edit, MultiEdit`). Names are
    /// forwarded verbatim to the backend via `--allowedTools`.
    ///
    /// Populated from the DSL `agent { tools [...] }` block. Opting into
    /// `Bash` (or any other shell-capable tool) bypasses the write-gate
    /// scope enforcement — only grant it to agents that must run
    /// `cargo check` / `cargo test` and similar.
    #[serde(default)]
    pub extra_allowed_tools: Vec<String>,
}

fn default_context_depth() -> u32 {
    2
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
    /// Optional short summary of changes (for bus broadcasts and UI).
    pub summary: Option<String>,
    /// Full agent text output (assistant messages concatenated).
    /// Used by `{{SUMMARY}}` in memory `write_content` templates.
    #[serde(default)]
    pub output: Option<String>,
    /// Estimated cost of this agent's run in USD (populated when usage data available).
    #[serde(default)]
    pub cost_usd: f64,
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
    /// HEAD SHA captured before any merges. Used by `revert_swarm` to reset
    /// the repo to its pre-swarm state. Empty string for non-worktree runs.
    #[serde(default)]
    pub pre_swarm_sha: String,
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
