//! `Read` tool — read a workspace file with `cat -n`-style line numbers.

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};

const DEFAULT_LIMIT: usize = 2000;

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Read",
                "description": "Read a file from the workspace. Returns the contents with 1-based line numbers (cat -n style). Use offset/limit to page through large files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Path to the file, relative to the workspace root or absolute within it." },
                        "offset": { "type": "integer", "description": "1-based line to start at (optional)." },
                        "limit": { "type": "integer", "description": "Maximum number of lines to return (optional; default 2000)." }
                    },
                    "required": ["file_path"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'file_path'");
        };
        let path = match ctx.confine(file_path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e.to_string()),
        };
        if let Err(e) = ctx.check_read(&path) {
            return ToolOutcome::error(e.to_string());
        }
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return ToolOutcome::error(format!("cannot read '{file_path}': {e}")),
        };

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_LIMIT);

        let mut out = String::new();
        for (i, line) in content.lines().enumerate().skip(offset - 1).take(limit) {
            out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
        }
        if out.is_empty() {
            return ToolOutcome::ok(format!("(file '{file_path}' is empty or offset is past end)"));
        }
        ToolOutcome::ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use tempfile::tempdir;

    fn ctx_for(root: &std::path::Path) -> ToolCtx {
        ToolCtx {
            workspace_root: root.to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: None,
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: false,
            observer: None,
        }
    }

    #[tokio::test]
    async fn reads_with_line_numbers_and_paging() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let ctx = ctx_for(dir.path());

        let out = ReadTool
            .run(json!({ "file_path": "a.txt", "offset": 2, "limit": 2 }), &ctx)
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("     2\ttwo"));
        assert!(out.content.contains("     3\tthree"));
        assert!(!out.content.contains("four"));
    }

    #[tokio::test]
    async fn blocks_sensitive_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        let ctx = ctx_for(dir.path());
        let out = ReadTool.run(json!({ "file_path": ".env" }), &ctx).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn rejects_escape() {
        let dir = tempdir().unwrap();
        let ctx = ctx_for(dir.path());
        let out = ReadTool
            .run(json!({ "file_path": "../../etc/passwd" }), &ctx)
            .await;
        assert!(out.is_error);
    }
}
