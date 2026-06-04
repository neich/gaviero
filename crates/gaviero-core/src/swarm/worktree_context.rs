//! Inject explicit context files into per-agent git worktrees.
//!
//! Worktrees check out `HEAD` only — uncommitted files and paths outside
//! `--repo` are absent. Callers pass an explicit path list (from the CLI:
//! `--var` / `--prompt-file`); agent `read_only` scopes are not scanned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use super::models::WorkUnit;

/// Result of [`prepare_worktree_read_only_context`].
#[derive(Debug, Default)]
pub struct WorktreeReadOnlyPrep {
    /// `(worktree_relative_path, file_contents)` copied into every worktree.
    pub injections: Vec<(String, String)>,
    /// Rewrite occurrences of the left path with the right (prompts + scopes).
    pub path_rewrites: HashMap<String, String>,
}

/// Resolve a scope `read_only` entry to a host filesystem path.
pub fn resolve_read_only_host_path(workspace_root: &Path, path_pattern: &str) -> Option<PathBuf> {
    let trimmed = path_pattern.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        return None;
    }
    let p = Path::new(trimmed);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    Some(abs)
}

fn canonical_dir(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether a read-only file path lies outside the workspace root.
pub fn read_only_file_is_outside_workspace(workspace_root: &Path, path_pattern: &str) -> bool {
    let Some(host) = resolve_read_only_host_path(workspace_root, path_pattern) else {
        return false;
    };
    if !host.is_file() {
        return false;
    }
    let ws = canonical_dir(workspace_root);
    let host_canon = canonical_dir(&host);
    !host_canon.starts_with(&ws)
}

/// Collect injections and path rewrites for an explicit list of file paths.
///
/// `explicit_paths` is typically built by the CLI from `--var` values and
/// `--prompt-file`. Directory paths and missing files are skipped.
pub fn prepare_worktree_read_only_context(
    workspace_root: &Path,
    explicit_paths: &[String],
) -> Result<WorktreeReadOnlyPrep> {
    let mut prep = WorktreeReadOnlyPrep::default();
    let mut seen_dest: HashSet<String> = HashSet::new();
    let ws_canon = canonical_dir(workspace_root);

    for pattern in explicit_paths {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.ends_with('/') {
            continue;
        }
        let Some(host) = resolve_read_only_host_path(workspace_root, pattern) else {
            continue;
        };
        if !host.is_file() {
            continue;
        }

        let host_canon = canonical_dir(&host);
        let outside = !host_canon.starts_with(&ws_canon);

        let dest_rel = if outside {
            let base = host
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("context.md");
            format!(".gaviero/injected/{base}")
        } else {
            host_canon
                .strip_prefix(&ws_canon)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| pattern.to_string())
        };

        if !outside {
            let rel = dest_rel.as_str();
            if git_tracked_at_head(workspace_root, rel) && !git_dirty_vs_head(workspace_root, rel) {
                continue;
            }
        }

        if seen_dest.insert(dest_rel.clone()) {
            let content = std::fs::read_to_string(&host)
                .with_context(|| format!("reading worktree context file {}", host.display()))?;
            prep.injections.push((dest_rel.clone(), content));
        }

        if outside {
            prep.path_rewrites
                .insert(pattern.to_string(), dest_rel.clone());
            if pattern != host.to_string_lossy().as_ref() {
                prep.path_rewrites
                    .insert(host.to_string_lossy().to_string(), dest_rel);
            }
        }
    }

    Ok(prep)
}

/// Apply path rewrites to work units (prompt text + read_only scope entries).
pub fn apply_worktree_path_rewrites(
    mut work_units: Vec<WorkUnit>,
    rewrites: &HashMap<String, String>,
) -> Vec<WorkUnit> {
    if rewrites.is_empty() {
        return work_units;
    }
    for unit in &mut work_units {
        for (from, to) in rewrites {
            if unit.coordinator_instructions.contains(from) {
                unit.coordinator_instructions = unit.coordinator_instructions.replace(from, to);
            }
            unit.scope.read_only_paths = unit
                .scope
                .read_only_paths
                .iter()
                .map(|p| {
                    if p == from {
                        to.clone()
                    } else {
                        p.clone()
                    }
                })
                .collect();
        }
    }
    work_units
}

fn git_tracked_at_head(workspace_root: &Path, rel: &str) -> bool {
    Command::new("git")
        .args(["cat-file", "-e", &format!("HEAD:{rel}")])
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_dirty_vs_head(workspace_root: &Path, rel: &str) -> bool {
    !Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--", rel])
        .current_dir(workspace_root)
        .status()
        .map(|s| s.success())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn outside_explicit_path_gets_injected_path_and_rewrite() {
        let ws = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let plan = outside.path().join("plan.md");
        fs::write(&plan, "# outside plan").unwrap();

        let prep = prepare_worktree_read_only_context(
            ws.path(),
            &[plan.to_str().unwrap().to_string()],
        )
        .unwrap();
        assert_eq!(prep.injections.len(), 1);
        assert!(prep.injections[0].0.starts_with(".gaviero/injected/"));
        assert!(!prep.path_rewrites.is_empty());
    }

    #[test]
    fn paths_not_in_explicit_list_are_not_injected() {
        let ws = tempdir().unwrap();
        let plan = ws.path().join("only-in-scope.md");
        fs::write(&plan, "x").unwrap();
        let prep = prepare_worktree_read_only_context(ws.path(), &[]).unwrap();
        assert!(prep.injections.is_empty());
    }
}
