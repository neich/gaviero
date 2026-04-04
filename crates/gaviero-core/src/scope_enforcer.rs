//! File-scope enforcement and sensitive-path blocking.
//!
//! `ScopeEnforcer` checks every file write/read against:
//! 1. The agent's declared `FileScope` (owned/read-only paths)
//! 2. A hard-coded block-list of sensitive files (credentials, keys, env files)
//!
//! Violations are returned as `ScopeViolation` errors rather than panics so
//! the caller can log them and stop the write without crashing the agent.
//!
//! See Phase 7 of the implementation plan.

use std::path::{Path, PathBuf};

use crate::types::FileScope;

/// Paths that are always blocked regardless of declared scope.
const BLOCKED: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.staging",
    "id_rsa",
    "id_ed25519",
    "id_dsa",
    ".ssh/",
    "credentials",
    ".aws/",
    ".netrc",
    "secrets.toml",
    "secrets.yaml",
    "secrets.json",
];

/// A scope or sensitivity violation detected before a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeViolation {
    pub path: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for ScopeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scope violation for {}: {}", self.path.display(), self.reason)
    }
}

/// Enforces read/write scope for a single agent.
pub struct ScopeEnforcer {
    scope: FileScope,
    workspace_root: PathBuf,
}

impl ScopeEnforcer {
    pub fn new(scope: FileScope, workspace_root: PathBuf) -> Self {
        Self { scope, workspace_root }
    }

    /// Check whether `path` may be written by this agent.
    ///
    /// Blocks if:
    /// - The path is on the hard-coded sensitive block-list
    /// - The path is not covered by `scope.owned_paths`
    pub fn check_write(&self, path: &Path) -> Result<(), ScopeViolation> {
        if Self::is_sensitive(path) {
            return Err(ScopeViolation {
                path: path.to_path_buf(),
                reason: format!(
                    "path matches sensitive file block-list"
                ),
            });
        }

        let path_str = path.to_string_lossy();
        let allowed = self.scope.owned_paths.is_empty()
            || self.scope.owned_paths.iter().any(|owned| {
                path_str.starts_with(owned.as_str()) || path_str == owned.as_str()
            });

        if !allowed {
            return Err(ScopeViolation {
                path: path.to_path_buf(),
                reason: format!(
                    "path is outside agent's owned scope {:?}",
                    self.scope.owned_paths
                ),
            });
        }

        Ok(())
    }

    /// Check whether `path` may be read by this agent.
    ///
    /// Blocks if the path is on the sensitive block-list and is not in
    /// the agent's explicitly declared readable paths.
    pub fn check_read(&self, path: &Path) -> Result<(), ScopeViolation> {
        if !Self::is_sensitive(path) {
            return Ok(());
        }

        let path_str = path.to_string_lossy();
        // Allow if explicitly listed in owned_paths or read_only_paths
        let explicitly_allowed = self.scope.owned_paths.iter()
            .chain(self.scope.read_only_paths.iter())
            .any(|p| path_str.starts_with(p.as_str()) || path_str == p.as_str());

        if explicitly_allowed {
            return Ok(());
        }

        Err(ScopeViolation {
            path: path.to_path_buf(),
            reason: "sensitive file not in declared scope".into(),
        })
    }

    /// Returns `true` if `path` matches any entry on the block-list.
    pub fn is_sensitive(path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        BLOCKED.iter().any(|blocked| {
            path_str == *blocked
                || path_str.ends_with(blocked)
                || path_str.contains(blocked)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_enforcer(owned: &[&str]) -> ScopeEnforcer {
        ScopeEnforcer::new(
            FileScope {
                owned_paths: owned.iter().map(|s| s.to_string()).collect(),
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            PathBuf::from("/workspace"),
        )
    }

    #[test]
    fn write_within_scope_allowed() {
        let e = make_enforcer(&["src/"]);
        assert!(e.check_write(Path::new("src/main.rs")).is_ok());
    }

    #[test]
    fn write_outside_scope_blocked() {
        let e = make_enforcer(&["src/"]);
        assert!(e.check_write(Path::new("tests/foo.rs")).is_err());
    }

    #[test]
    fn write_to_env_blocked() {
        let e = make_enforcer(&["."]);
        assert!(e.check_write(Path::new(".env")).is_err());
    }

    #[test]
    fn write_to_nested_env_blocked() {
        let e = make_enforcer(&["."]);
        assert!(e.check_write(Path::new("config/.env.local")).is_err());
    }

    #[test]
    fn read_sensitive_not_in_scope_blocked() {
        let e = make_enforcer(&["src/"]);
        assert!(e.check_read(Path::new("id_rsa")).is_err());
    }

    #[test]
    fn empty_owned_scope_allows_all_non_sensitive() {
        let e = make_enforcer(&[]);
        assert!(e.check_write(Path::new("src/main.rs")).is_ok());
        assert!(e.check_write(Path::new(".env")).is_err());
    }
}
