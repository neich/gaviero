//! `Glob` tool — find files whose path matches a glob pattern.
//!
//! Like [`super::grep`], uses `walkdir` + a glob→regex compile (`glob_to_regex`)
//! rather than the `globset`/`ignore` crates (not in the lockfile). Follow-up:
//! gitignore-aware traversal via `ignore`.

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome, glob_to_regex, is_ignored_dir};

const MAX_RESULTS: usize = 500;

pub struct GlobTool;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Glob",
                "description": "Find files whose workspace-relative path matches a glob (e.g. src/**/*.rs). Returns matching paths, skipping .git/target/node_modules.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Glob pattern, e.g. src/**/*.rs" },
                        "path": { "type": "string", "description": "Directory to search under (optional; default workspace root)." }
                    },
                    "required": ["pattern"]
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'pattern'");
        };
        let glob_re = match glob_to_regex(pattern) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::error(e),
        };
        let search_root = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => match ctx.confine(p) {
                Ok(p) => p,
                Err(e) => return ToolOutcome::error(e.to_string()),
            },
            None => ctx.workspace_root.clone(),
        };
        let workspace_root = ctx.workspace_root.clone();
        let pattern = pattern.to_string();

        let result = tokio::task::spawn_blocking(move || {
            let mut out: Vec<String> = Vec::new();
            for entry in walkdir::WalkDir::new(&search_root)
                .into_iter()
                .filter_entry(|e| !is_ignored_dir(e))
            {
                let Ok(entry) = entry else { continue };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let rel = path.strip_prefix(&workspace_root).unwrap_or(path);
                let rel_str = rel.to_string_lossy().to_string();
                if glob_re.is_match(&rel_str) {
                    out.push(rel_str);
                    if out.len() >= MAX_RESULTS {
                        break;
                    }
                }
            }
            out
        })
        .await;

        match result {
            Ok(files) if files.is_empty() => ToolOutcome::ok(format!("No files match '{pattern}'")),
            Ok(files) => ToolOutcome::ok(files.join("\n")),
            Err(e) => ToolOutcome::error(format!("glob failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use tempfile::tempdir;

    #[tokio::test]
    async fn matches_recursive_pattern() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/inner")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/inner/b.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/c.txt"), "").unwrap();
        let ctx = ToolCtx {
            workspace_root: dir.path().to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: None,
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: false,
            observer: None,
        };

        let out = GlobTool.run(json!({ "pattern": "**/*.rs" }), &ctx).await;
        assert!(!out.is_error);
        assert!(out.content.contains("src/a.rs"));
        assert!(out.content.contains("src/inner/b.rs"));
        assert!(!out.content.contains("c.txt"));
    }
}
