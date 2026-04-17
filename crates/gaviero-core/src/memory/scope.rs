//! Scope hierarchy for hierarchical memory.
//!
//! Five levels, from narrowest to widest:
//!
//! ```text
//! global                  personal cross-workspace knowledge
//!   └─ workspace          business-level project (.gaviero-workspace)
//!        └─ repo          single git repository (WorkspaceFolder)
//!             └─ module   crate / package / subdirectory (FileScope.owned_paths)
//!                  └─ run single swarm execution (ephemeral, consolidated upward)
//! ```

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
/// Used by retrieval to determine which scope levels to cascade through.
#[derive(Debug, Clone)]
pub struct MemoryScope {
    /// Global DB path: `~/.config/gaviero/memory.db`.
    pub global_db: PathBuf,
    /// Workspace DB path: `<workspace-root>/.gaviero/memory.db`.
    pub workspace_db: PathBuf,
    /// Hash of canonical workspace path.
    pub workspace_id: String,
    /// Hash of canonical repo root (present when agent has repo context).
    pub repo_id: Option<String>,
    /// Narrowest module prefix derived from FileScope.owned_paths.
    pub module_path: Option<String>,
    /// Present during swarm execution.
    pub run_id: Option<String>,
}

impl MemoryScope {
    /// Build a scope chain from the current execution context.
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

        // Module = longest common prefix of owned_paths
        let module_path = owned_paths
            .and_then(|paths| common_prefix(paths).filter(|p| !p.is_empty() && p != "."));

        Self {
            global_db,
            workspace_db,
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
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Factual => "factual",
            Self::Procedural => "procedural",
            Self::Decision => "decision",
            Self::Pattern => "pattern",
            Self::Gotcha => "gotcha",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "procedural" => Self::Procedural,
            "decision" => Self::Decision,
            "pattern" => Self::Pattern,
            "gotcha" => Self::Gotcha,
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

/// Metadata for a scoped memory write.
#[derive(Debug, Clone)]
pub struct WriteMeta {
    pub memory_type: MemoryType,
    pub importance: f32,
    pub trust: Trust,
    pub source: String,
    pub tag: Option<String>,
}

impl Default for WriteMeta {
    fn default() -> Self {
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.5,
            trust: Trust::Medium,
            source: String::new(),
            tag: None,
        }
    }
}

impl WriteMeta {
    /// Convenience: metadata for an agent observation during a swarm run.
    pub fn agent_observation(agent_id: &str) -> Self {
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.4,
            trust: Trust::Medium,
            source: format!("agent:{agent_id}"),
            tag: None,
        }
    }

    /// Convenience: metadata for a user /remember command.
    pub fn user_remember() -> Self {
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.8,
            trust: Trust::High,
            source: "user:/remember".to_string(),
            tag: None,
        }
    }

    /// Convenience: metadata for consolidation-promoted memories.
    pub fn consolidation(source_run_id: &str) -> Self {
        Self {
            memory_type: MemoryType::Factual,
            importance: 0.5,
            trust: Trust::Low,
            source: format!("consolidation:run:{source_run_id}"),
            tag: None,
        }
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
        ] {
            assert_eq!(MemoryType::parse_str(ty.as_str()), ty);
        }
    }
}
