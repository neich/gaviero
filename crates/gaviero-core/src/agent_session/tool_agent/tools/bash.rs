//! `Bash` tool — run a shell command confined to the workspace root (Unit 12).

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use super::{Tool, ToolCtx, ToolOutcome};

pub struct BashTool;

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Bash",
                "description": "Run a shell command in the workspace root. stdout and stderr are combined. Long output is truncated.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute." },
                        "description": { "type": "string", "description": "Short human-readable summary of what the command does (for logs)." }
                    },
                    "required": ["command"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'command'");
        };
        if command.trim().is_empty() {
            return ToolOutcome::error("command must not be empty");
        }

        let observer = match &ctx.observer {
            Some(o) => o.as_ref(),
            None => return ToolOutcome::error("Bash tool is not configured (missing observer)"),
        };

        if let Err(e) = ctx
            .policy
            .gate_bash(command, ctx.auto_approve, observer)
            .await
        {
            return ToolOutcome::error(e);
        }

        match run_command(
            command,
            &ctx.workspace_root,
            ctx.policy.timeout,
            ctx.policy.output_cap,
        )
        .await
        {
            Ok(out) => ToolOutcome::ok(out),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

/// Execute `command` via `bash -c` with `cwd` confined to `workspace_root`.
pub(crate) async fn run_command(
    command: &str,
    workspace_root: &Path,
    timeout: Duration,
    output_cap: usize,
) -> Result<String, String> {
    let child = Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn bash: {e}"))?;

    let wait = async {
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("waiting for command: {e}"))?;
        Ok(output)
    };

    let output = match tokio::time::timeout(timeout, wait).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(format!("command timed out after {}s", timeout.as_secs())),
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    let code = output.status.code().unwrap_or(-1);
    let body = truncate_output(&combined, output_cap);
    Ok(format!("exit code: {code}\n{body}"))
}

/// Keep head + tail when `text` exceeds `cap` bytes (UTF-8 safe on char boundaries).
fn truncate_output(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let half = cap / 2;
    let head_end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i < half)
        .last()
        .unwrap_or(0);
    let tail_start = text
        .char_indices()
        .map(|(i, _)| i)
        .rev()
        .find(|&i| text.len() - i <= half)
        .unwrap_or(text.len());
    format!(
        "{}\n... [{} bytes truncated] ...\n{}",
        &text[..head_end],
        text.len().saturating_sub(cap),
        &text[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::tool_agent::policy::{NoopObserver, ToolPolicy};
    use crate::agent_session::tool_agent::snapshot::TurnSnapshot;
    use crate::types::FileScope;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    fn bash_ctx(root: &Path, auto_approve: bool) -> ToolCtx {
        ToolCtx {
            workspace_root: root.to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: Some(Arc::new(Mutex::new(TurnSnapshot::new()))),
            policy: ToolPolicy::default(),
            auto_approve,
            observer: Some(Arc::new(NoopObserver)),
        }
    }

    #[tokio::test]
    async fn echo_and_ls_succeed() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        let ctx = bash_ctx(dir.path(), true);
        let out = BashTool
            .run(json!({ "command": "echo hello" }), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("hello"));
        assert!(out.content.contains("exit code: 0"));

        let out = BashTool.run(json!({ "command": "ls f.txt" }), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("f.txt"));
    }

    #[tokio::test]
    async fn timeout_kills_sleep() {
        let dir = tempdir().unwrap();
        let mut ctx = bash_ctx(dir.path(), true);
        ctx.policy.timeout = Duration::from_millis(200);
        let out = BashTool
            .run(json!({ "command": "sleep 5" }), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("timed out"));
    }

    #[tokio::test]
    async fn output_truncation() {
        let long = "x".repeat(40_000);
        let t = truncate_output(&long, 30 * 1024);
        assert!(t.len() < long.len());
        assert!(t.contains("truncated"));
    }

    #[tokio::test]
    async fn denylist_blocks_without_auto_approve_bypass() {
        let dir = tempdir().unwrap();
        let ctx = bash_ctx(dir.path(), true);
        let out = BashTool
            .run(json!({ "command": "sudo id" }), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("sudo"));
    }
}
