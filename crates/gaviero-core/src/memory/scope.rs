//! Scope hierarchy for hierarchical memory.
//!
//! Five levels, from narrowest to widest:
//!
//! ```text
//! global                  personal cross-workspace knowledge
//!   └─ workspace          business-level project (.gaviero-workspace)
//!        └─ repo          a workspace folder (shared across workspaces)
//!             └─ module   crate / package / subdirectory (FileScope.owned_paths)
//!                  └─ run single swarm execution (ephemeral, consolidated upward)
//! ```
//!
//! Physical storage layout (3 SQLite files):
//! - `global` lives in `~/.config/gaviero/memory.db`.
//! - `workspace` and `run` live in `<workspace-root>/.gaviero/memory.db`.
//! - `repo` and `module` live in `<folder-root>/.gaviero/memory.db`.
//!
//! When a workspace is a single directly-opened directory, `workspace_root`
//! and `folder_root` are the same path, so the workspace and folder DBs
//! collapse to one file.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

// ── Scope level constants ─────────────────────────────────────

/// Integer scope level — lower is broader.
pub const SCOPE_GLOBAL: i32 = 0;
pub const SCOPE_WORKSPACE: i32 = 1;
pub const SCOPE_REPO: i32 = 2;
pub const SCOPE_MODULE: i32 = 3;
pub const SCOPE_RUN: i32 = 4;

// ── Hash helper ───────────────────────────────────────────────

/// Deterministic 12-hex-char hash of a canonical path, used as repo/workspace ID.
pub fn hash_path(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    // First 6 bytes = 12 hex chars — enough to avoid collisions in practice.
    digest[..6]
        .iter()
        .fold(String::with_capacity(12), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

// ── StoreKind ─────────────────────────────────────────────────

/// Identifies which physical SQLite file a scope is stored in.
///
/// Routing rules:
/// - `Global` → `~/.config/gaviero/memory.db`
/// - `Workspace` → `<workspace-root>/.gaviero/memory.db` (workspace + run scopes)
/// - `Folder(repo_id)` → `<folder-root>/.gaviero/memory.db` (repo + module scopes)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StoreKind {
    Global,
    Workspace,
    Folder { repo_id: String },
}

/// Derive the [`StoreKind`] that owns a row given its persisted
/// `(scope_level, repo_id)`. Mirrors the routing in
/// [`WriteScope::target_store`] / [`ScopeFilter::target_store`] so the
/// retrieval merge can route per-row access logging back to the right
/// store.
///
/// Returns `None` when the inputs are inconsistent (e.g., a repo /
/// module / run row without a `repo_id`).
pub fn store_kind_for_scope(scope_level: i32, repo_id: Option<&str>) -> Option<StoreKind> {
    match scope_level {
        SCOPE_GLOBAL => Some(StoreKind::Global),
        SCOPE_WORKSPACE => Some(StoreKind::Workspace),
        SCOPE_RUN => Some(StoreKind::Workspace),
        SCOPE_REPO | SCOPE_MODULE => repo_id.map(|r| StoreKind::Folder {
            repo_id: r.to_string(),
        }),
        _ => None,
    }
}

// ── ScopeFilter ───────────────────────────────────────────────

/// A filter for a single scope level, used during cascading search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeFilter {
    Global,
    Workspace,
    Repo {
        repo_id: String,
    },
    Module {
        repo_id: String,
        module_path: String,
    },
    Run {
        repo_id: String,
        run_id: String,
    },
}

impl ScopeFilter {
    /// Integer scope level for this filter.
    pub fn level_int(&self) -> i32 {
        match self {
            Self::Global => SCOPE_GLOBAL,
            Self::Workspace => SCOPE_WORKSPACE,
            Self::Repo { .. } => SCOPE_REPO,
            Self::Module { .. } => SCOPE_MODULE,
            Self::Run { .. } => SCOPE_RUN,
        }
    }

    /// The `repo_id` for this filter, if any.
    pub fn repo_id(&self) -> Option<&str> {
        match self {
            Self::Repo { repo_id } | Self::Module { repo_id, .. } | Self::Run { repo_id, .. } => {
                Some(repo_id)
            }
            _ => None,
        }
    }

    /// The `module_path` for this filter, if any.
    pub fn module_path(&self) -> Option<&str> {
        match self {
            Self::Module { module_path, .. } => Some(module_path),
            _ => None,
        }
    }

    /// The `run_id` for this filter, if any.
    pub fn run_id(&self) -> Option<&str> {
        match self {
            Self::Run { run_id, .. } => Some(run_id),
            _ => None,
        }
    }

    /// Which physical store this scope reads from.
    ///
    /// Run-scope reads go to the workspace store (run rows live there);
    /// repo and module reads go to the folder store keyed by `repo_id`.
    pub fn target_store(&self) -> StoreKind {
        match self {
            Self::Global => StoreKind::Global,
            Self::Workspace => StoreKind::Workspace,
            Self::Run { .. } => StoreKind::Workspace,
            Self::Repo { repo_id } | Self::Module { repo_id, .. } => StoreKind::Folder {
                repo_id: repo_id.clone(),
            },
        }
    }
}

// ── WriteScope ────────────────────────────────────────────────

/// Declares the target scope level for a memory write.
///
/// Every write must specify a scope — the system never guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteScope {
    /// Agent writing run-local observations.
    Run { repo_id: String, run_id: String },
    /// Agent /remember or consolidation promoting to module.
    Module {
        repo_id: String,
        module_path: String,
    },
    /// Cross-module repo knowledge.
    Repo { repo_id: String },
    /// Cross-repo workspace knowledge.
    Workspace,
    /// User's personal global knowledge.
    Global,
}

impl WriteScope {
    /// Integer scope level.
    pub fn level_int(&self) -> i32 {
        match self {
            Self::Global => SCOPE_GLOBAL,
            Self::Workspace => SCOPE_WORKSPACE,
            Self::Repo { .. } => SCOPE_REPO,
            Self::Module { .. } => SCOPE_MODULE,
            Self::Run { .. } => SCOPE_RUN,
        }
    }

    /// Canonical string representation of the scope path.
    ///
    /// Used as the `scope_path` column value and for dedup checks.
    pub fn to_path_string(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Workspace => "workspace".to_string(),
            Self::Repo { repo_id } => format!("repo:{repo_id}"),
            Self::Module {
                repo_id,
                module_path,
            } => {
                format!("repo:{repo_id}/module:{module_path}")
            }
            Self::Run { repo_id, run_id } => {
                format!("repo:{repo_id}/run:{run_id}")
            }
        }
    }

    /// The `repo_id` embedded in this scope, if any.
    pub fn repo_id(&self) -> Option<&str> {
        match self {
            Self::Repo { repo_id } | Self::Module { repo_id, .. } | Self::Run { repo_id, .. } => {
                Some(repo_id)
            }
            _ => None,
        }
    }

    /// The `module_path` embedded in this scope, if any.
    pub fn module_path(&self) -> Option<&str> {
        match self {
            Self::Module { module_path, .. } => Some(module_path),
            _ => None,
        }
    }

    /// Which physical store this write targets.
    ///
    /// Run-scope writes go to the workspace store (run rows live there);
    /// repo and module writes go to the folder store keyed by `repo_id`.
    pub fn target_store(&self) -> StoreKind {
        match self {
            Self::Global => StoreKind::Global,
            Self::Workspace => StoreKind::Workspace,
            Self::Run { .. } => StoreKind::Workspace,
            Self::Repo { repo_id } | Self::Module { repo_id, .. } => StoreKind::Folder {
                repo_id: repo_id.clone(),
            },
        }
    }

    /// Broader (ancestor) scopes for cross-scope dedup, ordered
    /// narrowest → widest. Each tuple is `(level, scope_path,
    /// target_store)`. Used by the registry to probe other physical
    /// DBs for content-hash coverage before performing the target
    /// write.
    pub fn ancestors(&self) -> Vec<(i32, String, StoreKind)> {
        match self {
            Self::Global => Vec::new(),
            Self::Workspace => vec![(SCOPE_GLOBAL, "global".to_string(), StoreKind::Global)],
            Self::Repo { .. } => vec![
                (
                    SCOPE_WORKSPACE,
                    "workspace".to_string(),
                    StoreKind::Workspace,
                ),
                (SCOPE_GLOBAL, "global".to_string(), StoreKind::Global),
            ],
            Self::Module { repo_id, .. } | Self::Run { repo_id, .. } => vec![
                (
                    SCOPE_REPO,
                    format!("repo:{repo_id}"),
                    StoreKind::Folder {
                        repo_id: repo_id.clone(),
                    },
                ),
                (
                    SCOPE_WORKSPACE,
                    "workspace".to_string(),
                    StoreKind::Workspace,
                ),
                (SCOPE_GLOBAL, "global".to_string(), StoreKind::Global),
            ],
        }
    }

    /// Convert to a `ScopeFilter` for use in search.
    pub fn to_filter(&self) -> ScopeFilter {
        match self {
            Self::Global => ScopeFilter::Global,
            Self::Workspace => ScopeFilter::Workspace,
            Self::Repo { repo_id } => ScopeFilter::Repo {
                repo_id: repo_id.clone(),
            },
            Self::Module {
                repo_id,
                module_path,
            } => ScopeFilter::Module {
                repo_id: repo_id.clone(),
                module_path: module_path.clone(),
            },
            Self::Run { repo_id, run_id } => ScopeFilter::Run {
                repo_id: repo_id.clone(),
                run_id: run_id.clone(),
            },
        }
    }
}

// ── MemoryScope ───────────────────────────────────────────────

/// Resolved scope chain for the current execution context.
///
/// Built from Workspace + WorkspaceFolder + FileScope + optional run_id.
/// Used by retrieval to determine which scope levels to cascade through,
/// and by the [`super::MemoryStores`] registry to pick which physical
/// SQLite file each scope level lives in.
#[derive(Debug, Clone)]
pub struct MemoryScope {
    /// Global DB path: `~/.config/gaviero/memory.db`.
    pub global_db: PathBuf,
    /// Workspace DB path: `<workspace-root>/.gaviero/memory.db`.
    /// Holds workspace and run scopes.
    pub workspace_db: PathBuf,
    /// Folder (repo) DB path: `<folder-root>/.gaviero/memory.db`.
    /// Holds repo and module scopes. `None` when the agent has no folder context.
    pub repo_db: Option<PathBuf>,
    /// Hash of canonical workspace path.
    pub workspace_id: String,
    /// Hash of canonical folder root (present when agent has repo context).
    pub repo_id: Option<String>,
    /// Narrowest module prefix derived from FileScope.owned_paths.
    pub module_path: Option<String>,
    /// Present during swarm execution.
    pub run_id: Option<String>,
}

impl MemoryScope {
    /// Build a scope chain from the current execution context.
    ///
    /// `folder` is the workspace folder root (which we treat as the "repo"
    /// — folder identity is what matters, no `.git` requirement). When
    /// `folder == workspace_root`, the workspace and folder DBs resolve
    /// to the same physical file.
    pub fn from_context(
        workspace_root: &Path,
        folder: Option<&Path>,
        owned_paths: Option<&[String]>,
        run_id: Option<&str>,
    ) -> Self {
        let global_db = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("gaviero/memory.db");

        let workspace_db = workspace_root.join(".gaviero/memory.db");
        let workspace_id = hash_path(workspace_root);

        let repo_id = folder.map(|f| hash_path(f));
        let repo_db = folder.map(|f| f.join(".gaviero/memory.db"));

        // Module = longest common prefix of owned_paths
        let module_path = owned_paths
            .and_then(|paths| common_prefix(paths).filter(|p| !p.is_empty() && p != "."));

        Self {
            global_db,
            workspace_db,
            repo_db,
            workspace_id,
            repo_id,
            module_path,
            run_id: run_id.map(String::from),
        }
    }

    /// Scope levels from narrowest to widest, for cascading search.
    pub fn levels(&self) -> Vec<ScopeFilter> {
        let mut levels = Vec::with_capacity(5);

        if let Some(run_id) = &self.run_id {
            if let Some(repo_id) = &self.repo_id {
                levels.push(ScopeFilter::Run {
                    repo_id: repo_id.clone(),
                    run_id: run_id.clone(),
                });
            }
        }
        if let Some(module_path) = &self.module_path {
            if let Some(repo_id) = &self.repo_id {
                levels.push(ScopeFilter::Module {
                    repo_id: repo_id.clone(),
                    module_path: module_path.clone(),
                });
            }
        }
        if let Some(repo_id) = &self.repo_id {
            levels.push(ScopeFilter::Repo {
                repo_id: repo_id.clone(),
            });
        }
        levels.push(ScopeFilter::Workspace);
        levels.push(ScopeFilter::Global);

        levels
    }

    /// Default write scope: the narrowest available persistent level.
    ///
    /// Run scope is excluded because it's ephemeral — callers creating
    /// run-level memories should pass `WriteScope::Run` explicitly.
    pub fn default_write_scope(&self) -> WriteScope {
        if let Some(repo_id) = &self.repo_id {
            if let Some(module_path) = &self.module_path {
                return WriteScope::Module {
                    repo_id: repo_id.clone(),
                    module_path: module_path.clone(),
                };
            }
            return WriteScope::Repo {
                repo_id: repo_id.clone(),
            };
        }
        WriteScope::Workspace
    }
}

// ── Helpers ───────────────────────────────────────────────────

/// Longest common prefix of a set of paths.
fn common_prefix(paths: &[String]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        // Single path: use its directory component
        let p = paths[0].trim_end_matches('/');
        return if p.contains('/') {
            Some(p.rsplit_once('/').unwrap().0.to_string())
        } else {
            Some(p.to_string())
        };
    }

    let mut prefix = paths[0].clone();
    for path in &paths[1..] {
        while !path.starts_with(&prefix) {
            if let Some(pos) = prefix.rfind('/') {
                prefix.truncate(pos);
            } else {
                return None;
            }
        }
    }

    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

// ── Trust level ───────────────────────────────────────────────

/// Trust level for a memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// Human-authored via /remember.
    High,
    /// LLM-extracted by agent.
    Medium,
    /// Inferred or consolidated.
    Low,
}

impl Trust {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "high" => Self::High,
            "low" => Self::Low,
            _ => Self::Medium,
        }
    }

    pub fn weight(&self) -> f32 {
        match self {
            Self::High => 1.2,
            Self::Medium => 1.0,
            Self::Low => 0.7,
        }
    }
}

use serde::{Deserialize, Serialize};

/// Memory type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Factual,
    Procedural,
    Decision,
    Pattern,
    Gotcha,
    Convention,
    Invariant,
    Preference,
    Lesson,
    Error,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Factual => "factual",
            Self::Procedural => "procedural",
            Self::Decision => "decision",
            Self::Pattern => "pattern",
            Self::Gotcha => "gotcha",
            Self::Convention => "convention",
            Self::Invariant => "invariant",
            Self::Preference => "preference",
            Self::Lesson => "lesson",
            Self::Error => "error",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "procedural" => Self::Procedural,
            "decision" => Self::Decision,
            "pattern" => Self::Pattern,
            "gotcha" => Self::Gotcha,
            "convention" => Self::Convention,
            "invariant" => Self::Invariant,
            "preference" => Self::Preference,
            "lesson" => Self::Lesson,
            "error" => Self::Error,
            _ => Self::Factual,
        }
    }
}

// ── StoreResult ───────────────────────────────────────────────

/// Outcome of a scoped store operation.
#[derive(Debug, Clone)]
pub enum StoreResult {
    /// New memory inserted.
    Inserted(i64),
    /// Existing memory at the same scope was reinforced.
    Deduplicated(i64),
    /// Content already exists at a broader scope — write skipped.
    AlreadyCovered,
}

// ── WriteMeta ─────────────────────────────────────────────────

use super::trust_defaults::{MemorySource, clamp_trust};

/// Metadata for a scoped memory write.
///
/// Tier A3 added `source_kind: MemorySource` (typed write origin) and
/// `trust_score: f32` (the [0.0, 1.0] retrieval multiplier). The legacy
/// `source: String` + `trust: Trust` fields are retained as mirrors so
/// pre-A3 call sites keep compiling; new code should prefer
/// [`WriteMeta::for_source`]. Inserts read the typed fields — the legacy
/// fields are written to their columns for backward compat only.
#[derive(Debug, Clone)]
pub struct WriteMeta {
    pub memory_type: MemoryType,
    pub importance: f32,
    pub trust: Trust,
    pub source: String,
    pub tag: Option<String>,
    /// Typed write-origin tag. Drives default `trust_score` and feeds
    /// the A3 `source` column.
    pub source_kind: MemorySource,
    /// Retrieval trust multiplier in [0.0, 1.0]. Defaults to
    /// `source_kind.default_trust()` via [`WriteMeta::for_source`].
    pub trust_score: f32,
    /// Tier C / C1: lifecycle class. Defaults to `Record` so legacy
    /// callers don't need updating; producers of immutable transcripts
    /// (`process_turn_complete`'s history row) and consolidator
    /// summaries (`process_session_consolidate`) override via
    /// [`WriteMeta::with_kind`].
    pub kind: super::kind::MemoryKind,
}

impl Default for WriteMeta {
    fn default() -> Self {
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.5,
            trust: Trust::Medium,
            source: String::new(),
            tag: None,
            source_kind: MemorySource::UnknownLegacy,
            trust_score: MemorySource::UnknownLegacy.default_trust(),
            kind: super::kind::MemoryKind::Record,
        }
    }
}

impl WriteMeta {
    /// Canonical constructor for A3+ callers. Sets both the typed
    /// `source_kind` and the default `trust_score` for that source;
    /// also mirrors the legacy `source: String` + `trust: Trust` fields
    /// so the row's old columns stay populated.
    pub fn for_source(source_kind: MemorySource) -> Self {
        let trust_score = source_kind.default_trust();
        let trust = if trust_score >= 0.9 {
            Trust::High
        } else if trust_score >= 0.65 {
            Trust::Medium
        } else {
            Trust::Low
        };
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.5,
            trust,
            source: source_kind.as_str().to_string(),
            tag: None,
            source_kind,
            trust_score,
            kind: super::kind::MemoryKind::Record,
        }
    }

    /// Tier C / C1: override the lifecycle class. Defaults to
    /// `Record`. History writes (immutable raw transcripts) and
    /// Summary writes (session consolidator output) override here.
    pub fn with_kind(mut self, kind: super::kind::MemoryKind) -> Self {
        self.kind = kind;
        self
    }

    /// Override the default trust score (clamped to [0.0, 1.0]).
    pub fn with_trust_score(mut self, t: f32) -> Self {
        self.trust_score = clamp_trust(t);
        self
    }

    /// Override importance (clamped to [0.0, 1.0]).
    pub fn with_importance(mut self, i: f32) -> Self {
        self.importance = clamp_trust(i);
        self
    }

    /// Override memory type.
    pub fn with_type(mut self, t: MemoryType) -> Self {
        self.memory_type = t;
        self
    }

    /// Override tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Convenience: metadata for an agent observation during a swarm run.
    pub fn agent_observation(agent_id: &str) -> Self {
        Self::for_source(MemorySource::SwarmConsolidated)
            .with_importance(0.4)
            .with_tag(format!("agent:{agent_id}"))
    }

    /// Convenience: metadata for a user /remember command.
    pub fn user_remember() -> Self {
        Self::for_source(MemorySource::UserRemember).with_importance(0.8)
    }

    /// Convenience: metadata for consolidation-promoted memories.
    pub fn consolidation(source_run_id: &str) -> Self {
        Self::for_source(MemorySource::LlmConsolidated)
            .with_importance(0.5)
            .with_tag(format!("consolidation:run:{source_run_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_path_deterministic() {
        let p = Path::new("/tmp/test-workspace");
        let h1 = hash_path(p);
        let h2 = hash_path(p);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 12); // 6 bytes = 12 hex chars
    }

    #[test]
    fn test_write_scope_path_strings() {
        assert_eq!(WriteScope::Global.to_path_string(), "global");
        assert_eq!(WriteScope::Workspace.to_path_string(), "workspace");
        assert_eq!(
            WriteScope::Repo {
                repo_id: "abc".into()
            }
            .to_path_string(),
            "repo:abc"
        );
        assert_eq!(
            WriteScope::Module {
                repo_id: "abc".into(),
                module_path: "crates/core".into(),
            }
            .to_path_string(),
            "repo:abc/module:crates/core"
        );
        assert_eq!(
            WriteScope::Run {
                repo_id: "abc".into(),
                run_id: "plan123".into(),
            }
            .to_path_string(),
            "repo:abc/run:plan123"
        );
    }

    #[test]
    fn test_write_scope_level_int() {
        assert_eq!(WriteScope::Global.level_int(), 0);
        assert_eq!(WriteScope::Workspace.level_int(), 1);
        assert_eq!(
            WriteScope::Repo {
                repo_id: "x".into()
            }
            .level_int(),
            2
        );
        assert_eq!(
            WriteScope::Module {
                repo_id: "x".into(),
                module_path: "m".into()
            }
            .level_int(),
            3
        );
        assert_eq!(
            WriteScope::Run {
                repo_id: "x".into(),
                run_id: "r".into()
            }
            .level_int(),
            4
        );
    }

    #[test]
    fn test_scope_filter_level_int() {
        assert_eq!(ScopeFilter::Global.level_int(), 0);
        assert_eq!(ScopeFilter::Workspace.level_int(), 1);
        assert_eq!(
            ScopeFilter::Repo {
                repo_id: "x".into()
            }
            .level_int(),
            2
        );
        assert_eq!(
            ScopeFilter::Module {
                repo_id: "x".into(),
                module_path: "m".into()
            }
            .level_int(),
            3
        );
        assert_eq!(
            ScopeFilter::Run {
                repo_id: "x".into(),
                run_id: "r".into()
            }
            .level_int(),
            4
        );
    }

    #[test]
    fn test_common_prefix() {
        assert_eq!(
            common_prefix(&["src/auth/login.rs".into(), "src/auth/session.rs".into()]),
            Some("src/auth".into())
        );
        assert_eq!(
            common_prefix(&["src/auth/".into(), "src/db/".into()]),
            Some("src".into())
        );
        assert_eq!(
            common_prefix(&["crates/core".into()]),
            Some("crates".into())
        );
        assert_eq!(common_prefix(&[]), None);
        assert_eq!(common_prefix(&["a/b".into(), "c/d".into()]), None);
    }

    #[test]
    fn test_memory_scope_levels() {
        let scope = MemoryScope {
            global_db: PathBuf::from("/tmp/global.db"),
            workspace_db: PathBuf::from("/tmp/ws.db"),
            repo_db: Some(PathBuf::from("/tmp/folder.db")),
            workspace_id: "ws1".into(),
            repo_id: Some("repo1".into()),
            module_path: Some("crates/core".into()),
            run_id: Some("run1".into()),
        };

        let levels = scope.levels();
        assert_eq!(levels.len(), 5);
        assert_eq!(levels[0].level_int(), SCOPE_RUN);
        assert_eq!(levels[1].level_int(), SCOPE_MODULE);
        assert_eq!(levels[2].level_int(), SCOPE_REPO);
        assert_eq!(levels[3].level_int(), SCOPE_WORKSPACE);
        assert_eq!(levels[4].level_int(), SCOPE_GLOBAL);
    }

    #[test]
    fn test_memory_scope_levels_no_run() {
        let scope = MemoryScope {
            global_db: PathBuf::from("/tmp/global.db"),
            workspace_db: PathBuf::from("/tmp/ws.db"),
            repo_db: Some(PathBuf::from("/tmp/folder.db")),
            workspace_id: "ws1".into(),
            repo_id: Some("repo1".into()),
            module_path: None,
            run_id: None,
        };

        let levels = scope.levels();
        assert_eq!(levels.len(), 3); // repo, workspace, global
        assert_eq!(levels[0].level_int(), SCOPE_REPO);
    }

    #[test]
    fn test_default_write_scope() {
        let scope = MemoryScope {
            global_db: PathBuf::from("/tmp/global.db"),
            workspace_db: PathBuf::from("/tmp/ws.db"),
            repo_db: Some(PathBuf::from("/tmp/folder.db")),
            workspace_id: "ws1".into(),
            repo_id: Some("repo1".into()),
            module_path: Some("crates/core".into()),
            run_id: Some("run1".into()),
        };
        // Default excludes run (ephemeral), picks module as narrowest persistent
        match scope.default_write_scope() {
            WriteScope::Module {
                repo_id,
                module_path,
            } => {
                assert_eq!(repo_id, "repo1");
                assert_eq!(module_path, "crates/core");
            }
            other => panic!("expected Module, got {:?}", other),
        }
    }

    #[test]
    fn test_from_context_distinct_workspace_and_folder() {
        let workspace = Path::new("/tmp/ws");
        let folder = Path::new("/tmp/lib-repo");
        let scope = MemoryScope::from_context(workspace, Some(folder), None, None);
        assert_eq!(scope.workspace_db, workspace.join(".gaviero/memory.db"));
        assert_eq!(
            scope.repo_db.as_deref(),
            Some(folder.join(".gaviero/memory.db").as_path())
        );
        assert_eq!(scope.workspace_id, hash_path(workspace));
        assert_eq!(scope.repo_id.as_deref(), Some(hash_path(folder).as_str()));
    }

    #[test]
    fn test_from_context_collapsed_single_folder() {
        // When workspace_root == folder_root, the workspace and folder DBs
        // resolve to the same physical file.
        let root = Path::new("/tmp/single-open");
        let scope = MemoryScope::from_context(root, Some(root), None, None);
        assert_eq!(scope.repo_db.as_deref(), Some(scope.workspace_db.as_path()));
    }

    #[test]
    fn test_target_store_routing() {
        assert_eq!(WriteScope::Global.target_store(), StoreKind::Global);
        assert_eq!(WriteScope::Workspace.target_store(), StoreKind::Workspace);
        // Run rows live in the workspace DB.
        assert_eq!(
            WriteScope::Run {
                repo_id: "abc".into(),
                run_id: "r1".into(),
            }
            .target_store(),
            StoreKind::Workspace
        );
        // Repo / module rows live in the folder DB, keyed by repo_id.
        assert_eq!(
            WriteScope::Repo {
                repo_id: "abc".into()
            }
            .target_store(),
            StoreKind::Folder {
                repo_id: "abc".into()
            }
        );
        assert_eq!(
            WriteScope::Module {
                repo_id: "abc".into(),
                module_path: "crates/core".into(),
            }
            .target_store(),
            StoreKind::Folder {
                repo_id: "abc".into()
            }
        );
    }

    #[test]
    fn test_scope_filter_target_store_matches_write_scope() {
        let cases = [
            (WriteScope::Global, ScopeFilter::Global),
            (WriteScope::Workspace, ScopeFilter::Workspace),
            (
                WriteScope::Repo {
                    repo_id: "r".into(),
                },
                ScopeFilter::Repo {
                    repo_id: "r".into(),
                },
            ),
            (
                WriteScope::Module {
                    repo_id: "r".into(),
                    module_path: "m".into(),
                },
                ScopeFilter::Module {
                    repo_id: "r".into(),
                    module_path: "m".into(),
                },
            ),
            (
                WriteScope::Run {
                    repo_id: "r".into(),
                    run_id: "x".into(),
                },
                ScopeFilter::Run {
                    repo_id: "r".into(),
                    run_id: "x".into(),
                },
            ),
        ];
        for (ws, sf) in cases {
            assert_eq!(ws.target_store(), sf.target_store());
        }
    }

    #[test]
    fn test_trust_roundtrip() {
        assert_eq!(Trust::parse_str("high"), Trust::High);
        assert_eq!(Trust::parse_str("medium"), Trust::Medium);
        assert_eq!(Trust::parse_str("low"), Trust::Low);
        assert_eq!(Trust::parse_str("unknown"), Trust::Medium);
    }

    #[test]
    fn test_memory_type_roundtrip() {
        for ty in [
            MemoryType::Factual,
            MemoryType::Procedural,
            MemoryType::Decision,
            MemoryType::Pattern,
            MemoryType::Gotcha,
            MemoryType::Convention,
            MemoryType::Invariant,
            MemoryType::Preference,
            MemoryType::Lesson,
            MemoryType::Error,
        ] {
            assert_eq!(MemoryType::parse_str(ty.as_str()), ty);
        }
    }
}
