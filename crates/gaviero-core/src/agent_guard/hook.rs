//! Decisions for `gaviero-cli --hook <provider:event>`.
//!
//! A provider runs the hook with the event JSON on stdin and reads the
//! decision JSON from stdout. The decision is taken from
//! [`SensitivePolicy`] — `.gaviero/settings.json` is the only policy source.
//! Shapes were verified live (2026-10-31): Claude `PostToolUse`
//! `hookSpecificOutput.updatedToolOutput` replaces a Bash result with the
//! same `{stdout, stderr, interrupted, isImage}` object; Cursor reads
//! `{"permission": "allow"|"deny", …}` from `beforeReadFile` and
//! `beforeShellExecution`.
//!
//! Malformed input never panics and never blocks: the hook passes and the
//! audit row says why.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agent_session::tool_agent::policy::command_protected_path;
use crate::scope_enforcer::SensitivePolicy;

/// Line substituted for a Bash output line that came from a protected file.
pub const WITHHELD_LINE: &str = "[gaviero: line from protected file withheld]";

/// Which agent CLI called the hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookProvider {
    Claude,
    Cursor,
}

/// Which hook event fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PostToolUse,
    BeforeReadFile,
    BeforeShellExecution,
}

impl HookProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
        }
    }
}

impl HookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::PostToolUse => "post-tool-use",
            Self::BeforeReadFile => "before-read-file",
            Self::BeforeShellExecution => "before-shell-execution",
        }
    }
}

/// Parse the `--hook` argument. Only the combinations gaviero wires up are
/// accepted.
pub fn parse_hook_arg(arg: &str) -> anyhow::Result<(HookProvider, HookEvent)> {
    match arg {
        "claude:post-tool-use" => Ok((HookProvider::Claude, HookEvent::PostToolUse)),
        "cursor:before-read-file" => Ok((HookProvider::Cursor, HookEvent::BeforeReadFile)),
        "cursor:before-shell-execution" => {
            Ok((HookProvider::Cursor, HookEvent::BeforeShellExecution))
        }
        other => anyhow::bail!(
            "unknown hook {other:?}; expected claude:post-tool-use, \
             cursor:before-read-file or cursor:before-shell-execution"
        ),
    }
}

/// What the hook prints and what it records.
#[derive(Debug, Clone)]
pub struct HookOutcome {
    /// Exactly what goes to stdout (may be empty: "no change").
    pub stdout: String,
    /// One audit row for `.gaviero/egress/hooks.ndjson`.
    pub audit: Value,
}

/// The workspace root the event belongs to: Claude's `cwd`; Cursor's first
/// `workspace_roots` entry, else its `cwd`.
pub fn hook_root(provider: HookProvider, input: &Value) -> Option<PathBuf> {
    let cwd = input.get("cwd").and_then(Value::as_str);
    let root = match provider {
        HookProvider::Claude => cwd,
        HookProvider::Cursor => input
            .get("workspace_roots")
            .and_then(|r| r.get(0))
            .and_then(Value::as_str)
            .or(cwd),
    }?;
    (!root.is_empty()).then(|| PathBuf::from(root))
}

/// Decide one hook event.
pub fn handle_hook(provider: HookProvider, event: HookEvent, input: &Value) -> HookOutcome {
    let tool = input.get("tool_name").and_then(Value::as_str);
    let mut audit = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "provider": provider.as_str(),
        "event": event.as_str(),
        "tool": tool,
        "decision": "pass",
        "reason": Value::Null,
        "path_or_command": Value::Null,
        "rules": [],
    });
    let Some(root) = hook_root(provider, input) else {
        audit["reason"] = json!("unrecognized input");
        return HookOutcome {
            stdout: pass_stdout(provider, event),
            audit,
        };
    };
    let policy = SensitivePolicy::resolve(&root);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());

    match (provider, event) {
        (HookProvider::Claude, HookEvent::PostToolUse) => {
            claude_post_tool_use(input, &cwd, &root, &policy, audit)
        }
        (HookProvider::Cursor, HookEvent::BeforeReadFile) => {
            let Some(file) = input.get("file_path").and_then(Value::as_str) else {
                audit["reason"] = json!("unrecognized input");
                return HookOutcome {
                    stdout: pass_stdout(provider, event),
                    audit,
                };
            };
            audit["path_or_command"] = json!(file);
            let path = absolutize(Path::new(file), &cwd);
            cursor_decision(policy.read_refusal(&path, &root), audit)
        }
        (HookProvider::Cursor, HookEvent::BeforeShellExecution) => {
            let Some(command) = input.get("command").and_then(Value::as_str) else {
                audit["reason"] = json!("unrecognized input");
                return HookOutcome {
                    stdout: pass_stdout(provider, event),
                    audit,
                };
            };
            audit["path_or_command"] = json!(command);
            cursor_decision(command_protected_path(command, &cwd, &policy, &root), audit)
        }
        _ => {
            audit["reason"] = json!("unsupported provider/event pair");
            HookOutcome {
                stdout: pass_stdout(provider, event),
                audit,
            }
        }
    }
}

/// Append `audit` to `<root>/.gaviero/egress/hooks.ndjson`. Best-effort.
pub fn append_audit(root: &Path, audit: &Value) {
    use std::io::Write;
    let dir = root.join(".gaviero").join("egress");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "hook audit: cannot create {}", dir.display());
        return;
    }
    let path = dir.join("hooks.ndjson");
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{audit}"));
    if let Err(e) = result {
        tracing::warn!(error = %e, "hook audit: cannot append to {}", path.display());
    }
}

fn absolutize(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// "Allow / no change" for each event: Claude reads an empty stdout as no
/// change; Cursor needs an explicit allow.
fn pass_stdout(provider: HookProvider, _event: HookEvent) -> String {
    match provider {
        HookProvider::Claude => String::new(),
        HookProvider::Cursor => json!({"permission": "allow", "continue": true}).to_string(),
    }
}

fn cursor_decision(refusal: Option<String>, mut audit: Value) -> HookOutcome {
    match refusal {
        Some(reason) => {
            audit["decision"] = json!("deny");
            audit["reason"] = json!(reason);
            HookOutcome {
                stdout: json!({
                    "permission": "deny",
                    "continue": true,
                    "user_message": reason,
                    "agent_message": reason,
                })
                .to_string(),
                audit,
            }
        }
        None => {
            audit["decision"] = json!("allow");
            HookOutcome {
                stdout: json!({"permission": "allow", "continue": true}).to_string(),
                audit,
            }
        }
    }
}

/// The path part of a `grep`/`rg` style `<path>:<rest>` line, skipping a
/// Windows drive colon (`C:\…`).
fn grep_line_path(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let skip = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        2
    } else {
        0
    };
    let idx = line[skip..].find(':')? + skip;
    let path = &line[..idx];
    (!path.is_empty()).then_some(path)
}

fn claude_post_tool_use(
    input: &Value,
    cwd: &Path,
    root: &Path,
    policy: &SensitivePolicy,
    mut audit: Value,
) -> HookOutcome {
    let response = input.get("tool_response");
    let stdout = response.and_then(|r| r.get("stdout")).and_then(Value::as_str);
    let (Some("Bash"), Some(stdout)) = (input.get("tool_name").and_then(Value::as_str), stdout)
    else {
        return HookOutcome {
            stdout: String::new(),
            audit,
        };
    };
    audit["path_or_command"] = input
        .get("tool_input")
        .and_then(|i| i.get("command"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut changed = false;
    let mut withheld = 0usize;
    let mut rules: Vec<&'static str> = Vec::new();
    let lines: Vec<String> = stdout
        .split('\n')
        .map(|line| {
            if let Some(path) = grep_line_path(line) {
                let resolved = absolutize(Path::new(path), cwd);
                if policy.read_refusal(&resolved, root).is_some() {
                    changed = true;
                    withheld += 1;
                    return WITHHELD_LINE.to_string();
                }
            }
            let (redacted, fired) = crate::secret_scan::redact(line);
            if fired.is_empty() {
                return line.to_string();
            }
            changed = true;
            for rule in fired {
                if !rules.contains(&rule) {
                    rules.push(rule);
                }
            }
            redacted
        })
        .collect();

    if !changed {
        return HookOutcome {
            stdout: String::new(),
            audit,
        };
    }
    let mut updated = response.cloned().unwrap_or_else(|| json!({}));
    updated["stdout"] = json!(lines.join("\n"));
    audit["decision"] = json!("redact");
    audit["rules"] = json!(rules);
    if withheld > 0 {
        audit["reason"] = json!(format!("{withheld} line(s) from protected files withheld"));
    }
    HookOutcome {
        stdout: json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "updatedToolOutput": updated,
            }
        })
        .to_string(),
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash_post(root: &Path, stdout: &str) -> Value {
        json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "cwd": root.to_string_lossy(),
            "tool_input": {"command": "grep -r DUMMY ."},
            "tool_response": {"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false},
        })
    }

    #[test]
    fn parse_hook_arg_accepts_only_wired_pairs() {
        assert!(parse_hook_arg("claude:post-tool-use").is_ok());
        assert!(parse_hook_arg("cursor:before-read-file").is_ok());
        assert!(parse_hook_arg("cursor:before-shell-execution").is_ok());
        assert!(parse_hook_arg("codex:pre-tool-use").is_err());
    }

    #[test]
    fn claude_bash_lines_from_protected_files_are_withheld() {
        let dir = tempfile::tempdir().unwrap();
        let out = handle_hook(
            HookProvider::Claude,
            HookEvent::PostToolUse,
            &bash_post(dir.path(), "./.env:DUMMY_SECRET=canary\nREADME.md:hello"), // pragma: allowlist secret
        );
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        let updated = &v["hookSpecificOutput"]["updatedToolOutput"];
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(
            updated["stdout"],
            format!("{WITHHELD_LINE}\nREADME.md:hello")
        );
        assert_eq!(updated["interrupted"], false);
        assert_eq!(updated["isImage"], false);
        assert_eq!(out.audit["decision"], "redact");
    }

    #[test]
    fn claude_bash_secret_values_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let key = format!("sk-ant-{}", "a".repeat(30)); // pragma: allowlist secret
        let out = handle_hook(
            HookProvider::Claude,
            HookEvent::PostToolUse,
            &bash_post(dir.path(), &format!("value is {key}")),
        );
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        let stdout = v["hookSpecificOutput"]["updatedToolOutput"]["stdout"]
            .as_str()
            .unwrap();
        assert!(!stdout.contains(&key));
        assert!(stdout.contains("[REDACTED:anthropic-api-key]"));
        assert_eq!(out.audit["rules"], json!(["anthropic-api-key"]));
    }

    #[test]
    fn claude_non_bash_tools_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let input = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Read",
            "cwd": dir.path().to_string_lossy(),
            "tool_response": {"file": {"content": "x"}},
        });
        let out = handle_hook(HookProvider::Claude, HookEvent::PostToolUse, &input);
        assert!(out.stdout.is_empty());
        assert_eq!(out.audit["decision"], "pass");
    }

    #[test]
    fn claude_clean_output_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let out = handle_hook(
            HookProvider::Claude,
            HookEvent::PostToolUse,
            &bash_post(dir.path(), "src/main.rs:fn main() {}"),
        );
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn cursor_read_of_env_is_denied() {
        let dir = tempfile::tempdir().unwrap();
        let input = json!({
            "hook_event_name": "beforeReadFile",
            "file_path": dir.path().join(".env").to_string_lossy(),
            "workspace_roots": [dir.path().to_string_lossy()],
        });
        let out = handle_hook(HookProvider::Cursor, HookEvent::BeforeReadFile, &input);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["permission"], "deny");
        assert_eq!(out.audit["decision"], "deny");
    }

    #[test]
    fn cursor_shell_cat_env_is_denied_and_ls_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let shell = |command: &str| {
            let input = json!({
                "hook_event_name": "beforeShellExecution",
                "command": command,
                "cwd": dir.path().to_string_lossy(),
                "workspace_roots": [dir.path().to_string_lossy()],
            });
            let out = handle_hook(HookProvider::Cursor, HookEvent::BeforeShellExecution, &input);
            serde_json::from_str::<Value>(&out.stdout).unwrap()["permission"].clone()
        };
        assert_eq!(shell("cat .env"), "deny");
        assert_eq!(shell("ls"), "allow");
    }

    #[test]
    fn garbage_input_passes() {
        let out = handle_hook(HookProvider::Cursor, HookEvent::BeforeReadFile, &Value::Null);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["permission"], "allow");
        assert_eq!(out.audit["decision"], "pass");
        assert_eq!(out.audit["reason"], "unrecognized input");
        let out = handle_hook(HookProvider::Claude, HookEvent::PostToolUse, &json!({"x": 1}));
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn append_audit_writes_one_line() {
        let dir = tempfile::tempdir().unwrap();
        append_audit(dir.path(), &json!({"decision": "deny"}));
        append_audit(dir.path(), &json!({"decision": "allow"}));
        let body =
            std::fs::read_to_string(dir.path().join(".gaviero/egress/hooks.ndjson")).unwrap();
        assert_eq!(body.lines().count(), 2);
    }

    #[test]
    fn grep_line_path_skips_windows_drive() {
        assert_eq!(grep_line_path(r"C:\w\.env:K=v"), Some(r"C:\w\.env"));
        assert_eq!(grep_line_path("./.env:K=v"), Some("./.env"));
        assert_eq!(grep_line_path("no colon here"), None);
    }
}
