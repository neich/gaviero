//! Provider-agnostic view of `agent.availableTools` / `approvedTools` /
//! `agent.permissions.bash`.
//!
//! Claude consumes these via `--tools` / `--allowedTools`. Cursor and Codex
//! have no equivalent argv, so the host translates the same lists into
//! deny rules (Cursor `cli.json`) and approval decisions (Codex
//! `requestApproval`).

use std::path::Path;

use crate::acp::session::AgentOptions;
use crate::agent_session::tool_agent::policy::ToolPolicy;

/// How the host should treat a shell command from a subprocess agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandDecision {
    /// Hard block: tool not on the surface, or denylist / builtin deny.
    Deny,
    /// Same auto-approve Claude would grant (`Bash` in approvedTools,
    /// allowlist prefix, or the turn's auto-approve flag).
    Allow,
    /// Bash is available but not auto-approved — Codex cannot prompt, so
    /// the caller falls back to its unattended cargo-verification policy.
    UnattendedFallback,
}

/// Snapshot of workspace tool settings for one session.
#[derive(Debug, Clone)]
pub(crate) struct AgentToolSurface {
    /// `None` when `AgentOptions` did not carry a list (legacy swarm
    /// constructors). Chat always supplies the workspace cascade, whose
    /// hardcoded default matches Claude (`Read`/`Write`/…, no `Bash`).
    available: Option<Vec<String>>,
    policy: ToolPolicy,
    auto_approve: bool,
}

impl AgentToolSurface {
    pub(crate) fn from_agent_options(options: &AgentOptions, workspace_root: &Path) -> Self {
        let mut policy = ToolPolicy::resolve(workspace_root);
        if let Some(approved) = options.approved_tools.as_ref() {
            policy.approved_tools = approved.clone();
        }
        Self {
            available: options.available_tools.clone(),
            policy,
            auto_approve: options.auto_approve,
        }
    }

    /// Test / Codex-default: no availableTools restriction, empty bash
    /// policy, no Bash auto-approve — callers fall back to cargo verification.
    #[cfg(test)]
    pub(crate) fn unrestricted_unattended() -> Self {
        Self {
            available: None,
            policy: ToolPolicy {
                allowlist: Vec::new(),
                denylist: Vec::new(),
                approved_tools: Vec::new(),
                ..ToolPolicy::default()
            },
            auto_approve: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn restricted_no_bash() -> Self {
        Self {
            available: Some(vec!["Read".into(), "Write".into(), "Edit".into()]),
            policy: ToolPolicy {
                allowlist: Vec::new(),
                denylist: Vec::new(),
                approved_tools: vec!["Read".into(), "Write".into(), "Edit".into()],
                ..ToolPolicy::default()
            },
            auto_approve: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn full_bash_approved() -> Self {
        Self {
            available: Some(vec!["Read".into(), "Bash".into()]),
            policy: ToolPolicy {
                allowlist: vec!["git status".into()],
                denylist: vec!["git push --force".into()],
                approved_tools: vec!["Read".into(), "Bash".into()],
                ..ToolPolicy::default()
            },
            auto_approve: false,
        }
    }

    pub(crate) fn bash_available(&self) -> bool {
        match &self.available {
            None => true,
            Some(list) => list.iter().any(|t| t == "Bash"),
        }
    }

    pub(crate) fn write_available(&self) -> bool {
        match &self.available {
            None => true,
            Some(list) => list
                .iter()
                .any(|t| matches!(t.as_str(), "Write" | "Edit" | "MultiEdit")),
        }
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
        AgentToolSurface {
            available: available.map(|a| a.iter().map(|s| s.to_string()).collect()),
            policy: ToolPolicy {
                allowlist: allow.iter().map(|s| s.to_string()).collect(),
                denylist: deny.iter().map(|s| s.to_string()).collect(),
                approved_tools: approved.iter().map(|s| s.to_string()).collect(),
                ..ToolPolicy::default()
            },
            auto_approve: false,
        }
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
    fn unset_available_tools_does_not_invent_a_restriction() {
        let s = surface(None, &[], &[], &[]);
        assert!(s.bash_available());
        assert!(s.write_available());
        assert_eq!(
            s.decide_command("git status"),
            CommandDecision::UnattendedFallback
        );
    }
}
