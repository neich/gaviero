//! `.gaviero/settings.json` → per-provider read policy.
//!
//! Gaviero owns the decision of which files an agent may read; each provider
//! only receives a compiled form of it:
//!
//! * [`hook`] — the decision logic behind `gaviero-cli --hook`, the command
//!   Claude (`PostToolUse` Bash output redaction) and Cursor
//!   (`beforeReadFile` / `beforeShellExecution`) call per tool use.
//! * [`claude`] — the `--settings` layer (Read denies, outside-read block,
//!   the Bash hook) every Claude spawn carries.
//! * [`cursor`] — `.cursor/hooks.json`, written before every Cursor spawn.
//! * [`codex`] — the `thread/start` config (deny-read permission profile,
//!   MXC sandbox on Windows) for the Codex app-server.
//!
//! `agent.permissions.read.extraFolders` ([`read_only_folders`]) widens the
//! readable set with folders that are not workspace folders: no turn
//! capture, indexing, memory scope or `GAVIERO.md` discovery, and no writes.
//! The same list carries the `.gaviero` directory of the settings file
//! itself when it lies outside every workspace folder (workspace mode), so
//! an agent can read the policy that governs it without a permission prompt.

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod hook;

use std::path::{Path, PathBuf};

use crate::scope_enforcer::SensitivePolicy;

/// Comparison key: `/` separators, no trailing slash, case-folded on Windows.
fn path_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(windows) { s.to_ascii_lowercase() } else { s }
}

/// `a` is `b` or lies under it.
fn within(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
}

/// [`path_key`] of every workspace folder.
fn root_keys(workspace_root: &Path, additional_roots: &[impl AsRef<Path>]) -> Vec<String> {
    std::iter::once(workspace_root)
        .chain(additional_roots.iter().map(AsRef::as_ref))
        .filter(|r| !r.as_os_str().is_empty())
        .map(path_key)
        .collect()
}

/// The `agent.permissions.read.extraFolders` a session over `workspace_root`
/// plus `additional_roots` may read: existing directories, deduplicated,
/// that neither lie inside nor contain a workspace folder. An entry inside
/// one is already readable; an entry containing one would make that folder
/// read-only for providers that compile the list into edit denies (Claude),
/// so both are skipped with a warning.
///
/// Last comes the policy's own `.gaviero` directory
/// ([`SensitivePolicy::settings_dir`]) when it is outside every workspace
/// folder. The state floor inside it (`history`, `turns`, `memory.db*`, …)
/// stays denied: Claude anchors those rules at its parent
/// ([`claude::claude_read_rules`]), Codex denies the existing entries by
/// path, and `deepseek:` matches the floor by path component.
pub fn read_only_folders(
    policy: &SensitivePolicy,
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
) -> Vec<PathBuf> {
    let roots = root_keys(workspace_root, additional_roots);
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for folder in policy.read_folders() {
        let key = path_key(folder);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        if !folder.is_dir() {
            tracing::warn!(
                target: "agent_guard",
                folder = %folder.display(),
                "agent.permissions.read.extraFolders entry is not a directory; skipped"
            );
            continue;
        }
        if roots.iter().any(|r| within(&key, r) || within(r, &key)) {
            tracing::warn!(
                target: "agent_guard",
                folder = %folder.display(),
                "agent.permissions.read.extraFolders entry overlaps a workspace folder; skipped"
            );
            continue;
        }
        out.push(folder.clone());
    }
    if let Some(dir) = settings_dir_outside(policy, &roots)
        && !seen.contains(&path_key(&dir))
    {
        out.push(dir);
    }
    out
}

/// The policy's `.gaviero` directory, when it exists and neither lies
/// inside nor contains one of `roots` (comparison keys).
fn settings_dir_outside(policy: &SensitivePolicy, roots: &[String]) -> Option<PathBuf> {
    let dir = policy.settings_dir()?;
    let key = path_key(dir);
    (dir.is_dir() && !roots.iter().any(|r| within(&key, r) || within(r, &key)))
        .then(|| dir.to_path_buf())
}

/// [`read_only_folders`] for the policy governing `workspace_root`
/// ([`SensitivePolicy::resolve`]).
pub fn read_only_folders_for(
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
) -> Vec<PathBuf> {
    read_only_folders(
        &SensitivePolicy::resolve(workspace_root),
        workspace_root,
        additional_roots,
    )
}

/// The system-prompt block naming the read-only folders, or `None` when
/// there are none.
pub fn read_only_folders_block(folders: &[PathBuf]) -> Option<String> {
    if folders.is_empty() {
        return None;
    }
    let mut out = String::from(
        "<read_only_folders>\n\
         These folders are outside the workspace and available for reading only. \
         Read and search them when they help; never create, edit, move or delete \
         files in them, and do not run commands that write there.\n",
    );
    for folder in folders {
        out.push_str(&format!("- {}\n", folder.display()));
    }
    out.push_str("</read_only_folders>");
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn write_settings(root: &Path, folders: &[&str]) {
        std::fs::create_dir_all(root.join(".gaviero")).unwrap();
        let doc = serde_json::json!({
            "agent": {"permissions": {"read": {"extraFolders": folders}}}
        });
        std::fs::write(root.join(".gaviero/settings.json"), doc.to_string()).unwrap();
    }

    #[test]
    fn read_only_folders_keeps_existing_outside_folders() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&docs).unwrap();
        write_settings(
            &ws,
            &[
                docs.to_str().unwrap(),
                "../docs",
                "../missing",
                "",
            ],
        );
        let folders = read_only_folders_for(&ws, &[] as &[PathBuf]);
        assert_eq!(folders, vec![docs]);
    }

    #[test]
    fn read_only_folders_skips_overlaps_with_workspace_folders() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let sibling = dir.path().join("sibling");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::create_dir_all(sibling.join("inner")).unwrap();
        write_settings(
            &ws,
            &[
                dir.path().to_str().unwrap(), // contains every folder
                "sub",                        // inside the primary folder
                sibling.join("inner").to_str().unwrap(),
            ],
        );
        assert!(read_only_folders_for(&ws, &[sibling]).is_empty());
    }

    /// `<root>/ws.gaviero-workspace` listing `<root>/app` and `<root>/lib`,
    /// with the authoritative settings at `<root>/.gaviero/settings.json`.
    pub(crate) fn workspace_layout(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let root = crate::util::fs::simplify_path(&dir.canonicalize().unwrap());
        let app = root.join("app");
        let lib = root.join("lib");
        std::fs::create_dir_all(app.join(".gaviero")).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            root.join("ws.gaviero-workspace"),
            serde_json::json!({"folders": [{"path": app}, {"path": lib}]}).to_string(),
        )
        .unwrap();
        std::fs::write(app.join(".gaviero/settings.json"), "{}").unwrap();
        write_settings(&root, &[]);
        (root, app, lib)
    }

    #[test]
    fn read_only_folders_add_the_workspace_settings_dir_outside_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        let (root, app, lib) = workspace_layout(dir.path());
        assert_eq!(
            read_only_folders_for(&app, std::slice::from_ref(&lib)),
            vec![root.join(".gaviero")]
        );
    }

    #[test]
    fn read_only_folders_skip_a_settings_dir_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        write_settings(&ws, &[]);
        assert!(read_only_folders_for(&ws, &[] as &[PathBuf]).is_empty());
    }

    #[test]
    fn read_only_folders_block_lists_each_folder() {
        assert!(read_only_folders_block(&[]).is_none());
        let block = read_only_folders_block(&[PathBuf::from("/srv/docs")]).unwrap();
        assert!(block.starts_with("<read_only_folders>"));
        assert!(block.contains("- /srv/docs\n"));
        assert!(block.ends_with("</read_only_folders>"));
    }
}
