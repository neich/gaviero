//! File-scope enforcement and sensitive-path blocking.
//!
//! Two independent rails, deliberately not nested:
//!
//! 1. **The sensitive-path block-list** — a hard rule. It does not consult the
//!    declared scope, because `owned_paths` is a restriction list an agent may
//!    legitimately widen to `.`, and a rail reachable only through an optional
//!    check is not a rail. The single escape is
//!    `agent.permissions.sensitivePaths.allow` in `.gaviero/settings.json`
//!    ([`SensitivePolicy`]), which the *user* authors, not the agent.
//! 2. **The declared [`FileScope`]** — a coordination boundary between swarm
//!    work units, not a security boundary.
//!
//! Both rails are enforced at the Write Gate's `propose_write` /
//! `propose_delete` entry points (`acp::client`, `swarm::backend::runner`) and
//! in the in-process tool agent (`agent_session::tool_agent::tools`).
//!
//! Violations are returned as `ScopeViolation` errors rather than panics so
//! the caller can log them and stop the write without crashing the agent.

use std::path::Path;
use std::path::PathBuf;

use crate::types::FileScope;

/// Path *components* that are always blocked, regardless of declared scope.
///
/// Matching is per-component and ASCII-case-insensitive, never substring:
/// `src/credentials_test.rs` is a source file, `.aws/credentials` is a
/// credential, and Windows resolves `ID_RSA` and `id_rsa` to the same inode.
/// Directory names (`.ssh`, `.aws`) are listed bare — a match on any component
/// blocks the whole subtree.
const BLOCKED: &[&str] = &[
    ".env",
    "id_rsa",
    "id_ed25519",
    "id_dsa",
    ".ssh",
    "credentials",
    ".aws",
    ".netrc",
    "secrets.toml",
    "secrets.yaml",
    "secrets.json",
];

/// `.env.<suffix>` files that are conventionally committed templates rather
/// than secrets. Everything else in the `.env.*` family is blocked.
const ENV_TEMPLATE_SUFFIXES: &[&str] = &[".example", ".sample", ".template", ".dist"];

/// Settings key holding the user's block-list exemptions.
const ALLOW_SETTINGS_KEY: &str = "agent.permissions.sensitivePaths.allow";

/// A scope or sensitivity violation detected before a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeViolation {
    pub path: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for ScopeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scope violation for {}: {}",
            self.path.display(),
            self.reason
        )
    }
}

/// Normalize a path for component comparison: `\` → `/`, drop empty and `.`
/// segments. Returns the segments in order.
fn components(path: &Path) -> Vec<String> {
    path.to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(|c| c.to_string())
        .collect()
}

/// Path rendered with `/` separators, for glob matching against the
/// user's exemption patterns.
fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The block-list entry `component` matches, if any.
fn blocked_component(component: &str) -> Option<&'static str> {
    if let Some(entry) = BLOCKED
        .iter()
        .copied()
        .find(|blocked| component.eq_ignore_ascii_case(blocked))
    {
        return Some(entry);
    }

    // `.env.local`, `.env.production`, `.env.whatever-the-user-invented` —
    // the whole family, minus the checked-in templates.
    let lower = component.to_ascii_lowercase();
    if lower.starts_with(".env.")
        && !ENV_TEMPLATE_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix))
    {
        return Some(".env.*");
    }

    None
}

/// The block-list entry `path` matches, if any. Named so callers can put the
/// specific entry in the refusal message instead of "sensitive file".
pub fn sensitive_match(path: &Path) -> Option<&'static str> {
    components(path)
        .iter()
        .find_map(|component| blocked_component(component))
}

/// Workspace-level exemptions from the sensitive block-list.
///
/// Read from `agent.permissions.sensitivePaths.allow` in
/// `<workspace>/.gaviero/settings.json` — a list of [`crate::path_pattern`]
/// globs. Empty (the default) makes the block-list absolute.
///
/// The escape exists because a repo may legitimately own a `secrets.*` or
/// `credentials` fixture; without it, the first CI run touching a test corpus
/// fails opaquely. It is a *settings* key rather than a DSL `scope {}` clause
/// on purpose: the agent authoring the plan must not be able to widen its own
/// access to secrets.
#[derive(Clone, Debug, Default)]
pub struct SensitivePolicy {
    allow: Vec<String>,
}

impl SensitivePolicy {
    /// A policy exempting nothing — the block-list is absolute.
    pub fn strict() -> Self {
        Self::default()
    }

    pub fn new(allow: Vec<String>) -> Self {
        Self {
            allow: allow
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .collect(),
        }
    }

    /// Load exemptions from `<workspace>/.gaviero/settings.json`. A missing or
    /// unparseable file yields [`Self::strict`] — failing closed is the whole
    /// point of the rail.
    pub fn resolve(workspace_root: &Path) -> Self {
        let path = workspace_root.join(".gaviero").join("settings.json");
        let Ok(body) = std::fs::read_to_string(&path) else {
            return Self::strict();
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&body) else {
            return Self::strict();
        };
        let Some(list) = doc
            .pointer("/agent/permissions/sensitivePaths/allow")
            .and_then(|v| v.as_array())
        else {
            return Self::strict();
        };
        Self::new(
            list.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect(),
        )
    }

    /// Is `path` exempted by an explicit user pattern?
    pub fn allows(&self, path: &Path) -> bool {
        if self.allow.is_empty() {
            return false;
        }
        let normalized = slash_path(path);
        self.allow
            .iter()
            .any(|pattern| crate::path_pattern::matches(pattern, &normalized))
    }

    /// `Some(reason)` when `path` is on the block-list and not exempted.
    ///
    /// This is the whole decision — callers log the reason and drop the write.
    pub fn refusal(&self, path: &Path) -> Option<String> {
        let entry = sensitive_match(path)?;
        if self.allows(path) {
            return None;
        }
        Some(format!(
            "matches sensitive block-list entry '{entry}'; exempt it with \
             `{ALLOW_SETTINGS_KEY}` in .gaviero/settings.json if intended"
        ))
    }
}

/// Enforces read/write scope for a single agent.
pub struct ScopeEnforcer {
    scope: FileScope,
    sensitive: SensitivePolicy,
}

impl ScopeEnforcer {
    /// Enforcer with an absolute block-list (no workspace exemptions).
    pub fn new(scope: FileScope) -> Self {
        Self {
            scope,
            sensitive: SensitivePolicy::strict(),
        }
    }

    /// Enforcer honouring the workspace's `sensitivePaths.allow` exemptions.
    pub fn with_policy(scope: FileScope, sensitive: SensitivePolicy) -> Self {
        Self { scope, sensitive }
    }

    /// Check whether `path` may be written by this agent.
    ///
    /// Blocks if:
    /// - The path is on the sensitive block-list and not exempted by the
    ///   workspace policy — checked first, and independently of the scope.
    /// - The path is outside a *declared* `scope.owned_paths`.
    ///
    /// An empty `owned_paths` declares **no write restriction**, matching
    /// [`crate::write_gate::WriteGatePipeline::is_scope_allowed`]. See
    /// [`FileScope::is_owned`] for why membership and authorization read the
    /// empty list differently.
    pub fn check_write(&self, path: &Path) -> Result<(), ScopeViolation> {
        if let Some(reason) = self.sensitive.refusal(path) {
            return Err(ScopeViolation {
                path: path.to_path_buf(),
                reason,
            });
        }

        if self.scope.owned_paths.is_empty() || self.scope.is_owned(&path.to_string_lossy()) {
            return Ok(());
        }

        Err(ScopeViolation {
            path: path.to_path_buf(),
            reason: format!(
                "path is outside agent's owned scope {:?}",
                self.scope.owned_paths
            ),
        })
    }

    /// Check whether `path` may be read by this agent.
    ///
    /// Blocks if the path is on the sensitive block-list and is neither
    /// exempted by the workspace policy nor named in the agent's declared
    /// readable paths.
    pub fn check_read(&self, path: &Path) -> Result<(), ScopeViolation> {
        let Some(entry) = sensitive_match(path) else {
            return Ok(());
        };
        if self.sensitive.allows(path) {
            return Ok(());
        }

        let path_str = path.to_string_lossy();
        // Allow if explicitly listed in owned_paths or read_only_paths
        let explicitly_allowed = self
            .scope
            .owned_paths
            .iter()
            .chain(self.scope.read_only_paths.iter())
            .any(|p| crate::path_pattern::matches(p, &path_str));

        if explicitly_allowed {
            return Ok(());
        }

        Err(ScopeViolation {
            path: path.to_path_buf(),
            reason: format!("sensitive file ('{entry}') not in declared scope"),
        })
    }

    /// Returns `true` if `path` matches any entry on the block-list, ignoring
    /// workspace exemptions. Use [`SensitivePolicy::refusal`] for the decision.
    pub fn is_sensitive(path: &Path) -> bool {
        sensitive_match(path).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_enforcer(owned: &[&str]) -> ScopeEnforcer {
        ScopeEnforcer::new(scope_with(owned))
    }

    fn scope_with(owned: &[&str]) -> FileScope {
        FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        }
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

    // ── Block-list matching is per-component, not substring ─────────────

    #[test]
    fn source_file_named_after_a_blocked_entry_is_not_sensitive() {
        // `path_str.contains("credentials")` used to block all of these.
        assert!(!ScopeEnforcer::is_sensitive(Path::new(
            "src/credentials_test.rs"
        )));
        assert!(!ScopeEnforcer::is_sensitive(Path::new(
            "docs/credentials.md"
        )));
        assert!(!ScopeEnforcer::is_sensitive(Path::new(
            "src/env_loader/mod.rs"
        )));
        assert!(!ScopeEnforcer::is_sensitive(Path::new("src/secrets_ui.rs")));
    }

    #[test]
    fn credential_files_are_still_sensitive() {
        assert!(ScopeEnforcer::is_sensitive(Path::new(".aws/credentials")));
        assert!(ScopeEnforcer::is_sensitive(Path::new("credentials")));
        assert!(ScopeEnforcer::is_sensitive(Path::new("home/.netrc")));
        assert!(ScopeEnforcer::is_sensitive(Path::new("cfg/secrets.toml")));
    }

    #[test]
    fn windows_separators_match_directory_entries() {
        // `.ssh/` and `.aws/` never matched a `\`-separated path before.
        assert!(ScopeEnforcer::is_sensitive(Path::new(r".ssh\id_rsa")));
        assert!(ScopeEnforcer::is_sensitive(Path::new(
            r"C:\Users\dev\.aws\config"
        )));
        assert!(ScopeEnforcer::is_sensitive(Path::new(r"home\.ssh")));
    }

    #[test]
    fn block_list_is_case_insensitive() {
        assert!(ScopeEnforcer::is_sensitive(Path::new("ID_RSA")));
        assert!(ScopeEnforcer::is_sensitive(Path::new(".SSH/known_hosts")));
    }

    #[test]
    fn env_family_is_blocked_but_templates_are_not() {
        assert!(ScopeEnforcer::is_sensitive(Path::new(".env")));
        assert!(ScopeEnforcer::is_sensitive(Path::new(".env.production")));
        assert!(ScopeEnforcer::is_sensitive(Path::new(".env.staging")));
        assert!(ScopeEnforcer::is_sensitive(Path::new("app/.env.ci")));

        assert!(!ScopeEnforcer::is_sensitive(Path::new(".env.example")));
        assert!(!ScopeEnforcer::is_sensitive(Path::new(".env.sample")));
        assert!(!ScopeEnforcer::is_sensitive(Path::new(".env.template")));
    }

    // ── Workspace exemptions ────────────────────────────────────────────

    #[test]
    fn policy_exempts_matching_fixture_paths() {
        let policy = SensitivePolicy::new(vec!["tests/fixtures/**".into()]);
        assert!(
            policy
                .refusal(Path::new("tests/fixtures/secrets.toml"))
                .is_none()
        );
        assert!(policy.refusal(Path::new("config/secrets.toml")).is_some());
    }

    #[test]
    fn strict_policy_exempts_nothing() {
        let policy = SensitivePolicy::strict();
        assert!(
            policy
                .refusal(Path::new("tests/fixtures/secrets.toml"))
                .is_some()
        );
        assert!(policy.refusal(Path::new("src/main.rs")).is_none());
    }

    #[test]
    fn refusal_names_the_matched_entry_and_the_escape_hatch() {
        let reason = SensitivePolicy::strict()
            .refusal(Path::new("config/.env.local"))
            .unwrap();
        assert!(reason.contains(".env.*"), "reason was: {reason}");
        assert!(reason.contains(ALLOW_SETTINGS_KEY), "reason was: {reason}");
    }

    #[test]
    fn enforcer_with_policy_allows_exempted_write_and_read() {
        let policy = SensitivePolicy::new(vec!["tests/fixtures/**".into()]);
        let e = ScopeEnforcer::with_policy(scope_with(&["."]), policy);
        assert!(
            e.check_write(Path::new("tests/fixtures/secrets.json"))
                .is_ok()
        );
        assert!(
            e.check_read(Path::new("tests/fixtures/secrets.json"))
                .is_ok()
        );
        assert!(e.check_write(Path::new(".env")).is_err());
    }

    #[test]
    fn resolve_reads_allow_list_from_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        let gaviero = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gaviero).unwrap();
        std::fs::write(
            gaviero.join("settings.json"),
            r#"{
              "agent": {
                "permissions": {
                  "sensitivePaths": { "allow": ["tests/fixtures/**", "  "] }
                }
              }
            }"#,
        )
        .unwrap();

        let policy = SensitivePolicy::resolve(dir.path());
        assert!(policy.allows(Path::new("tests/fixtures/.env")));
        assert!(!policy.allows(Path::new(".env")));
    }

    #[test]
    fn resolve_without_settings_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let policy = SensitivePolicy::resolve(dir.path());
        assert!(policy.refusal(Path::new(".env")).is_some());
    }
}
