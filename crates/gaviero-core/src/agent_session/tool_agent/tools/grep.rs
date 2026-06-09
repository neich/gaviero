//! `Grep` tool — regex search over workspace file contents.
//!
//! Uses `walkdir` + `regex` (both already in the lockfile). This is the
//! offline-buildable stand-in for ripgrep-as-library (`grep-searcher` + the
//! gitignore-aware `ignore` crate), which the plan names as a follow-up. It
//! prunes `.git`/`target`/`node_modules`/`.gaviero` but is not otherwise
//! gitignore-aware.

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome, glob_to_regex, is_ignored_dir};

const MAX_MATCHES: usize = 200;

pub struct GrepTool;

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "Grep",
                "description": "Search file contents with a regular expression. Returns matching lines as `path:line:text`. Skips .git/target/node_modules.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Regular expression to search for." },
                        "path": { "type": "string", "description": "Directory or file to search under (optional; default workspace root)." },
                        "glob": { "type": "string", "description": "Only search files whose workspace-relative path matches this glob (optional, e.g. **/*.rs)." }
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
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::error(format!("invalid regex: {e}")),
        };
        let glob_re = match args.get("glob").and_then(|v| v.as_str()) {
            Some(g) => match glob_to_regex(g) {
                Ok(r) => Some(r),
                Err(e) => return ToolOutcome::error(e),
            },
            None => None,
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
            let mut hits: Vec<String> = Vec::new();
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
                let rel_str = rel.to_string_lossy();
                if let Some(gre) = &glob_re
                    && !gre.is_match(&rel_str)
                {
                    continue;
                }
                // Non-UTF8 / binary files are skipped.
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                for (lineno, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        hits.push(format!("{}:{}:{}", rel_str, lineno + 1, line.trim_end()));
                        if hits.len() >= MAX_MATCHES {
                            return (hits, true);
                        }
                    }
                }
            }
            (hits, false)
        })
        .await;

        match result {
            Ok((hits, _)) if hits.is_empty() => ToolOutcome::ok(format!("No matches for /{pattern}/")),
            Ok((hits, truncated)) => {
                let mut out = hits.join("\n");
                if truncated {
                    out.push_str(&format!("\n... (truncated at {MAX_MATCHES} matches)"));
                }
                ToolOutcome::ok(out)
            }
            Err(e) => ToolOutcome::error(format!("search failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use tempfile::tempdir;

    #[tokio::test]
    async fn finds_matches_with_path_line_text() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::write(dir.path().join("src/b.txt"), "alpha here\n").unwrap();
        let ctx = ToolCtx {
            workspace_root: dir.path().to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: None,
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: false,
            observer: None,
        };

        let out = GrepTool
            .run(json!({ "pattern": "alpha", "glob": "**/*.rs" }), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("src/a.rs:1:fn alpha() {}"));
        // The .txt file is excluded by the glob filter.
        assert!(!out.content.contains("b.txt"));
    }

    #[tokio::test]
    async fn reports_no_matches() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "nothing\n").unwrap();
        let ctx = ToolCtx {
            workspace_root: dir.path().to_path_buf(),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: None,
            policy: crate::agent_session::tool_agent::policy::ToolPolicy::default(),
            auto_approve: false,
            observer: None,
        };
        let out = GrepTool.run(json!({ "pattern": "zzz" }), &ctx).await;
        assert!(!out.is_error);
        assert!(out.content.contains("No matches"));
    }
}
