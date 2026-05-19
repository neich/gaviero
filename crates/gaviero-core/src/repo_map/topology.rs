//! Shallow directory topology for first-turn `<repo_topology>` injection.
//!
//! Filesystem-only walk — no tree-sitter, no [`super::RepoMap::build`].

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::builder::{self, SKIP_DIRS};

/// Settings-driven caps for [`build_folder_topology`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyConfig {
    pub enabled: bool,
    pub max_depth: u8,
    pub max_dirs: usize,
    pub max_token_budget: usize,
}

impl Default for TopologyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 2,
            max_dirs: 64,
            max_token_budget: 600,
        }
    }
}

const PRIORITY_DIRS: &[&str] = &[
    "crates", "src", "lib", "apps", "packages", "docs", "tests",
];

/// Build a directory-only tree for the workspace root (no XML wrapper).
pub fn build_folder_topology(
    workspace: &Path,
    excludes: &[String],
    cfg: &TopologyConfig,
) -> Result<String> {
    if !cfg.enabled {
        return Ok(String::new());
    }

    let workspace_name = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(". (workspace: {workspace_name})"));

    let mut dir_count = 0usize;
    walk_dirs(
        workspace,
        workspace,
        0,
        cfg.max_depth,
        cfg.max_dirs,
        &mut dir_count,
        excludes,
        &mut lines,
    )?;

    if let Some(members_line) = scan_cargo_members(workspace) {
        lines.push(members_line);
    }

    Ok(truncate_to_budget(lines.join("\n"), cfg.max_token_budget))
}

fn walk_dirs(
    workspace: &Path,
    dir: &Path,
    depth: u8,
    max_depth: u8,
    max_dirs: usize,
    dir_count: &mut usize,
    excludes: &[String],
    lines: &mut Vec<String>,
) -> Result<()> {
    if depth >= max_depth || *dir_count >= max_dirs {
        return Ok(());
    }

    let mut children: Vec<PathBuf> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("topology: cannot read {}: {}", dir.display(), e);
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') || SKIP_DIRS.contains(&file_name) {
            continue;
        }
        let rel = path.strip_prefix(workspace).unwrap_or(&path);
        if builder::is_excluded(file_name, rel, excludes) {
            continue;
        }
        children.push(path);
    }

    sort_dir_children(&mut children);

    for child in children {
        if *dir_count >= max_dirs {
            break;
        }
        *dir_count += 1;
        let rel = child.strip_prefix(workspace).unwrap_or(&child);
        let indent = "  ".repeat(depth as usize + 1);
        let name = rel
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        lines.push(format!("{indent}{name}/"));
        walk_dirs(
            workspace,
            &child,
            depth + 1,
            max_depth,
            max_dirs,
            dir_count,
            excludes,
            lines,
        )?;
    }

    Ok(())
}

fn sort_dir_children(children: &mut [PathBuf]) {
    children.sort_by(|a, b| {
        let an = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let bn = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ap = priority_rank(an);
        let bp = priority_rank(bn);
        match ap.cmp(&bp) {
            std::cmp::Ordering::Equal => an.cmp(bn),
            other => other,
        }
    });
}

fn priority_rank(name: &str) -> u8 {
    PRIORITY_DIRS
        .iter()
        .position(|p| *p == name)
        .map(|i| i as u8)
        .unwrap_or(u8::MAX)
}

/// Best-effort line-scanner for `[workspace] members = [...]` in root `Cargo.toml`.
fn scan_cargo_members(workspace: &Path) -> Option<String> {
    let cargo = workspace.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo).ok()?;
    let mut in_workspace = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace = trimmed == "[workspace]";
            continue;
        }
        if !in_workspace {
            continue;
        }
        let rest = trimmed.strip_prefix("members")?.trim().strip_prefix('=')?.trim();
        let members = parse_members_list(rest)?;
        if members.is_empty() {
            return None;
        }
        return Some(format!("members: {}", members.join(", ")));
    }
    None
}

fn parse_members_list(raw: &str) -> Option<Vec<String>> {
    let raw = raw.trim();
    if !raw.starts_with('[') {
        return None;
    }
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    let mut out = Vec::new();
    for part in inner.split(',') {
        let token = part.trim().trim_matches('"').trim_matches('\'');
        if !token.is_empty() {
            out.push(token.to_string());
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn truncate_to_budget(mut body: String, max_token_budget: usize) -> String {
    if max_token_budget == 0 {
        return String::new();
    }
    let max_chars = max_token_budget.saturating_mul(4);
    if body.len() <= max_chars {
        return body;
    }
    let suffix = "… (truncated)";
    let keep = max_chars.saturating_sub(suffix.len());
    body.truncate(keep);
    // Avoid splitting a multibyte char.
    while !body.is_empty() && !body.is_char_boundary(body.len()) {
        body.pop();
    }
    body.push_str(suffix);
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg() -> TopologyConfig {
        TopologyConfig {
            enabled: true,
            max_depth: 3,
            max_dirs: 64,
            max_token_budget: 2000,
        }
    }

    #[test]
    fn builds_fixture_tree_with_priority_sort() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("zzz")).unwrap();
        fs::create_dir_all(dir.path().join("crates/foo")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();

        let out = build_folder_topology(dir.path(), &[], &cfg()).unwrap();
        assert!(out.contains("crates/"));
        assert!(out.contains("src/"));
        let crates_pos = out.find("crates/").unwrap();
        let zzz_pos = out.find("zzz/").unwrap();
        assert!(crates_pos < zzz_pos, "priority dirs should sort first");
    }

    #[test]
    fn respects_max_depth() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c/d")).unwrap();
        let shallow = TopologyConfig {
            max_depth: 1,
            ..cfg()
        };
        let out = build_folder_topology(dir.path(), &[], &shallow).unwrap();
        assert!(out.contains("a/"));
        assert!(!out.contains("b/"));
    }

    #[test]
    fn respects_max_dirs() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            fs::create_dir(dir.path().join(format!("dir{i}"))).unwrap();
        }
        let capped = TopologyConfig {
            max_dirs: 3,
            ..cfg()
        };
        let out = build_folder_topology(dir.path(), &[], &capped).unwrap();
        let dir_lines = out.lines().filter(|l| l.contains('/')).count();
        assert!(dir_lines <= 3);
    }

    #[test]
    fn truncates_to_token_budget() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::create_dir(dir.path().join(format!("very_long_directory_name_{i}"))).unwrap();
        }
        let tiny = TopologyConfig {
            max_token_budget: 50,
            max_dirs: 64,
            ..cfg()
        };
        let out = build_folder_topology(dir.path(), &[], &tiny).unwrap();
        assert!(out.ends_with("… (truncated)"));
        assert!(out.len() <= 50 * 4 + 20);
    }

    #[test]
    fn cargo_members_line_when_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[workspace]
members = ["foo", "bar"]
"#,
        )
        .unwrap();
        let out = build_folder_topology(dir.path(), &[], &cfg()).unwrap();
        assert!(out.contains("members: foo, bar"));
    }

    #[test]
    fn disabled_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let off = TopologyConfig {
            enabled: false,
            ..cfg()
        };
        assert!(build_folder_topology(dir.path(), &[], &off).unwrap().is_empty());
    }

}
