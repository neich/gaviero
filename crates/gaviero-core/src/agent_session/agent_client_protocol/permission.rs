//! `session/request_permission` → the workspace tool policy.
//!
//! dsh asks the client before a sensitive action proceeds. In practice that is
//! almost always a **sandbox escalation**: dsh runs every `bash` / `pwsh`
//! command under its own `workspace-write` sandbox (on Windows a Low-integrity
//! restricted token), a command that writes outside the session workspace —
//! cargo's target dir, a tool cache, `%TEMP%` — is denied, and the model
//! retries the exact command with `sandbox_permissions` + `justification`.
//! The other source is an imported Claude Code `PreToolUse` hook answering
//! `ask`.
//!
//! The request itself carries only `toolCall.toolCallId` (dsh 0.2.1-rc.2), so
//! the session records each `tool_call` update and this module looks the call
//! up to recover the tool name and arguments. A shell escalation is decided by
//! the same [`AgentToolSurface::decide_command`] Codex applies to its command
//! approvals — `agent.approvedTools` containing `Bash`, the bash allowlist, the
//! denylist — so an operator who approved Bash for Claude and Codex is not
//! prompted for dsh either. Everything else still asks, with a description
//! that names the tool instead of a bare "ACP permission".

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::acp::client::format_tool_summary;
use crate::agent_session::tool_surface::{AgentToolSurface, CommandDecision};

/// What the host does with one permission request.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PermissionPlan {
    Allow,
    Reject,
    /// Ask the user through `AcpObserver::on_permission_request`.
    Ask {
        tool_name: String,
        description: String,
        input: Value,
    },
}

/// `tool_call` updates of the running turn, by `toolCallId`.
#[derive(Debug, Default)]
pub(super) struct ToolCallLog {
    calls: HashMap<String, Value>,
}

impl ToolCallLog {
    /// Record a `tool_call` update; forget a call once it settles. Any other
    /// update is ignored.
    pub(super) fn observe(&mut self, notification: &Value) {
        let update = notification
            .get("params")
            .and_then(|p| p.get("update"))
            .unwrap_or(notification);
        let Some(id) = update
            .get("toolCallId")
            .or_else(|| update.get("tool_call_id"))
            .and_then(|v| v.as_str())
        else {
            return;
        };
        match update.get("sessionUpdate").and_then(|v| v.as_str()) {
            Some("tool_call") => {
                self.calls.insert(id.to_string(), update.clone());
            }
            Some("tool_call_update")
                if matches!(
                    update.get("status").and_then(|v| v.as_str()),
                    Some("completed" | "failed")
                ) =>
            {
                self.calls.remove(id);
            }
            _ => {}
        }
    }

    pub(super) fn get(&self, id: &str) -> Option<&Value> {
        self.calls.get(id)
    }
}

/// The shell tools: dsh's `bash` off Windows and `pwsh` on it (plus their
/// persistent variants), and agy's `run_command`.
fn is_shell_tool(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "run_command"
        || ["bash", "pwsh", "powershell", "shell"]
            .iter()
            .any(|shell| name == *shell || name.starts_with(&format!("{shell}_")))
}

/// gaviero's own MCP server, as agy (`mcp_gaviero_<tool>`) or Claude
/// (`mcp__gaviero__<tool>`) name its tools. Read-only by construction and
/// auto-approved for Claude through `acp::session::GAVIERO_MCP_RULE`.
fn is_gaviero_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_gaviero_") || name.starts_with("mcp__gaviero__")
}

/// Whether an `agent.approvedTools` rule in Claude's MCP form approves the
/// agy tool `name`: `mcp__<s>__<t>` approves `mcp_<s>_<t>`, and a
/// server-wide `mcp__<s>` approves every `mcp_<s>_*`.
fn approved_mcp_tool(name: &str, approved: &[String]) -> bool {
    let Some(rest) = name.strip_prefix("mcp_") else {
        return false;
    };
    if rest.starts_with('_') {
        return false;
    }
    approved.iter().any(|rule| {
        let Some(rule) = rule.trim().strip_prefix("mcp__") else {
            return false;
        };
        match rule.split_once("__") {
            Some((server, tool)) => !server.is_empty() && rest == format!("{server}_{tool}"),
            None => !rule.is_empty() && rest.starts_with(&format!("{rule}_")),
        }
    })
}

/// Decide one `session/request_permission`. `agent_label` (`dsh`, `agy`)
/// names the agent when the request carries no tool name.
pub(super) fn plan_permission(
    params: &Value,
    log: &ToolCallLog,
    surface: &AgentToolSurface,
    auto_approve: bool,
    workspace_root: &Path,
    agent_label: &str,
) -> PermissionPlan {
    if auto_approve {
        return PermissionPlan::Allow;
    }
    // ACP lets the request carry the tool call inline; dsh sends only its id,
    // so fall back to the recorded `tool_call` update for each field.
    let inline = params.get("toolCall").unwrap_or(&Value::Null);
    let recorded = inline
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .and_then(|id| log.get(id))
        .unwrap_or(&Value::Null);
    let field = |key: &str| {
        inline
            .get(key)
            .filter(|v| !v.is_null())
            .or_else(|| recorded.get(key))
    };
    // agy names the tool (`run_command`, `mcp_<server>_<tool>`) and puts the
    // command text in `title`; dsh only sends a `title`.
    let name = field("name")
        .or_else(|| field("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let raw_input = field("rawInput").cloned().unwrap_or(Value::Null);

    // The ask-user tool *is* the question channel: gaviero's MCP
    // `ask_user_question` parks the overlay itself, so approving the call would
    // put two prompts in a row — the approval first, the question second. There
    // is nothing to approve either: the call's only effect is that question.
    // Same rule as `agent_session/tool_agent/tools/ask.rs` ("the permission
    // channel is the question channel").
    //
    // Matched as a suffix because clients namespace MCP tools differently —
    // `mcp__gaviero__ask_user_question` under Claude, and dsh may prefix its own.
    if name.trim_end().ends_with(crate::mcp::ask::TOOL_ASK_USER_QUESTION) {
        return PermissionPlan::Allow;
    }

    // gaviero's own read-only server is approved for every provider, and a
    // Claude-form `approvedTools` rule approves the same tool under agy's name.
    if is_gaviero_mcp_tool(name) || approved_mcp_tool(name, &surface.policy().approved_tools) {
        return PermissionPlan::Allow;
    }

    if is_shell_tool(name)
        && let Some(command) = raw_input
            .get("command")
            .or_else(|| raw_input.get("CommandLine"))
            .and_then(|v| v.as_str())
    {
        return match surface.decide_command(command) {
            CommandDecision::Allow => PermissionPlan::Allow,
            CommandDecision::Deny => PermissionPlan::Reject,
            CommandDecision::UnattendedFallback => PermissionPlan::Ask {
                tool_name: "Bash".to_string(),
                description: shell_description(command, &raw_input),
                input: json!({ "command": command }),
            },
        };
    }

    let description = if name.is_empty() {
        params
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{agent_label} asks to run a tool call"))
    } else {
        format_tool_summary(name, &raw_input, workspace_root)
    };
    PermissionPlan::Ask {
        tool_name: if name.is_empty() { agent_label } else { name }.to_string(),
        description,
        input: raw_input,
    }
}

/// `command` plus the sandbox widening dsh asks for and the model's reason.
fn shell_description(command: &str, raw_input: &Value) -> String {
    let mode = raw_input.get("sandbox_permissions").and_then(|v| v.as_str());
    let reason = raw_input.get("justification").and_then(|v| v.as_str());
    match (mode, reason) {
        (Some(mode), Some(reason)) => format!("{command}\n(dsh sandbox → {mode}: {reason})"),
        (Some(mode), None) => format!("{command}\n(dsh sandbox → {mode})"),
        _ => command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::tool_agent::policy::ToolPolicy;

    fn surface(approved: &[&str], allow: &[&str], deny: &[&str]) -> AgentToolSurface {
        AgentToolSurface::from_parts(
            Some(vec!["Read".into(), "Write".into(), "Bash".into()]),
            ToolPolicy {
                allowlist: allow.iter().map(|s| s.to_string()).collect(),
                denylist: deny.iter().map(|s| s.to_string()).collect(),
                approved_tools: approved.iter().map(|s| s.to_string()).collect(),
                ..ToolPolicy::default()
            },
            false,
        )
    }

    fn log_with(id: &str, title: &str, raw_input: Value) -> ToolCallLog {
        let mut log = ToolCallLog::default();
        log.observe(&json!({
            "method": "session/update",
            "params": { "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": title,
                "kind": "other",
                "status": "in_progress",
                "rawInput": raw_input,
            }}
        }));
        log
    }

    fn request(id: &str) -> Value {
        json!({
            "sessionId": "s",
            "toolCall": { "toolCallId": id },
            "options": [
                { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" }
            ]
        })
    }

    fn escalation(command: &str) -> Value {
        json!({
            "command": command,
            "sandbox_permissions": "danger-full-access",
            "justification": "cargo writes its target dir outside the workspace"
        })
    }

    #[test]
    fn approved_bash_allows_a_shell_escalation_without_asking() {
        let log = log_with("c1", "pwsh", escalation("cargo build"));
        let plan = plan_permission(
            &request("c1"),
            &log,
            &surface(&["Bash"], &[], &[]),
            false,
            Path::new("."), "dsh",
        );
        assert_eq!(plan, PermissionPlan::Allow);
    }

    #[test]
    fn allowlisted_command_is_allowed_without_bash_approval() {
        let log = log_with("c1", "bash", escalation("cargo test -p x"));
        let plan = plan_permission(
            &request("c1"),
            &log,
            &surface(&[], &["cargo test"], &[]),
            false,
            Path::new("."), "dsh",
        );
        assert_eq!(plan, PermissionPlan::Allow);
    }

    #[test]
    fn denylisted_command_is_rejected_without_asking() {
        let log = log_with("c1", "pwsh", escalation("git push --force origin main"));
        let plan = plan_permission(
            &request("c1"),
            &log,
            &surface(&["Bash"], &[], &["git push"]),
            false,
            Path::new("."), "dsh",
        );
        assert_eq!(plan, PermissionPlan::Reject);
    }

    #[test]
    fn unapproved_command_asks_as_bash_with_the_reason() {
        let log = log_with("c1", "pwsh", escalation("npm install"));
        match plan_permission(
            &request("c1"),
            &log,
            &surface(&[], &[], &[]),
            false,
            Path::new("."), "dsh",
        ) {
            PermissionPlan::Ask {
                tool_name,
                description,
                input,
            } => {
                assert_eq!(tool_name, "Bash");
                assert!(description.starts_with("npm install"), "{description}");
                assert!(description.contains("danger-full-access"), "{description}");
                assert_eq!(input, json!({ "command": "npm install" }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_call_still_asks() {
        let plan = plan_permission(
            &request("missing"),
            &ToolCallLog::default(),
            &surface(&["Bash"], &[], &[]),
            false,
            Path::new("."), "dsh",
        );
        assert!(
            matches!(&plan, PermissionPlan::Ask { tool_name, .. } if tool_name == "dsh"),
            "{plan:?}"
        );
    }

    #[test]
    fn non_shell_tool_asks_under_its_own_name() {
        let log = log_with("c1", "write_file", json!({ "path": "a.rs" }));
        let plan = plan_permission(
            &request("c1"),
            &log,
            &surface(&["Bash", "Write"], &[], &[]),
            false,
            Path::new("."), "dsh",
        );
        assert!(
            matches!(&plan, PermissionPlan::Ask { tool_name, .. } if tool_name == "write_file"),
            "{plan:?}"
        );
    }

    /// `ask_user_question` must never cost two prompts: the tool exists only to
    /// ask, so its own call is allowed straight through and the parked question
    /// is the one thing the user sees.
    #[test]
    fn the_ask_user_tool_is_allowed_without_an_extra_prompt() {
        for title in [
            "ask_user_question",
            "mcp__gaviero__ask_user_question",
            "gaviero/ask_user_question",
        ] {
            let log = log_with("c1", title, json!({ "questions": [] }));
            let plan = plan_permission(
                &request("c1"),
                &log,
                &surface(&[], &[], &[]),
                false,
                Path::new("."), "dsh",
            );
            assert_eq!(plan, PermissionPlan::Allow, "{title} must not prompt");
        }
        // A lookalike that is not the tool still asks.
        let log = log_with("c1", "ask_user_question_helper", json!({}));
        let plan = plan_permission(
            &request("c1"),
            &log,
            &surface(&[], &[], &[]),
            false,
            Path::new("."), "dsh",
        );
        assert!(
            matches!(&plan, PermissionPlan::Ask { tool_name, .. } if tool_name == "ask_user_question_helper"),
            "{plan:?}"
        );
    }

    #[test]
    fn auto_approve_allows_everything() {
        let plan = plan_permission(
            &request("missing"),
            &ToolCallLog::default(),
            &surface(&[], &[], &[]),
            true,
            Path::new("."), "dsh",
        );
        assert_eq!(plan, PermissionPlan::Allow);
    }

    /// agy's live shape (agy 1.5.18): the tool call rides inline with `name`
    /// and `rawInput.CommandLine`; `title` is the command text.
    fn agy_request(name: &str, raw_input: Value) -> Value {
        json!({
            "sessionId": "s",
            "toolCall": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "name": name,
                "title": raw_input.get("CommandLine").and_then(|v| v.as_str()).unwrap_or(name),
                "rawInput": raw_input,
                "status": "pending",
                "kind": "execute"
            },
            "options": [
                { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                { "optionId": "deny", "name": "Deny", "kind": "reject_once" }
            ]
        })
    }

    fn surface_with(available: &[&str], approved: &[&str]) -> AgentToolSurface {
        AgentToolSurface::from_parts(
            Some(available.iter().map(|s| s.to_string()).collect()),
            ToolPolicy {
                approved_tools: approved.iter().map(|s| s.to_string()).collect(),
                ..ToolPolicy::default()
            },
            false,
        )
    }

    #[test]
    fn agy_run_command_uses_commandline_and_policy() {
        // Not on the default allowlist (`cargo test` is), so approval decides.
        let req = agy_request(
            "run_command",
            json!({ "CommandLine": "npm install", "WaitMsBeforeAsync": 5000 }),
        );
        let log = ToolCallLog::default();
        let root = Path::new(".");
        assert_eq!(
            plan_permission(&req, &log, &surface_with(&["Read", "Bash"], &["Bash"]), false, root, "agy"),
            PermissionPlan::Allow
        );
        match plan_permission(&req, &log, &surface_with(&["Read", "Bash"], &[]), false, root, "agy") {
            PermissionPlan::Ask {
                tool_name, input, ..
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(input, json!({ "command": "npm install" }));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan_permission(&req, &log, &surface_with(&["Read"], &["Bash"]), false, root, "agy"),
            PermissionPlan::Reject
        );
    }

    #[test]
    fn gaviero_mcp_tools_are_auto_allowed() {
        for name in ["mcp_gaviero_memory_search", "mcp__gaviero__node_doc"] {
            let req = agy_request(name, json!({ "query": "x" }));
            assert_eq!(
                plan_permission(
                    &req,
                    &ToolCallLog::default(),
                    &surface_with(&["Read"], &[]),
                    false,
                    Path::new("."),
                    "agy"
                ),
                PermissionPlan::Allow,
                "{name}"
            );
        }
    }

    #[test]
    fn approved_claude_mcp_rule_approves_agy_tool_name() {
        let tool = agy_request("mcp_context7_query_docs", json!({}));
        let other = agy_request("mcp_context7_resolve_library_id", json!({}));
        let log = ToolCallLog::default();
        let root = Path::new(".");
        let exact = surface_with(&["Read"], &["mcp__context7__query_docs"]);
        assert_eq!(
            plan_permission(&tool, &log, &exact, false, root, "agy"),
            PermissionPlan::Allow
        );
        assert!(matches!(
            plan_permission(&other, &log, &exact, false, root, "agy"),
            PermissionPlan::Ask { .. }
        ));
        let server_wide = surface_with(&["Read"], &["mcp__context7"]);
        assert_eq!(
            plan_permission(&other, &log, &server_wide, false, root, "agy"),
            PermissionPlan::Allow
        );
    }

    #[test]
    fn unknown_mcp_tool_asks() {
        let req = agy_request("mcp_consensus_search", json!({ "query": "x" }));
        match plan_permission(
            &req,
            &ToolCallLog::default(),
            &surface_with(&["Read", "Bash"], &["Bash", "mcp__context7"]),
            false,
            Path::new("."),
            "agy",
        ) {
            PermissionPlan::Ask { tool_name, .. } => assert_eq!(tool_name, "mcp_consensus_search"),
            other => panic!("{other:?}"),
        }
        // No tool name at all: the agent label names the request.
        match plan_permission(
            &request("missing"),
            &ToolCallLog::default(),
            &surface_with(&["Read"], &[]),
            false,
            Path::new("."),
            "agy",
        ) {
            PermissionPlan::Ask {
                tool_name,
                description,
                ..
            } => {
                assert_eq!(tool_name, "agy");
                assert_eq!(description, "agy asks to run a tool call");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn settled_calls_are_forgotten() {
        let mut log = log_with("c1", "pwsh", escalation("cargo build"));
        log.observe(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "status": "completed"
        }));
        assert!(log.get("c1").is_none());
    }
}
