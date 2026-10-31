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
pub(crate) const BLOCKED: &[&str] = &[
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
pub(crate) const ENV_TEMPLATE_SUFFIXES: &[&str] = &[".example", ".sample", ".template", ".dist"];

/// Settings key holding the user's block-list exemptions.
const ALLOW_SETTINGS_KEY: &str = "agent.permissions.sensitivePaths.allow";

/// Settings key holding extra globs agents may never read.
const READ_DENY_SETTINGS_KEY: &str = "agent.permissions.read.deny";

// ── Read-only additions ─────────────────────────────────────────────────
//
// Everything below widens what agents may *read*; `check_write` and the
// turn-capture auto-revert keep using `BLOCKED` alone, so an agent can still
// generate a test certificate without its write being reverted.

/// Credential files (whole components) refused for reads only.
pub(crate) const READ_ONLY_EXTRA_NAMES: &[&str] =
    &[".npmrc", ".pypirc", ".git-credentials", ".pgpass", "id_ecdsa"];

/// Key/certificate extensions refused for reads only, matched
/// case-insensitively on the final component. `.pub` is never listed.
pub(crate) const READ_ONLY_EXTRA_EXTENSIONS: &[&str] =
    &["pem", "key", "p12", "pfx", "keystore", "jks"];

/// Trailing component sequences refused for reads only.
pub(crate) const READ_ONLY_EXTRA_SUFFIX_PATHS: &[&[&str]] =
    &[&[".docker", "config.json"], &[".kube", "config"]];

/// Hard floor under the home directory: agent and gaviero credentials. Never
/// exemptable. The label is what refusal messages show.
pub(crate) const HOME_FLOOR: &[(&[&str], &str)] = &[
    (&[".claude", ".credentials.json"], "~/.claude/.credentials.json"),
    (&[".codex", "auth.json"], "~/.codex/auth.json"),
    (&[".gaviero", "remote"], "~/.gaviero/remote"),
    (&[".gaviero", "secrets.toml"], "~/.gaviero/secrets.toml"),
];

/// Hard floor for gaviero's own state: a `.gaviero` component followed by one
/// of these. Never exemptable. `settings.json`, `skills/`, `tmp/` (spilled
/// prompts the agent is told to read) and `MEMORY.md` stay readable.
pub(crate) const GAVIERO_STATE_FLOOR: &[(&str, &str)] = &[
    ("mcp-http-token", ".gaviero/mcp-http-token"),
    ("secrets.toml", ".gaviero/secrets.toml"),
    ("history", ".gaviero/history"),
    ("turns", ".gaviero/turns"),
    ("remote", ".gaviero/remote"),
    ("egress", ".gaviero/egress"),
    ("mcp_calls.ndjson", ".gaviero/mcp_calls.ndjson"),
];

/// Same floor, matched by prefix (`memory.db-wal`, `memory.db.bak-…`).
pub(crate) const GAVIERO_STATE_FLOOR_PREFIXES: &[(&str, &str)] = &[
    ("memory.db", ".gaviero/memory.db*"),
    ("code_graph.db", ".gaviero/code_graph.db*"),
];

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

/// [`components`] with `..` resolved lexically (no filesystem access).
fn lexical_components(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in components(path) {
        if c == ".." {
            out.pop();
        } else {
            out.push(c);
        }
    }
    out
}

fn component_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The hard-floor entry `path` matches, if any: agent credential files under
/// the home directory and gaviero's secret-bearing state. No exemption
/// applies to these.
pub fn read_floor_match(path: &Path) -> Option<&'static str> {
    let comps = lexical_components(path);
    for pair in comps.windows(2) {
        if !pair[0].eq_ignore_ascii_case(".gaviero") {
            continue;
        }
        let next = &pair[1];
        if let Some((_, label)) = GAVIERO_STATE_FLOOR
            .iter()
            .find(|(entry, _)| next.eq_ignore_ascii_case(entry))
        {
            return Some(label);
        }
        let lower = next.to_ascii_lowercase();
        if let Some((_, label)) = GAVIERO_STATE_FLOOR_PREFIXES
            .iter()
            .find(|(prefix, _)| lower.starts_with(prefix))
        {
            return Some(label);
        }
    }
    let home = lexical_components(&dirs::home_dir()?);
    if home.is_empty() {
        return None;
    }
    HOME_FLOOR.iter().find_map(|(seq, label)| {
        let full: Vec<&str> = home
            .iter()
            .map(|s| s.as_str())
            .chain(seq.iter().copied())
            .collect();
        let hit = comps.len() >= full.len()
            && comps
                .iter()
                .zip(full.iter())
                .all(|(a, b)| component_eq(a, b));
        hit.then_some(*label)
    })
}

/// Every read-sensitive entry `path` matches: the hard floor (prefixed
/// `floor:`), the write block-list, then the read-only additions.
pub fn read_sensitive_match(path: &Path) -> Option<String> {
    if let Some(label) = read_floor_match(path) {
        return Some(format!("floor:{label}"));
    }
    if let Some(entry) = sensitive_match(path) {
        return Some(entry.to_string());
    }
    let comps = components(path);
    if let Some(name) = comps.iter().find_map(|c| {
        READ_ONLY_EXTRA_NAMES
            .iter()
            .find(|n| c.eq_ignore_ascii_case(n))
    }) {
        return Some((*name).to_string());
    }
    if let Some(last) = comps.last()
        && let Some((_, ext)) = last.rsplit_once('.')
        && READ_ONLY_EXTRA_EXTENSIONS
            .iter()
            .any(|e| ext.eq_ignore_ascii_case(e))
    {
        return Some(format!("*.{}", ext.to_ascii_lowercase()));
    }
    READ_ONLY_EXTRA_SUFFIX_PATHS.iter().find_map(|seq| {
        let hit = comps.len() >= seq.len()
            && comps[comps.len() - seq.len()..]
                .iter()
                .zip(seq.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        hit.then(|| seq.join("/"))
    })
}

/// `path` relative to `root` with `/` separators, or the whole path when it
/// is not under `root`.
fn relative_slash(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    slash_path(rel)
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
///
/// The same file supplies `agent.permissions.read.deny`: extra globs agents
/// may never read, on top of the block-list and the hard floor
/// ([`read_floor_match`]).
#[derive(Clone, Debug, Default)]
pub struct SensitivePolicy {
    allow: Vec<String>,
    read_deny: Vec<String>,
    /// Workspace root the policy was resolved for; anchors `read_deny` globs.
    root: Option<PathBuf>,
}

fn non_blank(list: Vec<String>) -> Vec<String> {
    list.into_iter().filter(|p| !p.trim().is_empty()).collect()
}

fn string_array(doc: &serde_json::Value, pointer: &str) -> Vec<String> {
    doc.pointer(pointer)
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

impl SensitivePolicy {
    /// A policy exempting nothing — the block-list is absolute.
    pub fn strict() -> Self {
        Self::default()
    }

    pub fn new(allow: Vec<String>) -> Self {
        Self {
            allow: non_blank(allow),
            ..Self::default()
        }
    }

    /// Add `agent.permissions.read.deny` globs (workspace-relative).
    pub fn with_read_deny(mut self, read_deny: Vec<String>) -> Self {
        self.read_deny = non_blank(read_deny);
        self
    }

    /// Load exemptions and read denies from
    /// `<workspace>/.gaviero/settings.json`. A missing or unparseable file
    /// yields [`Self::strict`] — failing closed is the whole point of the
    /// rail.
    pub fn resolve(workspace_root: &Path) -> Self {
        let root = Some(workspace_root.to_path_buf());
        let path = workspace_root.join(".gaviero").join("settings.json");
        let Ok(body) = std::fs::read_to_string(&path) else {
            return Self {
                root,
                ..Self::strict()
            };
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&body) else {
            return Self {
                root,
                ..Self::strict()
            };
        };
        Self {
            allow: non_blank(string_array(&doc, "/agent/permissions/sensitivePaths/allow")),
            read_deny: non_blank(string_array(&doc, "/agent/permissions/read/deny")),
            root,
        }
    }

    /// `agent.permissions.read.deny` globs.
    pub fn read_deny(&self) -> &[String] {
        &self.read_deny
    }

    /// `agent.permissions.sensitivePaths.allow` globs.
    pub fn allow_globs(&self) -> &[String] {
        &self.allow
    }

    /// Workspace root this policy was resolved for, when known.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// `Some(reason)` when an agent must not read `path`.
    ///
    /// Order: the hard floor (never exemptable), then
    /// `agent.permissions.read.deny`, then the block-list plus the read-only
    /// additions (exemptable through `sensitivePaths.allow`).
    pub fn read_refusal(&self, path: &Path, workspace_root: &Path) -> Option<String> {
        if let Some(label) = read_floor_match(path) {
            return Some(format!(
                "{} is a protected credential/state file ({label}) and is never sent to agents",
                path.display()
            ));
        }
        let rel = relative_slash(path, workspace_root);
        if let Some(glob) = self
            .read_deny
            .iter()
            .find(|g| crate::path_pattern::matches(g, &rel))
        {
            return Some(format!(
                "{} matches {READ_DENY_SETTINGS_KEY} {glob:?}",
                path.display()
            ));
        }
        let entry = read_sensitive_match(path)?;
        if self.allows(path) || self.allows(Path::new(&rel)) {
            return None;
        }
        Some(format!(
            "{} is a sensitive file ({entry}); exempt it with {ALLOW_SETTINGS_KEY} in \
             .gaviero/settings.json",
            path.display()
        ))
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
    /// Blocks hard-floor paths ([`read_floor_match`]) unconditionally. Any
    /// other refusal from [`SensitivePolicy::read_refusal`] (read denies, the
    /// block-list, the read-only additions) is lifted when the path is named
    /// in the agent's declared readable paths.
    pub fn check_read(&self, path: &Path) -> Result<(), ScopeViolation> {
        if let Some(label) = read_floor_match(path) {
            return Err(ScopeViolation {
                path: path.to_path_buf(),
                reason: format!("protected credential/state file ('{label}')"),
            });
        }
        let root = self.sensitive.root().unwrap_or(Path::new(""));
        let Some(entry) = self.sensitive.read_refusal(path, root) else {
            return Ok(());
        };

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
            reason: format!("not in declared scope: {entry}"),
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

    // ── Read policy ─────────────────────────────────────────────────────

    fn permissive() -> SensitivePolicy {
        SensitivePolicy::new(vec!["**".into()])
    }

    #[test]
    fn home_floor_is_refused_even_when_everything_is_exempted() {
        let home = dirs::home_dir().expect("home dir");
        let root = Path::new("/w");
        for tail in [
            ".claude/.credentials.json",
            ".codex/auth.json",
            ".gaviero/remote/token",
            ".gaviero/secrets.toml",
        ] {
            let path = home.join(tail);
            assert!(read_floor_match(&path).is_some(), "{tail} not on the floor");
            assert!(permissive().read_refusal(&path, root).is_some(), "{tail} exempted");
        }
    }

    #[test]
    fn gaviero_state_floor_is_refused_even_when_everything_is_exempted() {
        let root = Path::new("/w");
        for tail in [
            ".gaviero/mcp-http-token",
            ".gaviero/history/turns.ndjson",
            ".gaviero/turns/blob",
            ".gaviero/remote/tls/key",
            ".gaviero/egress/claude-bodies/x.request.json",
            ".gaviero/mcp_calls.ndjson",
            ".gaviero/memory.db-wal",
            ".gaviero/memory.db.bak-v10",
            ".gaviero/code_graph.db",
        ] {
            let path = root.join(tail);
            assert!(read_floor_match(&path).is_some(), "{tail} not on the floor");
            assert!(permissive().read_refusal(&path, root).is_some(), "{tail} exempted");
            let e = ScopeEnforcer::with_policy(scope_with(&["."]), permissive());
            assert!(e.check_read(&path).is_err(), "{tail} readable via scope");
        }
    }

    #[test]
    fn gaviero_settings_skills_and_tmp_stay_readable() {
        let root = Path::new("/w");
        let strict = SensitivePolicy::strict();
        for tail in [
            ".gaviero/settings.json",
            ".gaviero/tmp/prompt-x.md",
            ".gaviero/skills/deep-research/SKILL.md",
            ".gaviero/MEMORY.md",
        ] {
            let path = root.join(tail);
            assert!(read_floor_match(&path).is_none(), "{tail} on the floor");
            assert!(strict.read_refusal(&path, root).is_none(), "{tail} refused");
        }
    }

    #[test]
    fn pem_and_key_files_are_refused_but_public_keys_are_not() {
        let root = Path::new("/w");
        let strict = SensitivePolicy::strict();
        assert!(strict.read_refusal(&root.join("certs/server.pem"), root).is_some());
        assert!(strict.read_refusal(&root.join("tls/Server.KEY"), root).is_some());
        assert!(strict.read_refusal(&root.join("home/.ssh2/id_ed25519.pub"), root).is_none());
        assert!(strict.read_refusal(&root.join(".npmrc"), root).is_some());
    }

    #[test]
    fn docker_and_kube_configs_are_refused_but_other_configs_are_not() {
        let root = Path::new("/w");
        let strict = SensitivePolicy::strict();
        assert!(strict.read_refusal(&root.join(".docker/config.json"), root).is_some());
        assert!(strict.read_refusal(&root.join(".kube/config"), root).is_some());
        assert!(strict.read_refusal(&root.join("src/config.json"), root).is_none());
    }

    #[test]
    fn read_deny_glob_refuses_matching_paths() {
        let root = Path::new("/w");
        let policy = SensitivePolicy::strict().with_read_deny(vec!["secrets/**".into()]);
        let reason = policy.read_refusal(&root.join("secrets/a.txt"), root).unwrap();
        assert!(reason.contains(READ_DENY_SETTINGS_KEY), "reason was: {reason}");
        assert!(policy.read_refusal(&root.join("src/a.txt"), root).is_none());
    }

    #[test]
    fn resolve_reads_read_deny_from_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        let gaviero = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gaviero).unwrap();
        std::fs::write(
            gaviero.join("settings.json"),
            r#"{"agent":{"permissions":{"read":{"deny":["private/**"]}}}}"#,
        )
        .unwrap();
        let policy = SensitivePolicy::resolve(dir.path());
        assert_eq!(policy.read_deny(), ["private/**".to_string()]);
        assert_eq!(policy.root(), Some(dir.path()));
        let e = ScopeEnforcer::with_policy(scope_with(&[]), policy);
        assert!(e.check_read(&dir.path().join("private/x.md")).is_err());
    }

    #[test]
    fn allow_exempts_env_family_but_never_the_floor() {
        let root = Path::new("/w");
        let policy = SensitivePolicy::new(vec![".env.local".into()]);
        assert!(policy.read_refusal(&root.join(".env.local"), root).is_none());
        let home = dirs::home_dir().expect("home dir");
        let policy = SensitivePolicy::new(vec!["**".into()]);
        assert!(policy.read_refusal(&home.join(".codex/auth.json"), root).is_some());
    }

    #[test]
    fn check_write_is_unchanged_for_read_only_additions() {
        let e = make_enforcer(&[]);
        assert!(e.check_write(Path::new("certs/server.pem")).is_ok());
        assert!(e.check_write(Path::new(".docker/config.json")).is_ok());
        assert!(e.check_write(Path::new(".env")).is_err());
    }

    #[test]
    fn resolve_without_settings_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let policy = SensitivePolicy::resolve(dir.path());
        assert!(policy.refusal(Path::new(".env")).is_some());
    }
}
