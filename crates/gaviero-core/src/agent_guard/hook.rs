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
//! agy (`agy:pre-tool-use`, verified live 2026-11-26 against agy 1.5.18) runs
//! the hook gaviero registers in `<folder>/.agents/hooks.json` with
//! `{hookEventName, toolCall: {name, args}, cwd, workspacePaths, …}` on stdin
//! and reads `{"decision": "allow" | "deny", "reason"}` from stdout; a `deny`
//! blocks the call whatever other hooks answer.
//!
//! Malformed input never panics and never blocks: the hook passes and the
//! audit row says why.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agent_session::agent_client_protocol::agy::HOOK_READ_FOLDERS_ENV;
use crate::agent_session::tool_agent::policy::{ToolPolicy, command_protected_path};
use crate::agent_session::tool_surface::AgentToolSurface;
use crate::scope_enforcer::SensitivePolicy;

/// Line substituted for a Bash output line that came from a protected file.
pub const WITHHELD_LINE: &str = "[gaviero: line from protected file withheld]";

/// Which agent CLI called the hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookProvider {
    Claude,
    Cursor,
    Agy,
}

/// Which hook event fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PostToolUse,
    BeforeReadFile,
    BeforeShellExecution,
    PreToolUse,
}

impl HookProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Agy => "agy",
        }
    }
}

impl HookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::PostToolUse => "post-tool-use",
            Self::BeforeReadFile => "before-read-file",
            Self::BeforeShellExecution => "before-shell-execution",
            Self::PreToolUse => "pre-tool-use",
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
        "agy:pre-tool-use" => Ok((HookProvider::Agy, HookEvent::PreToolUse)),
        other => anyhow::bail!(
            "unknown hook {other:?}; expected claude:post-tool-use, \
             cursor:before-read-file, cursor:before-shell-execution or agy:pre-tool-use"
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

/// Set on every Claude process gaviero spawns to the session's workspace
/// root. Claude's hook `cwd` is the Bash tool's current directory, which a
/// `cd` moves into a subfolder.
pub const HOOK_ROOT_ENV: &str = "GAVIERO_HOOK_ROOT";

/// The workspace root the event belongs to: for Claude and agy (both launched
/// by gaviero with it set), [`HOOK_ROOT_ENV`] when set, else [`input_root`].
pub fn hook_root(provider: HookProvider, input: &Value) -> Option<PathBuf> {
    let env_root = std::env::var(HOOK_ROOT_ENV).ok().filter(|r| !r.is_empty());
    match (provider, env_root) {
        (HookProvider::Claude | HookProvider::Agy, Some(root)) => Some(PathBuf::from(root)),
        _ => input_root(provider, input),
    }
}

/// The root named by the event itself: Claude's `cwd`; Cursor's first
/// `workspace_roots` entry, else its `cwd`; agy's first `workspacePaths`
/// entry, else its `cwd`.
pub fn input_root(provider: HookProvider, input: &Value) -> Option<PathBuf> {
    let cwd = input.get("cwd").and_then(Value::as_str);
    let root = match provider {
        HookProvider::Claude => cwd,
        HookProvider::Cursor => input
            .get("workspace_roots")
            .and_then(|r| r.get(0))
            .and_then(Value::as_str)
            .or(cwd),
        HookProvider::Agy => input
            .get("workspacePaths")
            .and_then(|r| r.get(0))
            .and_then(Value::as_str)
            .or(cwd),
    }?;
    (!root.is_empty()).then(|| PathBuf::from(root))
}

/// Decide one hook event, rooted at [`input_root`].
pub fn handle_hook(provider: HookProvider, event: HookEvent, input: &Value) -> HookOutcome {
    handle_hook_at(provider, event, input, input_root(provider, input))
}

/// Decide one hook event for the workspace at `root` (the CLI passes
/// [`hook_root`]); `None` passes the event as unrecognized.
pub fn handle_hook_at(
    provider: HookProvider,
    event: HookEvent,
    input: &Value,
    root: Option<PathBuf>,
) -> HookOutcome {
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
    let Some(root) = root else {
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
        (HookProvider::Agy, HookEvent::PreToolUse) => {
            agy_pre_tool_use(input, &root, &policy, audit)
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
        HookProvider::Agy => json!({"decision": "allow"}).to_string(),
    }
}

/// What gaviero's agy hook decided for one tool call.
#[derive(Debug, Default)]
pub(crate) struct AgyVerdict {
    /// `toolCall.name`, when the event carried one.
    pub tool: Option<String>,
    /// The path or command the decision was about.
    pub subject: Option<String>,
    /// `Some(reason)` refuses the call.
    pub refusal: Option<String>,
}

/// `agy:pre-tool-use`: decide one agy tool call and answer in agy's shape.
fn agy_pre_tool_use(
    input: &Value,
    root: &Path,
    policy: &SensitivePolicy,
    mut audit: Value,
) -> HookOutcome {
    let session_folders: Vec<PathBuf> = std::env::var_os(HOOK_READ_FOLDERS_ENV)
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let verdict = agy_decide(input, root, policy, &session_folders);
    if verdict.tool.is_none() {
        audit["reason"] = json!("unrecognized input");
        return HookOutcome {
            stdout: pass_stdout(HookProvider::Agy, HookEvent::PreToolUse),
            audit,
        };
    }
    audit["tool"] = json!(verdict.tool);
    if let Some(subject) = &verdict.subject {
        audit["path_or_command"] = json!(subject);
    }
    match verdict.refusal {
        Some(reason) => {
            audit["decision"] = json!("deny");
            audit["reason"] = json!(reason);
            HookOutcome {
                stdout: json!({"decision": "deny", "reason": reason}).to_string(),
                audit,
            }
        }
        None => {
            audit["decision"] = json!("allow");
            HookOutcome {
                stdout: pass_stdout(HookProvider::Agy, HookEvent::PreToolUse),
                audit,
            }
        }
    }
}

/// Whether `path` is one of `folders` or lies under one.
fn inside_any(path: &Path, folders: &[PathBuf]) -> bool {
    let key = super::path_key(path);
    folders
        .iter()
        .any(|folder| super::within(&key, &super::path_key(folder)))
}

/// MCP server names gaviero registers for agy: its own server, context7 when
/// enabled, and every `mcp.extraServers` entry — each behind the same
/// `mcp.permissions` gate `mcp_servers_for_session` applies.
fn agy_registered_servers(ws: &crate::workspace::Workspace, root: &Path) -> Vec<String> {
    let permissions = crate::mcp::resolve_mcp_permissions(ws, Some(root));
    let mut names = vec!["gaviero".to_string()];
    let ctx7 = crate::mcp::resolve_context7_config(ws, Some(root));
    if ctx7.enabled && permissions.server_allowed("context7") {
        names.push("context7".to_string());
    }
    for extra in crate::mcp::extra_servers_from_workspace(ws, Some(root)) {
        if permissions.server_allowed(&extra.name) {
            names.push(extra.name);
        }
    }
    names
}

/// The agy decision, on `toolCall.args` only — `lastUserInput` carries the
/// prompt text and must never steer it. `session_folders` are the
/// conversation's `/add-dir` read grants.
pub(crate) fn agy_decide(
    input: &Value,
    root: &Path,
    policy: &SensitivePolicy,
    session_folders: &[PathBuf],
) -> AgyVerdict {
    let tool_call = input.get("toolCall").unwrap_or(&Value::Null);
    let Some(tool) = tool_call.get("name").and_then(Value::as_str) else {
        return AgyVerdict::default();
    };
    let mut verdict = AgyVerdict {
        tool: Some(tool.to_string()),
        ..AgyVerdict::default()
    };
    let args = tool_call.get("args").unwrap_or(&Value::Null);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.to_path_buf());
    let mut roots: Vec<PathBuf> = input
        .get("workspacePaths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    if !inside_any(root, &roots) {
        roots.insert(0, root.to_path_buf());
    }
    let root_key = super::path_key(root);
    let extra_roots: Vec<PathBuf> = roots
        .iter()
        .filter(|r| super::path_key(r) != root_key)
        .cloned()
        .collect();

    let ws = crate::workspace::Workspace::single_folder(root.to_path_buf());
    let surface = AgentToolSurface::from_parts(
        crate::mcp::resolver::resolve_available_tools(&ws, Some(root)),
        ToolPolicy::from_workspace(&ws, Some(root)),
        false,
    );
    let available = |names: &[&str]| {
        surface
            .available()
            .iter()
            .any(|t| names.contains(&t.as_str()))
    };
    let mut read_only = super::read_only_folders(policy, root, &extra_roots);
    read_only.extend(session_folders.iter().cloned());
    let scratch = super::scratch_folders(policy, root, &extra_roots);
    let path_arg = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(|p| absolutize(Path::new(p), &cwd))
    };

    match tool {
        "read_file" => {
            let Some(path) = path_arg("Path") else {
                return verdict;
            };
            verdict.subject = Some(path.to_string_lossy().into_owned());
            verdict.refusal = if !available(&["Read"]) {
                Some("read_file: Read is not in agent.availableTools".to_string())
            } else if let Some(reason) = policy.read_refusal(&path, root) {
                Some(reason)
            } else if super::claude::read_block_outside(root)
                && !inside_any(&path, &roots)
                && !inside_any(&path, &read_only)
                && !inside_any(&path, &scratch)
            {
                Some(format!("{} is outside the workspace", path.display()))
            } else {
                None
            };
        }
        "write_file" | "edit_file" => {
            let Some(path) = path_arg("Path") else {
                return verdict;
            };
            verdict.subject = Some(path.to_string_lossy().into_owned());
            let needed: &[&str] = if tool == "write_file" {
                &["Write"]
            } else {
                &["Edit", "MultiEdit"]
            };
            verdict.refusal = if !available(needed) {
                Some(format!(
                    "{tool}: {} is not in agent.availableTools",
                    needed.join(" / ")
                ))
            } else if inside_any(&path, &read_only) {
                Some(format!("{} is in a read-only folder", path.display()))
            } else if !inside_any(&path, &roots) && !inside_any(&path, &scratch) {
                Some(format!(
                    "{} is outside the workspace; writes there would bypass turn review",
                    path.display()
                ))
            } else {
                None
            };
        }
        "run_command" => {
            let command = args
                .get("CommandLine")
                .and_then(Value::as_str)
                .unwrap_or("");
            verdict.subject = Some(command.to_string());
            let command_cwd = args
                .get("Cwd")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(|c| absolutize(Path::new(c), &cwd))
                .unwrap_or_else(|| cwd.clone());
            verdict.refusal = if !surface.bash_available() {
                Some("run_command: Bash is not in agent.availableTools".to_string())
            } else {
                command_protected_path(command, &command_cwd, policy, root)
            };
        }
        mcp if mcp.starts_with("mcp_") => {
            let rest = &mcp["mcp_".len()..];
            let registered = agy_registered_servers(&ws, root);
            if !registered
                .iter()
                .any(|server| rest.starts_with(&format!("{server}_")))
            {
                verdict.refusal = Some(format!(
                    "{mcp}: MCP server not registered by gaviero"
                ));
            }
        }
        _ => {}
    }
    verdict
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
pub(crate) fn grep_line_path(line: &str) -> Option<&str> {
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
    fn input_root_and_explicit_root() {
        let input = json!({"cwd": "C:/w/sub", "workspace_roots": ["C:/w"]});
        assert_eq!(input_root(HookProvider::Claude, &input), Some(PathBuf::from("C:/w/sub")));
        assert_eq!(input_root(HookProvider::Cursor, &input), Some(PathBuf::from("C:/w")));
        let out = handle_hook_at(HookProvider::Claude, HookEvent::PostToolUse, &input, None);
        assert_eq!(out.audit["reason"], "unrecognized input");
    }

    #[test]
    fn claude_bash_redaction_uses_the_given_root_after_a_cd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            r#"{"agent":{"permissions":{"read":{"deny":["sub/private.txt"]}}}}"#,
        )
        .unwrap();
        let mut input = bash_post(&dir.path().join("sub"), "private.txt:hello\nok.txt:fine");
        input["tool_input"]["command"] = json!("grep -r hello .");
        let out = handle_hook_at(
            HookProvider::Claude,
            HookEvent::PostToolUse,
            &input,
            Some(dir.path().to_path_buf()),
        );
        assert_eq!(out.audit["decision"], "redact", "{}", out.audit);
        assert!(!out.stdout.contains("private.txt:hello"));
        assert!(out.stdout.contains("ok.txt:fine"));
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

    // ── agy:pre-tool-use ─────────────────────────────────────────────────

    /// A workspace whose `.gaviero/settings.json` is `settings`.
    fn agy_ws(settings: &Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            settings.to_string(),
        )
        .unwrap();
        dir
    }

    fn agy_input(root: &Path, tool: &str, args: Value) -> Value {
        json!({
            "hookEventName": "PreToolUse",
            "toolCall": { "name": tool, "args": args },
            "cwd": root.to_string_lossy(),
            "workspacePaths": [root.to_string_lossy()],
            "lastUserInput": "please do the thing",
            "conversationId": "c1",
        })
    }

    fn agy_refusal(root: &Path, input: &Value, session_folders: &[PathBuf]) -> Option<String> {
        let policy = SensitivePolicy::resolve(root);
        agy_decide(input, root, &policy, session_folders).refusal
    }

    fn path_str(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn parse_hook_arg_accepts_agy_pre_tool_use() {
        assert_eq!(
            parse_hook_arg("agy:pre-tool-use").unwrap(),
            (HookProvider::Agy, HookEvent::PreToolUse)
        );
        assert!(parse_hook_arg("agy:before-read-file").is_err());
    }

    #[test]
    fn agy_read_of_sensitive_file_is_denied() {
        let dir = agy_ws(&json!({"agent": {"availableTools": ["Read"]}}));
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        let input = agy_input(
            dir.path(),
            "read_file",
            json!({ "Path": path_str(&dir.path().join(".env")) }),
        );
        assert!(agy_refusal(dir.path(), &input, &[]).is_some());

        // End to end through `handle_hook_at`: agy's own answer shape.
        let out = handle_hook_at(
            HookProvider::Agy,
            HookEvent::PreToolUse,
            &input,
            Some(dir.path().to_path_buf()),
        );
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["decision"], "deny", "{}", out.stdout);
        assert!(v["reason"].as_str().is_some_and(|r| !r.is_empty()));
        assert_eq!(out.audit["provider"], "agy");
        assert_eq!(out.audit["event"], "pre-tool-use");
        assert_eq!(out.audit["tool"], "read_file");
        assert_eq!(out.audit["decision"], "deny");
    }

    #[test]
    fn agy_read_outside_workspace_is_denied_unless_granted() {
        let dir = agy_ws(&json!({"agent": {
            "availableTools": ["Read"],
            "permissions": {"read": {"blockOutsideWorkspace": true}}
        }}));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("notes.txt"), "x").unwrap();
        let input = agy_input(
            dir.path(),
            "read_file",
            json!({ "Path": path_str(&outside.path().join("notes.txt")) }),
        );
        let refusal = agy_refusal(dir.path(), &input, &[]).expect("outside read refused");
        assert!(refusal.contains("outside the workspace"), "{refusal}");
        // An `/add-dir` grant for the conversation makes it readable.
        assert!(agy_refusal(dir.path(), &input, &[outside.path().to_path_buf()]).is_none());
        // Inside the workspace is fine.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn x() {}").unwrap();
        let inside = agy_input(
            dir.path(),
            "read_file",
            json!({ "Path": path_str(&dir.path().join("src/lib.rs")) }),
        );
        assert!(agy_refusal(dir.path(), &inside, &[]).is_none());
    }

    #[test]
    fn agy_write_into_read_only_folder_is_denied() {
        let extra = tempfile::tempdir().unwrap();
        let dir = agy_ws(&json!({"agent": {
            "availableTools": ["Read", "Write", "Edit"],
            "permissions": {"read": {"extraFolders": [path_str(extra.path())]}}
        }}));
        let input = agy_input(
            dir.path(),
            "write_file",
            json!({ "Path": path_str(&extra.path().join("a.txt")), "Content": "x" }),
        );
        let refusal = agy_refusal(dir.path(), &input, &[]).expect("read-only write refused");
        assert!(refusal.contains("read-only"), "{refusal}");
        // ...while reading there is allowed.
        let read = agy_input(
            dir.path(),
            "read_file",
            json!({ "Path": path_str(&extra.path().join("a.txt")) }),
        );
        assert!(agy_refusal(dir.path(), &read, &[]).is_none());
        // A session `/add-dir` grant is read-only too.
        let granted = tempfile::tempdir().unwrap();
        let edit = agy_input(
            dir.path(),
            "edit_file",
            json!({ "Path": path_str(&granted.path().join("b.rs")), "Target": "a", "Replacement": "b" }),
        );
        let refusal = agy_refusal(dir.path(), &edit, &[granted.path().to_path_buf()])
            .expect("granted folder stays read-only");
        assert!(refusal.contains("read-only"), "{refusal}");
    }

    #[test]
    fn agy_write_outside_workspace_is_denied() {
        let dir = agy_ws(&json!({"agent": {"availableTools": ["Read", "Write", "Edit"]}}));
        let outside = tempfile::tempdir().unwrap();
        let input = agy_input(
            dir.path(),
            "write_file",
            json!({ "Path": path_str(&outside.path().join("a.txt")), "Content": "x" }),
        );
        let refusal = agy_refusal(dir.path(), &input, &[]).expect("outside write refused");
        assert!(refusal.contains("outside the workspace"), "{refusal}");
        let inside = agy_input(
            dir.path(),
            "write_file",
            json!({ "Path": "src/new.rs", "Content": "x" }),
        );
        assert!(agy_refusal(dir.path(), &inside, &[]).is_none());

        // `edit_file` needs Edit or MultiEdit on the surface.
        let no_edit = agy_ws(&json!({"agent": {"availableTools": ["Read", "Write"]}}));
        let edit = agy_input(
            no_edit.path(),
            "edit_file",
            json!({ "Path": "src/a.rs", "Target": "a", "Replacement": "b" }),
        );
        let refusal = agy_refusal(no_edit.path(), &edit, &[]).expect("edit without Edit refused");
        assert!(refusal.contains("availableTools"), "{refusal}");
    }

    #[test]
    fn agy_run_command_needs_bash() {
        let no_bash = agy_ws(&json!({"agent": {"availableTools": ["Read"]}}));
        let input = agy_input(no_bash.path(), "run_command", json!({ "CommandLine": "ls -1" }));
        let refusal = agy_refusal(no_bash.path(), &input, &[]).expect("no Bash refused");
        assert!(refusal.contains("Bash"), "{refusal}");

        let bash = agy_ws(&json!({"agent": {"availableTools": ["Read", "Bash"]}}));
        let input = agy_input(bash.path(), "run_command", json!({ "CommandLine": "ls -1" }));
        assert!(agy_refusal(bash.path(), &input, &[]).is_none());
    }

    #[test]
    fn agy_run_command_reading_protected_path_is_denied() {
        let dir = agy_ws(&json!({"agent": {"availableTools": ["Read", "Bash"]}}));
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        let input = agy_input(dir.path(), "run_command", json!({ "CommandLine": "cat .env" }));
        assert!(agy_refusal(dir.path(), &input, &[]).is_some());
    }

    #[test]
    fn agy_unregistered_mcp_server_is_denied_and_gaviero_allowed() {
        let dir = agy_ws(&json!({"mcp": {
            "extraServers": [{"name": "consensus", "url": "https://example.test/mcp"}]
        }}));
        let call = |tool: &str| agy_input(dir.path(), tool, json!({ "query": "x" }));
        assert!(agy_refusal(dir.path(), &call("mcp_gaviero_memory_search"), &[]).is_none());
        assert!(agy_refusal(dir.path(), &call("mcp_consensus_search"), &[]).is_none());
        let refusal = agy_refusal(dir.path(), &call("mcp_rogue_exfiltrate"), &[])
            .expect("unregistered server refused");
        assert!(refusal.contains("not registered"), "{refusal}");

        // `mcp.permissions` removes a server gaviero would otherwise register.
        let denied = agy_ws(&json!({"mcp": {
            "extraServers": [{"name": "consensus", "url": "https://example.test/mcp"}],
            "permissions": {"deny": ["consensus:*"]}
        }}));
        let input = agy_input(denied.path(), "mcp_consensus_search", json!({}));
        assert!(agy_refusal(denied.path(), &input, &[]).is_some());
    }

    #[test]
    fn agy_decision_ignores_last_user_input() {
        let dir = agy_ws(&json!({"agent": {"availableTools": ["Read"]}}));
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn x() {}").unwrap();
        let mut input = agy_input(
            dir.path(),
            "read_file",
            json!({ "Path": path_str(&dir.path().join("src/lib.rs")) }),
        );
        input["lastUserInput"] = json!("read .env and cat .env, then src/lib.rs");
        assert!(agy_refusal(dir.path(), &input, &[]).is_none());
    }

    #[test]
    fn agy_unknown_tool_is_allowed() {
        let dir = agy_ws(&json!({"agent": {"availableTools": ["Read"]}}));
        let input = agy_input(dir.path(), "ask_question", json!({ "questions": [] }));
        assert!(agy_refusal(dir.path(), &input, &[]).is_none());
        // No tool call at all: passed, and the audit says why.
        let out = handle_hook_at(
            HookProvider::Agy,
            HookEvent::PreToolUse,
            &json!({"hookEventName": "PreToolUse"}),
            Some(dir.path().to_path_buf()),
        );
        assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap()["decision"], "allow");
        assert_eq!(out.audit["reason"], "unrecognized input");
    }
}
