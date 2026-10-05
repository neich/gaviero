//! Stat-only walk of the capture scope.
//!
//! Honours `.gitignore` / `.ignore` files as plain pattern files
//! (`require_git(false)` — no repository needed) plus the host's
//! `files.exclude` globs, and always skips `.git/` and `.gaviero/`. Two
//! carve-outs are added back because an agent changing them is exactly what a
//! reviewer must see:
//!
//! * `<root>/.gaviero/settings.json` — tool permissions live there;
//! * sensitive paths (`.env`, keys, credentials — [`sensitive_match`]), which
//!   are usually gitignored. Every walked directory is swept for direct
//!   children with a sensitive name, and a sensitive directory is listed whole.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::scope_enforcer::sensitive_match;

/// What a turn's capture covers.
#[derive(Debug, Clone, Default)]
pub struct CaptureScope {
    /// Workspace folder roots (absolute).
    pub roots: Vec<PathBuf>,
    /// `files.exclude` patterns (gitignore syntax), applied to every root.
    pub excludes: Vec<String>,
}

/// One file seen by the walk.
#[derive(Debug, Clone)]
pub struct Observed {
    pub path: PathBuf,
    pub root: PathBuf,
    /// Root-relative path with `/` separators.
    pub rel: String,
    pub size: u64,
    pub mtime_ns: i64,
}

/// Directory names never descended into.
const ALWAYS_SKIPPED: &[&str] = &[".git", ".gaviero"];

/// Walk every root. Output is keyed by absolute path, so overlapping roots and
/// the sensitive sweep cannot produce duplicates.
pub fn walk(scope: &CaptureScope) -> BTreeMap<PathBuf, Observed> {
    let mut out = BTreeMap::new();
    for root in &scope.roots {
        walk_root(root, &scope.excludes, &mut out);
    }
    out
}

fn walk_root(root: &Path, excludes: &[String], out: &mut BTreeMap<PathBuf, Observed>) {
    let excluder = build_excluder(root, excludes);
    let filter_root = root.to_path_buf();
    let filter_excluder = excluder.clone();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if is_dir
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| ALWAYS_SKIPPED.contains(&n))
            {
                return false;
            }
            !is_excluded(&filter_excluder, &filter_root, entry.path(), is_dir)
        })
        .build();

    for entry in walker.flatten() {
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() {
            push(out, root, entry.path());
        } else if file_type.is_dir() {
            sweep_sensitive(out, root, entry.path());
        }
    }

    let settings = root.join(".gaviero").join("settings.json");
    if settings.is_file() {
        push(out, root, &settings);
    }
}

fn build_excluder(root: &Path, excludes: &[String]) -> Option<Gitignore> {
    if excludes.is_empty() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(root);
    for pattern in excludes {
        if let Err(e) = builder.add_line(None, pattern) {
            tracing::warn!("turn capture: ignoring invalid files.exclude pattern {pattern:?}: {e}");
        }
    }
    builder.build().ok()
}

fn is_excluded(excluder: &Option<Gitignore>, root: &Path, path: &Path, is_dir: bool) -> bool {
    let Some(gi) = excluder else {
        return false;
    };
    match path.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => {
            gi.matched_path_or_any_parents(rel, is_dir).is_ignore()
        }
        _ => false,
    }
}

/// Add the direct children of `dir` whose name is on the sensitive block-list,
/// whether or not an ignore rule hid them. A sensitive *directory* (`.ssh`,
/// `.aws`) is listed recursively.
fn sweep_sensitive(out: &mut BTreeMap<PathBuf, Observed>, root: &Path, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if sensitive_match(Path::new(&name)).is_none() {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_file() {
            push(out, root, &path);
        } else if ft.is_dir() {
            for sub in walkdir::WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if sub.file_type().is_file() {
                    push(out, root, sub.path());
                }
            }
        }
    }
}

fn push(out: &mut BTreeMap<PathBuf, Observed>, root: &Path, path: &Path) {
    if out.contains_key(path) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    out.insert(
        path.to_path_buf(),
        Observed {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
            rel,
            size: meta.len(),
            mtime_ns: mtime_ns(&meta),
        },
    );
}

pub(crate) fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rels(scope: &CaptureScope) -> Vec<String> {
        walk(scope).into_values().map(|o| o.rel).collect()
    }

    #[test]
    fn honours_gitignore_without_a_repo_and_keeps_carve_outs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\n.env\n*.log\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join(".gaviero")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "x").unwrap();
        std::fs::write(root.join("target/debug/out"), "x").unwrap();
        std::fs::write(root.join("a.log"), "x").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join(".gaviero/settings.json"), "{}").unwrap();
        std::fs::write(root.join(".gaviero/memory.db"), "x").unwrap();
        std::fs::write(root.join(".git/HEAD"), "x").unwrap();

        let got = rels(&CaptureScope {
            roots: vec![root.to_path_buf()],
            excludes: vec![],
        });
        assert!(got.contains(&"src/lib.rs".to_string()));
        assert!(got.contains(&".gitignore".to_string()));
        assert!(
            got.contains(&".env".to_string()),
            "sensitive carve-out: {got:?}"
        );
        assert!(got.contains(&".gaviero/settings.json".to_string()));
        assert!(!got.iter().any(|r| r.starts_with("target/")), "{got:?}");
        assert!(!got.contains(&"a.log".to_string()));
        assert!(!got.contains(&".gaviero/memory.db".to_string()));
        assert!(!got.iter().any(|r| r.starts_with(".git/")));
    }

    #[test]
    fn files_exclude_patterns_prune() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("tmp/deep")).unwrap();
        std::fs::write(root.join("tmp/deep/x"), "x").unwrap();
        std::fs::write(root.join("keep.txt"), "x").unwrap();
        let got = rels(&CaptureScope {
            roots: vec![root.to_path_buf()],
            excludes: vec!["tmp/".into()],
        });
        assert_eq!(got, vec!["keep.txt".to_string()]);
    }
}
