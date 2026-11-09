//! The scope system shared by folder-per-entry catalogs.
//!
//! Skills (`.gaviero/skills/<name>/SKILL.md`) and workflow scripts
//! (`.gaviero/dsl/<name>/<name>.gaviero`) are discovered and named the same way:
//! one directory per scope, a bare name resolves to the nearest definition,
//! and `<source>/<name>` selects a specific one. Those rules live here so the
//! catalogs cannot drift apart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::memory::scope::{SCOPE_GLOBAL, SCOPE_REPO, SCOPE_WORKSPACE, hash_path};
use crate::workspace::Workspace;

/// Catalog-local scope for `skills.extraRoots` entries. Intentionally
/// outside the memory `SCOPE_*` range so extra skills never route through
/// memory DBs. Ranked after global for an unqualified name.
pub(crate) const SCOPE_EXTRA_ROOT: i32 = 10;

/// What the shared ordering needs to know about a catalog entry.
pub(crate) trait ScopedEntry {
    fn entry_name(&self) -> &str;
    fn scope_level(&self) -> i32;
    fn entry_path(&self) -> &Path;
}

/// One directory a catalog scans, before any catalog-specific roots.
pub(crate) struct NativeRoot {
    /// `repo <folder>`, `workspace`, or `global`.
    pub scope: String,
    pub level: i32,
    pub path: PathBuf,
}

/// `.gaviero/<subdir>` of every workspace folder, then the workspace layer's,
/// then `global_dir` — each directory once. The workspace layer is the
/// folder's own `.gaviero` in plain single-folder mode, so it is not listed
/// twice there.
pub(crate) fn native_roots(
    workspace: &Workspace,
    subdir: &str,
    global_dir: &Path,
) -> Vec<NativeRoot> {
    let mut roots = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    for folder in workspace.folders() {
        let path = folder.path.join(".gaviero").join(subdir);
        seen.push(canonicalize_path(&path));
        roots.push(NativeRoot {
            scope: format!("repo {}", folder.display_name()),
            level: SCOPE_REPO,
            path,
        });
    }

    let workspace_dir = workspace.workspace_config_subdir(subdir);
    let workspace_canon = canonicalize_path(&workspace_dir);
    if !seen.contains(&workspace_canon) {
        seen.push(workspace_canon);
        roots.push(NativeRoot {
            scope: "workspace".to_string(),
            level: SCOPE_WORKSPACE,
            path: workspace_dir,
        });
    }

    if !seen.contains(&canonicalize_path(global_dir)) {
        roots.push(NativeRoot {
            scope: "global".to_string(),
            level: SCOPE_GLOBAL,
            path: global_dir.to_path_buf(),
        });
    }

    roots
}

/// Canonical folder root → display label, for [`native_scope_label`].
pub(crate) fn folder_labels(workspace: &Workspace) -> HashMap<PathBuf, String> {
    workspace
        .folders()
        .iter()
        .map(|f| (canonicalize_path(&f.path), f.display_name().to_string()))
        .collect()
}

/// Human-readable source of a native entry: `global`, `workspace`, or the
/// display name of the folder that owns it.
pub(crate) fn native_scope_label(
    level: i32,
    path: &Path,
    folders: &HashMap<PathBuf, String>,
    subdir: &str,
) -> String {
    match level {
        SCOPE_GLOBAL => "global".to_string(),
        SCOPE_WORKSPACE => "workspace".to_string(),
        _ => repo_root_of(path, subdir)
            .and_then(|root| folders.get(&root).cloned())
            .unwrap_or_else(|| "repo".to_string()),
    }
}

/// Repo folder root for an entry under `<repo>/.gaviero/<subdir>/<name>/…`.
pub(crate) fn repo_root_of(path: &Path, subdir: &str) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if is_native_dir(ancestor, subdir) {
            return ancestor
                .parent()
                .and_then(|p| p.parent())
                .map(canonicalize_path);
        }
    }
    None
}

/// True when `path` lies under any `*/.gaviero/<subdir>/` directory.
pub(crate) fn is_under_native_dir(path: &Path, subdir: &str) -> bool {
    path.ancestors().any(|a| is_native_dir(a, subdir))
}

fn is_native_dir(dir: &Path, subdir: &str) -> bool {
    dir.file_name().is_some_and(|n| n == subdir)
        && dir
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == ".gaviero")
}

fn repo_id_of<T: ScopedEntry>(entry: &T, subdir: &str) -> Option<String> {
    if entry.scope_level() != SCOPE_REPO {
        return None;
    }
    repo_root_of(entry.entry_path(), subdir).map(|root| hash_path(&root))
}

/// Nearest definition of one name: the active folder's, then other folders',
/// then workspace, global and extra roots.
pub(crate) fn nearest<'a, T: ScopedEntry>(
    candidates: &'a [T],
    active_repo_id: Option<&str>,
    subdir: &str,
) -> Option<&'a T> {
    if let Some(repo_id) = active_repo_id
        && let Some(active) = candidates
            .iter()
            .find(|c| repo_id_of(*c, subdir).as_deref() == Some(repo_id))
    {
        return Some(active);
    }

    let mut ordered: Vec<&T> = candidates.iter().collect();
    ordered.sort_by_key(|c| (scope_rank(c.scope_level()), c.entry_path().to_path_buf()));
    ordered.into_iter().next()
}

/// Sort for completion and listing: nearest scope first, then by name.
pub(crate) fn sort_nearest_first<T: ScopedEntry>(
    entries: &mut [&T],
    active_repo_id: Option<&str>,
    subdir: &str,
) {
    entries.sort_by(|a, b| {
        let rank = |e: &T| -> (u8, String) {
            let scope = match e.scope_level() {
                SCOPE_REPO => {
                    let active = active_repo_id
                        .is_some_and(|id| repo_id_of(e, subdir).as_deref() == Some(id));
                    if active { 0 } else { 1 }
                }
                level => scope_rank(level) + 1,
            };
            (scope, e.entry_name().to_string())
        };
        rank(a).cmp(&rank(b))
    });
}

fn scope_rank(level: i32) -> u8 {
    match level {
        SCOPE_REPO => 0,
        SCOPE_WORKSPACE => 1,
        SCOPE_GLOBAL => 2,
        SCOPE_EXTRA_ROOT => 3,
        _ => 4,
    }
}

/// `<folder>/<file_name>`, matched case-insensitively; the exact name wins.
pub(crate) fn find_entry_file(folder: &Path, file_name: &str) -> Option<PathBuf> {
    let exact = folder.join(file_name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(file_name))
        })
}

pub(crate) fn canonicalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
