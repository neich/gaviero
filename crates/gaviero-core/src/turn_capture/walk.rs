//! Stat-only walk of the capture scope.
//!
//! Honours `.gitignore` / `.ignore` files as plain pattern files
//! (`require_git(false)` — no repository needed) plus the host's
//! `files.exclude` globs, and always skips `.git/` and `.gaviero/`. Three
//! carve-outs are added back because an agent changing them is exactly what a
//! reviewer must see:
//!
//! * `<root>/.gaviero/settings.json` — tool permissions live there;
//! * sensitive paths (`.env`, keys, credentials — [`sensitive_match`]), which
//!   are usually gitignored. Every walked directory is swept for direct
//!   children with a sensitive name, and a sensitive directory is listed whole;
//! * the folders in [`CaptureScope::include_ignored`] (the user's
//!   `turnReview.includeIgnored`), walked whole whether or not an ignore file
//!   hides them — `files.exclude` still applies inside them.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
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
    /// Gitignored folders to capture anyway, root-relative (`plans`,
    /// `docs/private`). Each is looked up under every root; a folder missing
    /// from a root is skipped.
    pub include_ignored: Vec<String>,
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

/// Walk every root. Output is keyed by absolute path, so overlapping roots,
/// included folders and the sensitive sweep cannot produce duplicates.
pub fn walk(scope: &CaptureScope) -> BTreeMap<PathBuf, Observed> {
    let mut out = BTreeMap::new();
    for root in &scope.roots {
        walk_root(root, scope, &mut out);
    }
    out
}

fn walk_root(root: &Path, scope: &CaptureScope, out: &mut BTreeMap<PathBuf, Observed>) {
    let excluder = build_excluder(root, &scope.excludes);
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
            if is_dir && always_skipped(entry.file_name()) {
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

    for folder in &scope.include_ignored {
        if let Some(dir) = included_dir(root, folder) {
            walk_included(root, &dir, &excluder, out);
        }
    }

    let settings = root.join(".gaviero").join("settings.json");
    if settings.is_file() {
        push(out, root, &settings);
    }
}

/// `folder` resolved under `root`, if it is a root-relative path to an
/// existing directory. Absolute paths and `..` are refused so an entry can
/// never widen the capture past the workspace folder.
fn included_dir(root: &Path, folder: &str) -> Option<PathBuf> {
    let folder = folder.trim().trim_end_matches(['/', '\\']);
    if folder.is_empty() {
        return None;
    }
    let rel = Path::new(folder);
    if !rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        tracing::warn!(
            "turn capture: ignoring turnReview.includeIgnored entry {folder:?} \
             (must be a path relative to the workspace folder)"
        );
        return None;
    }
    let dir = root.join(rel);
    dir.is_dir().then_some(dir)
}

/// Every file under `dir`, ignore files disregarded; `.git/`, `.gaviero/` and
/// `files.exclude` still prune.
fn walk_included(
    root: &Path,
    dir: &Path,
    excluder: &Option<Gitignore>,
    out: &mut BTreeMap<PathBuf, Observed>,
) {
    let filter_root = root.to_path_buf();
    let filter_excluder = excluder.clone();
    let walker = WalkBuilder::new(dir)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if is_dir && always_skipped(entry.file_name()) {
                return false;
            }
            !is_excluded(&filter_excluder, &filter_root, entry.path(), is_dir)
        })
        .build();
    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|t| t.is_file()) {
            push(out, root, entry.path());
        }
    }
}

fn always_skipped(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| ALWAYS_SKIPPED.contains(&n))
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

    fn write(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    fn scope(root: &Path) -> CaptureScope {
        CaptureScope {
            roots: vec![root.to_path_buf()],
            ..CaptureScope::default()
        }
    }

    #[test]
    fn honours_gitignore_without_a_repo_and_keeps_carve_outs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\n.env\n*.log\n").unwrap();
        for rel in [
            "src/lib.rs",
            "target/debug/out",
            "a.log",
            ".env",
            ".gaviero/settings.json",
            ".gaviero/memory.db",
            ".git/HEAD",
        ] {
            write(root, rel);
        }

        let got = rels(&scope(root));
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
    fn included_ignored_folders_are_walked_whole() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "plans/\ndata/\n*.log\n").unwrap();
        for rel in [
            "plans/next.md",
            "plans/deep/run.log",
            "docs/private/notes.md",
            "data/huge.bin",
        ] {
            write(root, rel);
        }
        std::fs::write(root.join("docs/.gitignore"), "private/\n").unwrap();

        let got = rels(&CaptureScope {
            include_ignored: vec![
                "plans".into(),
                "docs/private/".into(),
                "missing".into(),
                "../outside".into(),
            ],
            ..scope(root)
        });
        assert!(got.contains(&"plans/next.md".to_string()), "{got:?}");
        assert!(got.contains(&"plans/deep/run.log".to_string()), "{got:?}");
        assert!(got.contains(&"docs/private/notes.md".to_string()), "{got:?}");
        assert!(!got.iter().any(|r| r.starts_with("data/")), "{got:?}");
    }

    #[test]
    fn files_exclude_applies_inside_included_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "plans/\n").unwrap();
        write(root, "plans/keep.md");
        write(root, "plans/tmp/x");
        let got = rels(&CaptureScope {
            excludes: vec!["tmp/".into()],
            include_ignored: vec!["plans".into()],
            ..scope(root)
        });
        assert!(got.contains(&"plans/keep.md".to_string()), "{got:?}");
        assert!(!got.iter().any(|r| r.starts_with("plans/tmp/")), "{got:?}");
    }

    #[test]
    fn files_exclude_patterns_prune() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "tmp/deep/x");
        write(root, "keep.txt");
        let got = rels(&CaptureScope {
            excludes: vec!["tmp/".into()],
            ..scope(root)
        });
        assert_eq!(got, vec!["keep.txt".to_string()]);
    }
}
