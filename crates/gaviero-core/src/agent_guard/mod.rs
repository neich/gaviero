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
//! TUI chat's `/add-dir` adds per-conversation entries to the same read-only
//! set ([`SensitivePolicy::with_session_read_folders`], checked by
//! [`check_session_read_folder`]); `/reset`, `/clear` and `/handoff` drop them.
//!
//! `agent.permissions.scratchFolders` ([`scratch_folders`]) widens the
//! writable set with folders outside the workspace where the agent works
//! without prompts — downloads, clones, worktrees, experiments. Same
//! exclusions as the read-only folders (no turn capture, indexing, memory
//! scope or `GAVIERO.md`), but reads and writes are both allowed; the
//! sensitive-file denies still apply inside them.

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

/// The `agent.permissions.scratchFolders` a session over `workspace_root`
/// plus `additional_roots` may read and write: existing directories,
/// deduplicated, that neither lie inside nor contain a workspace folder. An
/// entry inside one is already writable (and its writes reviewed); an entry
/// containing one would let writes to that folder escape turn review. An
/// entry inside a `read.extraFolders` folder is skipped too: the read-only
/// rule wins there. Each skip logs a warning.
///
/// Unlike workspace folders, scratch folders get no turn capture, indexing,
/// memory scope or `GAVIERO.md` discovery — they hold downloaded packages,
/// clones and worktrees, which must not steer the agent or flood the review.
pub fn scratch_folders(
    policy: &SensitivePolicy,
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
) -> Vec<PathBuf> {
    let roots = root_keys(workspace_root, additional_roots);
    let read_only: Vec<String> = policy.read_folders().iter().map(|f| path_key(f)).collect();
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for folder in policy.scratch_folders() {
        let key = path_key(folder);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        let skip = if !folder.is_dir() {
            Some("is not a directory")
        } else if roots.iter().any(|r| within(&key, r) || within(r, &key)) {
            Some("overlaps a workspace folder")
        } else if read_only.iter().any(|r| within(&key, r)) {
            Some("lies inside an agent.permissions.read.extraFolders entry")
        } else {
            None
        };
        if let Some(reason) = skip {
            tracing::warn!(
                target: "agent_guard",
                folder = %folder.display(),
                "agent.permissions.scratchFolders entry {reason}; skipped"
            );
            continue;
        }
        out.push(folder.clone());
    }
    out
}

/// [`scratch_folders`] for the policy governing `workspace_root`
/// ([`SensitivePolicy::resolve`]).
pub fn scratch_folders_for(
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
) -> Vec<PathBuf> {
    scratch_folders(
        &SensitivePolicy::resolve(workspace_root),
        workspace_root,
        additional_roots,
    )
}

/// The system-prompt block naming the scratch folders, or `None` when there
/// are none.
pub fn scratch_folders_block(folders: &[PathBuf]) -> Option<String> {
    if folders.is_empty() {
        return None;
    }
    let mut out = String::from(
        "<scratch_folders>\n\
         These folders are outside the workspace and yours to use without asking: \
         read, create, edit and delete files there, and run commands in them. \
         Put downloaded packages, cloned repositories, git worktrees, throwaway \
         scripts and experiments here instead of elsewhere outside the workspace. \
         Changes here are not part of the user's review, so never use them to \
         change the workspace itself.\n",
    );
    for folder in folders {
        out.push_str(&format!("- {}\n", folder.display()));
    }
    out.push_str("</scratch_folders>");
    Some(out)
}

/// The `<read_only_folders>` and `<scratch_folders>` blocks for a session
/// over `workspace_root` plus `additional_roots`, joined, or `None` when
/// neither applies.
pub fn outside_folders_prompt(
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
) -> Option<String> {
    outside_folders_prompt_with_grants(workspace_root, additional_roots, &[])
}

/// [`outside_folders_prompt`] with the conversation's `/add-dir` grants
/// ([`SensitivePolicy::with_session_read_folders`]) listed among the
/// read-only folders.
pub fn outside_folders_prompt_with_grants(
    workspace_root: &Path,
    additional_roots: &[impl AsRef<Path>],
    session_read_folders: &[PathBuf],
) -> Option<String> {
    let policy =
        SensitivePolicy::resolve(workspace_root).with_session_read_folders(session_read_folders);
    let blocks: Vec<String> = [
        read_only_folders_block(&read_only_folders(&policy, workspace_root, additional_roots)),
        scratch_folders_block(&scratch_folders(&policy, workspace_root, additional_roots)),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!blocks.is_empty()).then(|| blocks.join("\n\n"))
}

/// Outcome of checking one `/add-dir` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionReadFolderCheck {
    /// Not readable yet; store this absolute path as a grant.
    Grant(PathBuf),
    /// Already readable; nothing to store. `.1` says why, for the user.
    AlreadyReadable(PathBuf, String),
}

/// Check a `/add-dir` argument for a conversation over `workspace_root`
/// plus `additional_roots`, with `existing` grants already stored. `~`,
/// `~/x`, relative (to `workspace_root`) and `..` forms are resolved
/// lexically. `Err` carries a user-facing reason.
///
/// A folder that is or contains a workspace folder or a scratch folder is
/// refused: granting it would make that folder read-only. One inside a
/// workspace folder, a scratch folder, a read-only folder from the settings
/// or an existing grant is already readable.
pub fn check_session_read_folder(
    raw: &str,
    workspace_root: &Path,
    additional_roots: &[PathBuf],
    existing: &[PathBuf],
) -> Result<SessionReadFolderCheck, String> {
    let mut raw = raw.trim();
    for quote in ['"', '\''] {
        if raw.len() >= 2 && raw.starts_with(quote) && raw.ends_with(quote) {
            raw = raw[1..raw.len() - 1].trim();
            break;
        }
    }
    if raw.is_empty() {
        return Err("usage: /add-dir <path>".to_string());
    }
    let path = crate::scope_enforcer::resolve_read_folder(raw, workspace_root)
        .ok_or_else(|| format!("cannot resolve {raw}"))?;
    if !path.is_dir() {
        return Err(format!("{} is not an existing directory", path.display()));
    }
    let key = path_key(&path);
    let roots: Vec<&Path> = std::iter::once(workspace_root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .filter(|r| !r.as_os_str().is_empty())
        .collect();
    for root in &roots {
        let r = path_key(root);
        if within(&key, &r) {
            return Ok(SessionReadFolderCheck::AlreadyReadable(
                path,
                format!("it is inside workspace folder {}", root.display()),
            ));
        }
        if within(&r, &key) {
            return Err(format!(
                "{} contains workspace folder {}; granting it would make that folder read-only",
                path.display(),
                root.display()
            ));
        }
    }
    let policy = SensitivePolicy::resolve(workspace_root);
    for scratch in scratch_folders(&policy, workspace_root, additional_roots) {
        let s = path_key(&scratch);
        if within(&key, &s) {
            return Ok(SessionReadFolderCheck::AlreadyReadable(
                path,
                format!("it is inside scratch folder {}", scratch.display()),
            ));
        }
        if within(&s, &key) {
            return Err(format!(
                "{} contains scratch folder {}; granting it would make that folder read-only",
                path.display(),
                scratch.display()
            ));
        }
    }
    for folder in read_only_folders(&policy, workspace_root, additional_roots) {
        if within(&key, &path_key(&folder)) {
            return Ok(SessionReadFolderCheck::AlreadyReadable(
                path,
                format!(
                    "it is inside read-only folder {} from .gaviero/settings.json",
                    folder.display()
                ),
            ));
        }
    }
    for grant in existing {
        if within(&key, &path_key(grant)) {
            return Ok(SessionReadFolderCheck::AlreadyReadable(
                path,
                format!(
                    "it is inside {}, already granted to this conversation",
                    grant.display()
                ),
            ));
        }
    }
    Ok(SessionReadFolderCheck::Grant(path))
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

    fn write_scratch_settings(root: &Path, read: &[&str], scratch: &[&str]) {
        std::fs::create_dir_all(root.join(".gaviero")).unwrap();
        let doc = serde_json::json!({
            "agent": {"permissions": {
                "read": {"extraFolders": read},
                "scratchFolders": scratch,
            }}
        });
        std::fs::write(root.join(".gaviero/settings.json"), doc.to_string()).unwrap();
    }

    #[test]
    fn scratch_folders_keep_existing_outside_folders() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let scratch = dir.path().join("scratch");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::create_dir_all(&scratch).unwrap();
        write_scratch_settings(
            &ws,
            &[],
            &[
                scratch.to_str().unwrap(),
                "../scratch",         // duplicate
                "../missing",         // not a directory
                "sub",                // inside the workspace folder
                dir.path().to_str().unwrap(), // contains the workspace folder
                "",
            ],
        );
        assert_eq!(scratch_folders_for(&ws, &[] as &[PathBuf]), vec![scratch.clone()]);
        // Scratch folders are not read-only folders.
        assert!(read_only_folders_for(&ws, &[] as &[PathBuf]).is_empty());
    }

    #[test]
    fn scratch_folders_skip_entries_inside_read_only_folders() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(docs.join("play")).unwrap();
        write_scratch_settings(&ws, &["../docs"], &["../docs/play"]);
        assert!(scratch_folders_for(&ws, &[] as &[PathBuf]).is_empty());
        assert_eq!(read_only_folders_for(&ws, &[] as &[PathBuf]), vec![docs]);
    }

    #[test]
    fn outside_folders_prompt_joins_both_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        assert!(outside_folders_prompt(&ws, &[] as &[PathBuf]).is_none());
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::create_dir_all(dir.path().join("scratch")).unwrap();
        write_scratch_settings(&ws, &["../docs"], &["../scratch"]);
        let prompt = outside_folders_prompt(&ws, &[] as &[PathBuf]).unwrap();
        assert!(prompt.starts_with("<read_only_folders>"));
        assert!(prompt.contains("<scratch_folders>"));
        assert!(prompt.ends_with("</scratch_folders>"));
    }

    /// `<tmp>/ws` (settings `{}`) and an outside `<tmp>/docs`.
    fn grant_layout(dir: &Path) -> (PathBuf, PathBuf) {
        let ws = dir.join("ws");
        let docs = dir.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        write_scratch_settings(&ws, &[], &[]);
        (ws, docs)
    }

    #[test]
    fn check_session_read_folder_grants_an_outside_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, docs) = grant_layout(dir.path());
        assert_eq!(
            check_session_read_folder("../docs", &ws, &[], &[]),
            Ok(SessionReadFolderCheck::Grant(docs.clone()))
        );
        let quoted = format!("\"{}\"", docs.display());
        assert_eq!(
            check_session_read_folder(&quoted, &ws, &[], &[]),
            Ok(SessionReadFolderCheck::Grant(docs))
        );
    }

    #[test]
    fn check_session_read_folder_rejects_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, _) = grant_layout(dir.path());
        let err = check_session_read_folder("../nope", &ws, &[], &[]).unwrap_err();
        assert!(err.contains("not an existing directory"), "{err}");
        assert!(check_session_read_folder("  ", &ws, &[], &[]).is_err());
    }

    #[test]
    fn check_session_read_folder_reports_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, _) = grant_layout(dir.path());
        std::fs::create_dir_all(ws.join("src")).unwrap();
        assert!(matches!(
            check_session_read_folder("src", &ws, &[], &[]),
            Ok(SessionReadFolderCheck::AlreadyReadable(..))
        ));
    }

    #[test]
    fn check_session_read_folder_refuses_a_parent_of_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, _) = grant_layout(dir.path());
        let err = check_session_read_folder("..", &ws, &[], &[]).unwrap_err();
        assert!(err.contains("contains workspace folder"), "{err}");
    }

    #[test]
    fn check_session_read_folder_refuses_a_parent_of_a_scratch_folder() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(dir.path().join("outer").join("scratch")).unwrap();
        write_scratch_settings(&ws, &[], &["../outer/scratch"]);
        let err = check_session_read_folder("../outer", &ws, &[], &[]).unwrap_err();
        assert!(err.contains("contains scratch folder"), "{err}");
        assert!(matches!(
            check_session_read_folder("../outer/scratch", &ws, &[], &[]),
            Ok(SessionReadFolderCheck::AlreadyReadable(..))
        ));
    }

    #[test]
    fn check_session_read_folder_reports_existing_grant() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, docs) = grant_layout(dir.path());
        std::fs::create_dir_all(docs.join("sub")).unwrap();
        assert!(matches!(
            check_session_read_folder("../docs/sub", &ws, &[], &[docs]),
            Ok(SessionReadFolderCheck::AlreadyReadable(..))
        ));
    }

    #[test]
    fn check_session_read_folder_reports_settings_extra_folder() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(dir.path().join("docs").join("sub")).unwrap();
        write_scratch_settings(&ws, &["../docs"], &[]);
        assert!(matches!(
            check_session_read_folder("../docs/sub", &ws, &[], &[]),
            Ok(SessionReadFolderCheck::AlreadyReadable(..))
        ));
    }

    #[test]
    fn outside_folders_prompt_with_grants_lists_the_grant() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, docs) = grant_layout(dir.path());
        let block =
            outside_folders_prompt_with_grants(&ws, &[] as &[PathBuf], std::slice::from_ref(&docs))
                .unwrap();
        assert!(block.starts_with("<read_only_folders>"));
        assert!(block.contains(&docs.display().to_string()));
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
