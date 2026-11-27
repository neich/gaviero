//! `Write` / `Edit` / `MultiEdit` tools — Option-B real-tree writes (Unit 9).
//!
//! Each tool snapshots the path on first touch, enforces scope + workspace
//! confinement, then writes directly to disk so the model can read its edits
//! back in the same turn. Post-turn review is the TUI external-change path.

use std::path::Path;

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};

pub struct WriteTool;
pub struct EditTool;
pub struct MultiEditTool;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Write",
                "description": "Write a file in the workspace. Creates parent directories as needed. Overwrites the full file contents.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Path to the file, relative to the workspace root." },
                        "contents": { "type": "string", "description": "Full new file contents." }
                    },
                    "required": ["file_path", "contents"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'file_path'");
        };
        let Some(contents) = args.get("contents").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'contents'");
        };
        match commit_write(ctx, file_path, contents).await {
            Ok(msg) => ToolOutcome::ok(msg),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Edit",
                "description": "Edit a file by replacing one unique occurrence of old_string with new_string. Whitespace must match exactly.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "old_string": { "type": "string", "description": "Exact text to replace (must be unique unless replace_all is true)." },
                        "new_string": { "type": "string" },
                        "replace_all": { "type": "boolean", "description": "Replace every occurrence (default false)." }
                    },
                    "required": ["file_path", "old_string", "new_string"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'file_path'");
        };
        let Some(old_string) = args.get("old_string").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'old_string'");
        };
        let Some(new_string) = args.get("new_string").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'new_string'");
        };
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = match ctx.confine(file_path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e.to_string()),
        };
        if let Err(e) = ctx.check_write(&path) {
            return ToolOutcome::error(e.to_string());
        }

        let original = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(format!("cannot read '{file_path}': {e}")),
        };
        let updated = match apply_single_edit(&original, old_string, new_string, replace_all) {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(e),
        };
        match commit_write(ctx, file_path, &updated).await {
            Ok(msg) => ToolOutcome::ok(msg),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

#[async_trait::async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "MultiEdit"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "MultiEdit",
                "description": "Apply multiple ordered search-and-replace edits to one file. All edits must succeed or none are written.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old_string": { "type": "string" },
                                    "new_string": { "type": "string" },
                                    "replace_all": { "type": "boolean" }
                                },
                                "required": ["old_string", "new_string"]
                            }
                        }
                    },
                    "required": ["file_path", "edits"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'file_path'");
        };
        let Some(edits) = args.get("edits").and_then(|v| v.as_array()) else {
            return ToolOutcome::error("missing required argument 'edits'");
        };
        if edits.is_empty() {
            return ToolOutcome::error("'edits' must not be empty");
        }

        let path = match ctx.confine(file_path) {
            Ok(p) => p,
            Err(e) => return ToolOutcome::error(e.to_string()),
        };
        if let Err(e) = ctx.check_write(&path) {
            return ToolOutcome::error(e.to_string());
        }

        let mut content = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) => return ToolOutcome::error(format!("cannot read '{file_path}': {e}")),
        };
        for (i, edit) in edits.iter().enumerate() {
            let Some(old_string) = edit.get("old_string").and_then(|v| v.as_str()) else {
                return ToolOutcome::error(format!("edit[{i}] missing old_string"));
            };
            let Some(new_string) = edit.get("new_string").and_then(|v| v.as_str()) else {
                return ToolOutcome::error(format!("edit[{i}] missing new_string"));
            };
            let replace_all = edit
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            content = match apply_single_edit(&content, old_string, new_string, replace_all) {
                Ok(s) => s,
                Err(e) => return ToolOutcome::error(format!("edit[{i}]: {e}")),
            };
        }

        match commit_write(ctx, file_path, &content).await {
            Ok(msg) => ToolOutcome::ok(msg),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

/// Unique-match string replace (Claude `Edit` semantics).
pub(crate) fn apply_single_edit(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<String, String> {
    let count = content.matches(old_string).count();
    if count == 0 {
        return Err("old_string not found in file".to_string());
    }
    if !replace_all && count > 1 {
        return Err(format!(
            "old_string is not unique (found {count} occurrences); set replace_all=true to replace all"
        ));
    }
    Ok(if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    })
}

async fn commit_write(ctx: &ToolCtx, file_path: &str, contents: &str) -> Result<String, String> {
    let path = ctx.confine(file_path).map_err(|e| e.to_string())?;
    ctx.check_write(&path).map_err(|e| e.to_string())?;

    snapshot(ctx, &path).await?;

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("creating parent directories: {e}"))?;
    }
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| format!("writing '{}': {e}", file_path))?;

    Ok(format!("Wrote {}", display_rel(&path, &ctx.workspace_root)))
}

async fn snapshot(ctx: &ToolCtx, path: &Path) -> Result<(), String> {
    let Some(snap) = &ctx.snapshot else {
        return Err("write tools require a turn snapshot".to_string());
    };
    snap.lock()
        .await
        .capture_before_write(path)
        .await
        .map_err(|e| e.to_string())
}

fn display_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::tool_agent::snapshot::TurnSnapshot;
    use crate::types::FileScope;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    fn ctx_for(root: &std::path::Path, snap: Arc<Mutex<TurnSnapshot>>) -> ToolCtx {
        ToolCtx {
            workspace_root: root.to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: Some(snap),
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: true,
            observer: None,
            sensitive: crate::scope_enforcer::SensitivePolicy::default(),
        }
    }

    #[test]
    fn edit_requires_unique_match() {
        let content = "aaa\naaa\n";
        assert!(apply_single_edit(content, "aaa", "b", false).is_err());
        let out = apply_single_edit(content, "aaa", "b", true).unwrap();
        assert_eq!(out, "b\nb\n");
    }

    #[tokio::test]
    async fn write_creates_file_on_disk() {
        let dir = tempdir().unwrap();
        let snap = Arc::new(Mutex::new(TurnSnapshot::new()));
        let ctx = ctx_for(dir.path(), snap.clone());
        let out = WriteTool
            .run(json!({ "file_path": "x.txt", "contents": "hello\n" }), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "hello\n"
        );
        assert_eq!(snap.lock().await.touched_paths().len(), 1);
    }

    #[tokio::test]
    async fn edit_changes_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo bar\n").unwrap();
        let snap = Arc::new(Mutex::new(TurnSnapshot::new()));
        let ctx = ctx_for(dir.path(), snap);
        let out = EditTool
            .run(
                json!({
                    "file_path": "a.txt",
                    "old_string": "bar",
                    "new_string": "baz"
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "foo baz\n"
        );
    }

    #[tokio::test]
    async fn blocks_sensitive_write() {
        let dir = tempdir().unwrap();
        let snap = Arc::new(Mutex::new(TurnSnapshot::new()));
        let ctx = ctx_for(dir.path(), snap);
        let out = WriteTool
            .run(json!({ "file_path": ".env", "contents": "X=1\n" }), &ctx)
            .await;
        assert!(out.is_error);
        assert!(!dir.path().join(".env").exists());
    }

    /// A swarm work unit's context: a declared, workspace-relative owned
    /// scope and the policy `tool_agent::swarm::run_turn` resolves.
    fn scoped_ctx(root: &std::path::Path, owned: &[&str]) -> ToolCtx {
        ToolCtx {
            scope: FileScope {
                owned_paths: owned.iter().map(|s| s.to_string()).collect(),
                ..FileScope::default()
            },
            sensitive: crate::scope_enforcer::SensitivePolicy::resolve(root),
            ..ctx_for(root, Arc::new(Mutex::new(TurnSnapshot::new())))
        }
    }

    /// Regression: `confine` hands the scope check an absolute path, while
    /// `scope { owned [...] }` entries are workspace-relative, so every write
    /// of a script agent with a declared scope was refused as "outside agent's
    /// owned scope" — on `deepseek:` and `llamacpp:` alike.
    #[tokio::test]
    async fn writes_inside_a_declared_owned_scope_are_allowed() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("calc.py"), "return a - b\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let ctx = scoped_ctx(dir.path(), &["calc.py", "src/"]);

        let out = EditTool
            .run(
                json!({ "file_path": "calc.py", "old_string": "-", "new_string": "+" }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("calc.py")).unwrap(),
            "return a + b\n"
        );
        // The absolute spelling of an owned path is the same file.
        let abs = dir.path().join("src/lib.rs");
        let out = WriteTool
            .run(json!({ "file_path": abs.to_string_lossy(), "contents": "x\n" }), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(abs.exists());
    }

    #[tokio::test]
    async fn writes_outside_a_declared_owned_scope_stay_refused() {
        let dir = tempdir().unwrap();
        let ctx = scoped_ctx(dir.path(), &["calc.py"]);
        for path in ["other.py", "src/calc.py", "calc.py.bak"] {
            let out = WriteTool
                .run(json!({ "file_path": path, "contents": "x\n" }), &ctx)
                .await;
            assert!(out.is_error, "{path} must be refused");
            assert!(out.content.contains("owned scope"), "{path}: {}", out.content);
            assert!(!dir.path().join(path).exists(), "{path}");
        }
        // The sensitive block-list still applies inside the owned scope.
        let ctx = scoped_ctx(dir.path(), &[".env"]);
        let out = WriteTool
            .run(json!({ "file_path": ".env", "contents": "X=1\n" }), &ctx)
            .await;
        assert!(out.is_error);
        assert!(!dir.path().join(".env").exists());
    }

    /// `agent.permissions.sensitivePaths.allow` globs are workspace-relative
    /// too; an exempted fixture must be writable through the absolute path.
    #[tokio::test]
    async fn sensitive_allow_exemptions_apply_to_tool_writes() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            r#"{ "agent": { "permissions": { "sensitivePaths": { "allow": ["fixtures/**"] } } } }"#,
        )
        .unwrap();
        let ctx = scoped_ctx(dir.path(), &[]);
        let out = WriteTool
            .run(json!({ "file_path": "fixtures/.env", "contents": "X=1\n" }), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        let out = WriteTool
            .run(json!({ "file_path": ".env", "contents": "X=1\n" }), &ctx)
            .await;
        assert!(out.is_error, "only the exempted folder is writable");
    }
}
