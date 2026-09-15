//! Provider-agnostic view of `agent.availableTools` / `approvedTools` /
//! `agent.permissions.bash`.
//!
//! **Consumers.** Codex app-server decides each command at runtime via
//! [`AgentToolSurface::decide_command`]; `mcp::config_synth` derives Cursor's
//! build-time `permissions.deny` rules from
//! [`bash_available`](Self::bash_available) /
//! [`write_available`](Self::write_available) rather than re-scanning the list
//! itself; the in-process tool-agent builds its registry from
//! [`available`](Self::available); and Claude's `--tools` argv is derived from
//! the list [`resolve_available`](Self::resolve_available) returns
//! (`AgentOptions::resolved_tools`, `acp/session.rs`), so no adapter
//! re-implements that setting.
//!
//! **This is the single resolution of `agent.availableTools`.** `None` — the
//! caller never populated the field — resolves to [`DEFAULT_AVAILABLE_TOOLS`],
//! the same list the workspace cascade yields when the key is absent. It does
//! *not* mean "unrestricted". That second reading was live: an unset field
//! granted `Bash` here while `AgentOptions::resolved_tools` (`acp/session.rs`)
//! withheld it, which is exactly the "`None` means opposite things in different
//! layers" divergence `plans/provider-parity` §2.4 records. It is resolved
//! **fail-closed**: an unpopulated field must never grant shell access.

use std::path::Path;

use crate::acp::session::{AgentOptions, DEFAULT_AVAILABLE_TOOLS};
use crate::agent_session::tool_agent::policy::ToolPolicy;

/// How the host should treat a shell command from a subprocess agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandDecision {
    /// Hard block: tool not on the surface, or denylist / builtin deny.
    Deny,
    /// Same auto-approve Claude would grant (`Bash` in approvedTools,
    /// allowlist prefix, or the turn's auto-approve flag).
    Allow,
    /// Bash is available on the surface but not auto-approved. The *caller*
    /// chooses what to do: Codex prompts the user (`prompt_for_command`,
    /// `codex_app_server.rs`), while unattended callers fall back to cargo
    /// verification.
    ///
    /// This arm does **not** mean "the provider cannot prompt" — Codex has a
    /// working `on_permission_request` channel. An earlier doc comment here
    /// claimed otherwise and was wrong.
    UnattendedFallback,
}

/// Snapshot of workspace tool settings for one session.
#[derive(Debug, Clone)]
pub(crate) struct AgentToolSurface {
    /// Never "unset": `agent.availableTools` is resolved to a concrete list in
    /// [`AgentToolSurface::from_parts`]. An empty vec is *not* the same as an
    /// absent setting — an operator who configures `[]` gets a surface with no
    /// built-in tools, which is what they asked for. See the module header.
    available: Vec<String>,
    policy: ToolPolicy,
    auto_approve: bool,
}

/// The list an absent `agent.availableTools` resolves to: the legacy Claude
/// fallback (`Read`/`Glob`/`Grep`/`Write`/`Edit`/`MultiEdit`, **no `Bash`**),
/// which is also what the workspace cascade hardcodes for the same key.
fn default_available_tools() -> Vec<String> {
    DEFAULT_AVAILABLE_TOOLS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

impl AgentToolSurface {
    /// Resolve `agent.availableTools` to a concrete list.
    ///
    /// This is the **single** resolution of that setting: an absent value is
    /// [`DEFAULT_AVAILABLE_TOOLS`] (no `Bash`), *not* "unrestricted", and an
    /// explicit `[]` is an empty surface. `AgentOptions::resolved_tools`
    /// (`acp/session.rs`) — Claude's `--tools` argv — delegates here rather
    /// than re-deriving the same fallback, so the two cannot drift (the
    /// divergence `plans/provider-parity` §2.4 records).
    pub(crate) fn resolve_available(available: Option<Vec<String>>) -> Vec<String> {
        available.unwrap_or_else(default_available_tools)
    }

    pub(crate) fn from_agent_options(options: &AgentOptions, workspace_root: &Path) -> Self {
        // The host resolves the policy through the workspace cascade and
        // hands it down; the path-based fallback only covers callers that
        // never populated `AgentOptions::tool_policy`.
        let mut policy = options
            .tool_policy
            .clone()
            .unwrap_or_else(|| ToolPolicy::resolve(workspace_root));
        if let Some(approved) = options.approved_tools.as_ref() {
            policy.approved_tools = approved.clone();
        }
        Self::from_parts(options.available_tools.clone(), policy, options.auto_approve)
    }

    /// Assemble from already-resolved parts.
    ///
    /// Split out from [`from_agent_options`](Self::from_agent_options) because
    /// a workspace-cascade caller — `mcp::config_synth`, which synthesizes
    /// Cursor's deny rules — holds a `Workspace` rather than an
    /// `AgentOptions`, and must be able to reach this same list without
    /// re-reading the settings.
    pub(crate) fn from_parts(
        available: Option<Vec<String>>,
        policy: ToolPolicy,
        auto_approve: bool,
    ) -> Self {
        Self {
            available: Self::resolve_available(available),
            policy,
            auto_approve,
        }
    }

    /// The resolved `agent.availableTools` list. Never "unset" — see the
    /// module header. Consumers that need the whole surface answer use
    /// [`bash_available`](Self::bash_available) /
    /// [`write_available`](Self::write_available) instead of scanning this.
    pub(crate) fn available(&self) -> &[String] {
        &self.available
    }

    pub(crate) fn policy(&self) -> &ToolPolicy {
        &self.policy
    }

    /// Test / Codex-default: no availableTools restriction, empty bash
    /// policy, no Bash auto-approve — callers fall back to cargo verification.
    #[cfg(test)]
    pub(crate) fn unrestricted_unattended() -> Self {
        // Spelled out rather than passing `None`: `None` now resolves to the
        // default list, which omits `Bash` — the opposite of what this
        // constructor promises.
        Self::from_parts(
            Some(vec![
                "Read".into(),
                "Glob".into(),
                "Grep".into(),
                "Write".into(),
                "Edit".into(),
                "MultiEdit".into(),
                "Bash".into(),
            ]),
            ToolPolicy {
                allowlist: Vec::new(),
                denylist: Vec::new(),
                approved_tools: Vec::new(),
                ..ToolPolicy::default()
            },
            false,
        )
    }

    #[cfg(test)]
    pub(crate) fn restricted_no_bash() -> Self {
        Self::from_parts(
            Some(vec!["Read".into(), "Write".into(), "Edit".into()]),
            ToolPolicy {
                allowlist: Vec::new(),
                denylist: Vec::new(),
                approved_tools: vec!["Read".into(), "Write".into(), "Edit".into()],
                ..ToolPolicy::default()
            },
            false,
        )
    }

    #[cfg(test)]
    pub(crate) fn full_bash_approved() -> Self {
        Self::from_parts(
            Some(vec!["Read".into(), "Bash".into()]),
            ToolPolicy {
                allowlist: vec!["git status".into()],
                denylist: vec!["git push --force".into()],
                approved_tools: vec!["Read".into(), "Bash".into()],
                ..ToolPolicy::default()
            },
            false,
        )
    }

    pub(crate) fn bash_available(&self) -> bool {
        self.available.iter().any(|t| t == "Bash")
    }

    pub(crate) fn write_available(&self) -> bool {
        self.available
            .iter()
            .any(|t| matches!(t.as_str(), "Write" | "Edit" | "MultiEdit"))
    }

    pub(crate) fn decide_command(&self, command: &str) -> CommandDecision {
        if !self.bash_available() {
            return CommandDecision::Deny;
        }
        if self.policy.deny_reason(command).is_some() {
            return CommandDecision::Deny;
        }
        if self.auto_approve
            || self.policy.bash_tool_approved()
            || self.policy.matches_allowlist(command)
        {
            return CommandDecision::Allow;
        }
        CommandDecision::UnattendedFallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(
        available: Option<&[&str]>,
        approved: &[&str],
        allow: &[&str],
        deny: &[&str],
    ) -> AgentToolSurface {
        AgentToolSurface::from_parts(
            available.map(|a| a.iter().map(|s| s.to_string()).collect()),
            ToolPolicy {
                allowlist: allow.iter().map(|s| s.to_string()).collect(),
                denylist: deny.iter().map(|s| s.to_string()).collect(),
                approved_tools: approved.iter().map(|s| s.to_string()).collect(),
                ..ToolPolicy::default()
            },
            false,
        )
    }

    #[test]
    fn restricted_profile_denies_all_shell() {
        let s = surface(
            Some(&["Read", "Write", "Edit"]),
            &["Read", "Write", "Edit"],
            &[],
            &["git push --force"],
        );
        assert!(!s.bash_available());
        assert_eq!(s.decide_command("cargo test"), CommandDecision::Deny);
        assert_eq!(s.decide_command("git status"), CommandDecision::Deny);
    }

    #[test]
    fn approved_bash_allows_non_denied_commands() {
        let s = surface(
            Some(&["Read", "Bash"]),
            &["Read", "Bash"],
            &["git status"],
            &["git push --force"],
        );
        assert_eq!(s.decide_command("git status"), CommandDecision::Allow);
        assert_eq!(s.decide_command("ls -la"), CommandDecision::Allow);
        assert_eq!(
            s.decide_command("git push --force origin main"),
            CommandDecision::Deny
        );
    }

    #[test]
    fn available_but_not_approved_uses_allowlist_then_fallback() {
        let s = surface(Some(&["Read", "Bash"]), &["Read"], &["git status"], &[]);
        assert_eq!(s.decide_command("git status"), CommandDecision::Allow);
        assert_eq!(
            s.decide_command("npm publish"),
            CommandDecision::UnattendedFallback
        );
    }

    #[test]
    fn unset_available_tools_resolves_to_the_documented_default() {
        // An unpopulated field means "the caller did not configure a surface",
        // which is the legacy default — *not* "no restriction". The two
        // readings disagreed with `AgentOptions::resolved_tools`; this is the
        // one that survives, and it is the fail-closed one.
        let s = surface(None, &[], &[], &[]);
        assert_eq!(s.available(), default_available_tools());
        assert!(!s.bash_available(), "unset must not grant shell access");
        assert!(s.write_available(), "the default surface keeps the write tools");
        assert_eq!(
            s.decide_command("git status"),
            CommandDecision::Deny,
            "Bash is off the default surface, so no fallback is reachable"
        );
    }

    #[test]
    fn empty_available_tools_is_not_the_same_as_unset() {
        // An operator who writes `"availableTools": []` gets no built-in tools
        // at all. Callers previously collapsed `Some([])` and `None` into one
        // branch, which turned an explicit "nothing" into "everything".
        let empty = surface(Some(&[]), &[], &[], &[]);
        assert!(empty.available().is_empty());
        assert!(!empty.bash_available());
        assert!(!empty.write_available());

        let unset = surface(None, &[], &[], &[]);
        assert_ne!(empty.available(), unset.available());
    }

    #[test]
    fn unrestricted_constructor_keeps_bash_on_the_surface() {
        // Guards the explicit list in `unrestricted_unattended`: it cannot be
        // expressed by passing `None` any more.
        let s = AgentToolSurface::unrestricted_unattended();
        assert!(s.bash_available());
        assert!(s.write_available());
    }
}
