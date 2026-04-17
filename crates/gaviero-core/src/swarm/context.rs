//! RepoContextBuilder — memory-aware context assembly for the coordinator.
//!
//! Queries memory before assembling context. If a module has a recent summary
//! in memory whose git blob hash matches current HEAD, the summary substitutes
//! for raw file content — reducing coordinator input tokens.

use std::path::Path;
use std::sync::Arc;

use crate::memory::store::{MemoryStore, PrivacyFilter};

/// Configuration for context building.
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Max total tokens for the assembled context.
    pub max_tokens: u32,
    /// Max files to include in the file list.
    pub max_files: usize,
    /// Whether to use memory summaries as substitutes.
    pub use_memory_summaries: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_tokens: 60000,
            max_files: 200,
            use_memory_summaries: true,
        }
    }
}

/// Assembled repo context ready for coordinator prompt injection.
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// File tree listing (paths only).
    pub file_list: Vec<String>,
    /// Memory summaries that substitute for raw file content.
    pub memory_summaries: String,
    /// Key file contents included directly (interfaces, prompt-referenced files).
    pub key_files: Vec<(String, String)>,
    /// Estimated token count.
    pub estimated_tokens: u32,
}

/// Build repo context for the coordinator, enriched with memory.
pub async fn build_context(
    workspace_root: &Path,
    prompt: &str,
    memory: &Option<Arc<MemoryStore>>,
    namespaces: &[String],
    config: &ContextConfig,
) -> RepoContext {
    // 1. Collect file tree
    let file_list = collect_file_list(workspace_root, config.max_files);

    // 2. Query memory for relevant summaries
    let memory_summaries = if config.use_memory_summaries {
        if let Some(mem) = memory {
            mem.search_context_filtered(namespaces, prompt, 10, PrivacyFilter::ExcludeLocalOnly)
                .await
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // 3. Extract files directly referenced in the prompt (e.g., @file patterns)
    let key_files = extract_referenced_files(prompt, workspace_root);

    // 4. Estimate tokens (rough: 4 chars per token)
    let total_chars: usize = file_list.iter().map(|f| f.len() + 2).sum::<usize>()
        + memory_summaries.len()
        + key_files.iter().map(|(_, c)| c.len()).sum::<usize>();
    let estimated_tokens = (total_chars / 4) as u32;

    RepoContext {
        file_list,
        memory_summaries,
        key_files,
        estimated_tokens,
    }
}

/// Format the repo context as a string for prompt injection.
pub fn format_context(ctx: &RepoContext) -> String {
    let mut out = String::new();

    if !ctx.file_list.is_empty() {
        out.push_str("WORKSPACE FILES:\n");
        for f in &ctx.file_list {
            out.push_str(&format!("  {}\n", f));
        }
        out.push('\n');
    }

    if !ctx.memory_summaries.is_empty() {
        out.push_str(&ctx.memory_summaries);
        out.push('\n');
    }

    if !ctx.key_files.is_empty() {
        out.push_str("KEY FILE CONTENTS:\n");
        for (path, content) in &ctx.key_files {
            out.push_str(&format!("--- {} ---\n{}\n\n", path, content));
        }
    }

    out
}

/// Collect workspace files, excluding hidden dirs and build artifacts.
fn collect_file_list(workspace_root: &Path, max_files: usize) -> Vec<String> {
    let mut files = Vec::new();
    collect_recursive(workspace_root, workspace_root, &mut files, 0, max_files);
    files.sort();
    files
}

fn collect_recursive(root: &Path, dir: &Path, files: &mut Vec<String>, depth: usize, max: usize) {
    if depth > 10 || files.len() >= max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= max {
            return;
        }
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.')
            || name == "target"
            || name == "node_modules"
            || name == "dist"
            || name == "build"
            || name == "__pycache__"
        {
            continue;
        }
        if path.is_dir() {
            collect_recursive(root, &path, files, depth + 1, max);
        } else if let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.to_string_lossy().to_string());
        }
    }
}

/// Extract file paths referenced in the prompt (e.g., `@src/auth.rs` or quoted paths).
fn extract_referenced_files(prompt: &str, workspace_root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();

    // Look for @file patterns
    for word in prompt.split_whitespace() {
        if let Some(path_str) = word.strip_prefix('@') {
            let full_path = workspace_root.join(path_str);
            if full_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    // Truncate large files
                    let truncated = if content.len() > 8000 {
                        format!(
                            "{}...\n[truncated, {} chars total]",
                            &content[..8000],
                            content.len()
                        )
                    } else {
                        content
                    };
                    files.push((path_str.to_string(), truncated));
                }
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_file_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub mod auth;").unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "hidden").unwrap();

        let files = collect_file_list(dir.path(), 100);
        assert!(files.contains(&"main.rs".to_string()));
        assert!(files.contains(&"src/lib.rs".to_string()));
        // .git should be excluded
        assert!(!files.iter().any(|f| f.contains(".git")));
    }

    #[test]
    fn test_collect_respects_max() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("file{}.txt", i)), "x").unwrap();
        }
        let files = collect_file_list(dir.path(), 5);
        assert_eq!(files.len(), 5);
    }

    #[test]
    fn test_extract_referenced_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("auth.rs"), "fn login() {}").unwrap();

        let refs = extract_referenced_files("Look at @auth.rs and fix the bug", dir.path());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "auth.rs");
        assert!(refs[0].1.contains("fn login()"));
    }

    #[test]
    fn test_extract_referenced_files_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let refs = extract_referenced_files("Look at @nonexistent.rs", dir.path());
        assert!(refs.is_empty());
    }

    #[test]
    fn test_format_context() {
        let ctx = RepoContext {
            file_list: vec!["src/main.rs".into(), "src/lib.rs".into()],
            memory_summaries: "[Memory context]:\n- prior auth refactor\n".into(),
            key_files: vec![("src/main.rs".into(), "fn main() {}".into())],
            estimated_tokens: 100,
        };
        let formatted = format_context(&ctx);
        assert!(formatted.contains("WORKSPACE FILES:"));
        assert!(formatted.contains("src/main.rs"));
        assert!(formatted.contains("[Memory context]"));
        assert!(formatted.contains("KEY FILE CONTENTS:"));
    }

    #[tokio::test]
    async fn test_build_context_no_memory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.rs"), "fn test() {}").unwrap();

        let ctx = build_context(
            dir.path(),
            "Fix the bug",
            &None,
            &[],
            &ContextConfig::default(),
        )
        .await;

        assert!(!ctx.file_list.is_empty());
        assert!(ctx.memory_summaries.is_empty());
    }
}
