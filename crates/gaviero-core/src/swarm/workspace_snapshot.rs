//! Whole-workspace fingerprints, used to tell whether anything changed
//! between two loop iterations.
//!
//! This generalizes [`snapshot_owned_files`](super::pipeline) from
//! per-agent owned globs to the entire workspace root: same bounded,
//! pruned walk, same (len, mtime) fingerprints. The walk and its prune
//! predicate are shared with that function so the two cannot drift.
//!
//! Files alone are not enough. A loop running under `branch_chain none`
//! overwrites `gaviero/{id}` every iteration, so a probe like
//! `git show gaviero/foo:report.md` reads different content on each pass
//! while the working tree never moves. Recording the `gaviero/*` branch
//! tips alongside the file fingerprints catches that without a status
//! walk. A root that is not a git repository simply has no tips to
//! record — that is the document-mode path, not an error.
//!
//! Deliberately *not* content digests: a formatter that rewrites a file
//! byte-identically bumps its mtime and makes this report "changed".
//! The only consumer (loop-gate dedup) reacts to "changed" by re-running
//! the probe, so a false "changed" costs one probe and a false
//! "unchanged" would skip a gate that should have run. The cheap
//! fingerprint errs in the safe direction.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

/// Depth bound for workspace walks.
///
/// Deep enough for source and artefact trees, shallow enough that an
/// unbounded walk of a large checkout — repeated per iteration — cannot
/// cost more than the check saves.
pub(crate) const MAX_WALK_DEPTH: usize = 8;

/// Directory names never worth walking: build output and VCS internals
/// that are either enormous, machine-generated, or both.
const PRUNED_DIRS: [&str; 3] = [".git", "target", "node_modules"];

/// Prune predicate for [`pruned_walk`].
///
/// Depth 0 is the walk root itself and is never pruned on its own name —
/// a workspace that happens to be called `target` still gets walked.
pub(crate) fn is_not_pruned(entry: &walkdir::DirEntry) -> bool {
    entry.depth() == 0
        || entry
            .file_name()
            .to_str()
            .map(|n| !PRUNED_DIRS.contains(&n))
            .unwrap_or(false)
}

/// A depth-bounded walk of `root` with [`PRUNED_DIRS`] skipped.
///
/// Shared by [`WorkspaceSnapshot::capture`] and `snapshot_owned_files`
/// so both see exactly the same set of files.
pub(crate) fn pruned_walk(root: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    walkdir::WalkDir::new(root)
        .max_depth(MAX_WALK_DEPTH)
        .into_iter()
        .filter_entry(is_not_pruned)
}

/// Workspace-relative path → (length, mtime).
pub type FileFingerprints = BTreeMap<String, (u64, Option<SystemTime>)>;

/// A comparable fingerprint of a workspace at one point in time.
///
/// Equality is the whole point: two snapshots that compare equal mean
/// nothing a loop agent could have written has changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    files: FileFingerprints,
    /// `None` when `root` is not a git repository.
    git_refs: Option<BTreeMap<String, String>>,
}

impl WorkspaceSnapshot {
    /// Fingerprint every file under `root`, plus the git refs a loop can
    /// move if `root` is a repository.
    ///
    /// Returns `Err` if the tree cannot be walked or a file's metadata
    /// cannot be read. A partial snapshot is worse than none: it could
    /// compare equal to a later one and skip a gate that should have
    /// run, so callers get a loud failure and decide for themselves
    /// (the dedup path treats it as "cannot dedup, run the probe").
    pub fn capture(root: &Path) -> Result<Self> {
        let mut files = FileFingerprints::new();

        for entry in pruned_walk(root) {
            let entry = entry.with_context(|| format!("walking workspace {}", root.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let meta = entry
                .metadata()
                .with_context(|| format!("reading metadata for {}", entry.path().display()))?;
            files.insert(rel, (meta.len(), meta.modified().ok()));
        }

        Ok(Self {
            files,
            git_refs: capture_git_refs(root)?,
        })
    }

    /// The file fingerprints captured, keyed by workspace-relative path.
    pub fn files(&self) -> &FileFingerprints {
        &self.files
    }

    /// Captured git refs, or `None` if the root is not a repository.
    pub fn git_refs(&self) -> Option<&BTreeMap<String, String>> {
        self.git_refs.as_ref()
    }
}

/// Record HEAD and every `refs/heads/gaviero/*` tip.
///
/// Uses `Repository::open` rather than `discover` on purpose: a
/// workspace nested inside some unrelated outer repository must not
/// inherit that repository's refs, or every snapshot would track commits
/// no loop agent can make.
///
/// `Ok(None)` means `root` is not a repository at all — the
/// document-mode path, not a failure. Anything that would yield a
/// *partial* view of the refs is an error, for the same reason a partial
/// file walk is: a ref we failed to read is a ref that can move without
/// the snapshot noticing, and a snapshot that wrongly compares equal
/// skips a gate that should have run.
///
/// The one tolerated gap is an unborn HEAD, which is what a freshly
/// initialised repository has and is genuinely "no commit yet".
fn capture_git_refs(root: &Path) -> Result<Option<BTreeMap<String, String>>> {
    let repo = match git2::Repository::open(root) {
        Ok(repo) => repo,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => {
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("opening git repository at {}", root.display()));
        }
    };

    let mut refs = BTreeMap::new();

    match repo.head() {
        Ok(head) => {
            if let Some(oid) = head.target() {
                refs.insert("HEAD".to_string(), oid.to_string());
            }
        }
        Err(e)
            if matches!(
                e.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) => {}
        Err(e) => return Err(anyhow::Error::new(e)).context("reading HEAD"),
    }

    let branches = repo
        .branches(Some(git2::BranchType::Local))
        .context("enumerating local branches")?;
    for entry in branches {
        let (branch, _) = entry.context("reading a local branch")?;
        let reference = branch.get();
        // Match on the full refname bytes rather than `Branch::name`,
        // which reports `Ok(None)` for a non-UTF-8 name and would
        // silently drop it from the snapshot.
        let name = String::from_utf8_lossy(reference.name_bytes()).into_owned();
        if !name.starts_with("refs/heads/gaviero/") {
            continue;
        }
        if let Some(oid) = reference.target() {
            refs.insert(name, oid.to_string());
        }
    }

    Ok(Some(refs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A repo with one commit, local identity configured, and autocrlf
    /// off — mirrors `git::tests::init_test_repo` so Windows CI agrees
    /// with libgit2 on working-tree bytes.
    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test").unwrap();
            config.set_str("user.email", "test@test.com").unwrap();
            config.set_str("core.autocrlf", "false").unwrap();
        }
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            std::fs::write(dir.path().join("tracked.md"), "original\n").unwrap();
            index.add_path(Path::new("tracked.md")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        {
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }
        dir
    }

    #[test]
    fn equal_states_compare_equal() {
        let dir = init_repo();
        let a = WorkspaceSnapshot::capture(dir.path()).unwrap();
        let b = WorkspaceSnapshot::capture(dir.path()).unwrap();
        assert_eq!(a, b, "two captures of an untouched workspace must match");
    }

    #[test]
    fn detects_tracked_file_edit() {
        let dir = init_repo();
        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();

        std::fs::write(dir.path().join("tracked.md"), "original, plus more\n").unwrap();
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn detects_untracked_file_add() {
        let dir = init_repo();
        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();

        std::fs::write(dir.path().join("brand-new.md"), "hello\n").unwrap();
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert_ne!(before, after);
        assert!(after.files().contains_key("brand-new.md"));
    }

    #[test]
    fn detects_gitignored_file_edit() {
        let dir = init_repo();
        std::fs::write(dir.path().join(".gitignore"), "ignored.log\n").unwrap();
        std::fs::write(dir.path().join("ignored.log"), "line one\n").unwrap();
        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();

        // The walk never consults gitignore, so an ignored artefact
        // counts as workspace change like any other file.
        std::fs::write(dir.path().join("ignored.log"), "line one, line two\n").unwrap();
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn detects_gaviero_branch_tip_move_without_file_change() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("gaviero/foo", &head_commit, true).unwrap();

        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();

        // Commit onto no ref (working tree untouched), reusing the same
        // tree so only the commit OID differs, then move the branch.
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree = head_commit.tree().unwrap();
        let next = repo
            .commit(None, &sig, &sig, "second", &tree, &[&head_commit])
            .unwrap();
        let next_commit = repo.find_commit(next).unwrap();
        repo.branch("gaviero/foo", &next_commit, true).unwrap();

        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert_eq!(
            before.files(),
            after.files(),
            "no working-tree file changed"
        );
        assert_ne!(before, after, "the gaviero/* tip moved");
        assert_eq!(
            after.git_refs().unwrap()["refs/heads/gaviero/foo"],
            next.to_string()
        );
    }

    #[test]
    fn ignores_non_gaviero_branch_tips() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();

        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();
        repo.branch("feature/unrelated", &head_commit, true)
            .unwrap();
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert_eq!(before, after, "only gaviero/* tips are tracked");
    }

    #[test]
    fn non_repo_root_has_no_git_refs_but_still_tracks_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("notes.md"), "draft\n").unwrap();

        let before = WorkspaceSnapshot::capture(dir.path()).unwrap();
        assert!(before.git_refs().is_none(), "document mode has no repo");

        std::fs::write(dir.path().join("notes.md"), "draft, revised\n").unwrap();
        let after = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert!(after.git_refs().is_none());
        assert_ne!(before, after);
    }

    #[test]
    fn nested_repo_root_does_not_inherit_outer_repo_refs() {
        let outer = init_repo();
        let inner = outer.path().join("subdir");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("doc.md"), "x\n").unwrap();

        let snap = WorkspaceSnapshot::capture(&inner).unwrap();

        assert!(
            snap.git_refs().is_none(),
            "Repository::open must not discover the parent repo"
        );
    }

    #[test]
    fn pruned_directories_are_not_walked() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/artifact.bin"), "junk\n").unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").unwrap();

        let snap = WorkspaceSnapshot::capture(dir.path()).unwrap();

        assert!(snap.files().contains_key("src.rs"));
        assert!(
            !snap.files().keys().any(|k| k.starts_with("target/")),
            "target/ must be pruned"
        );
    }

    #[test]
    fn a_repository_with_no_commits_is_not_an_error() {
        // An unborn HEAD is the one tolerated gap in ref capture: it
        // means "no commit yet", not "could not read".
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("draft.md"), "x\n").unwrap();

        let snap = WorkspaceSnapshot::capture(dir.path()).unwrap();

        let refs = snap.git_refs().expect("it is still a repository");
        assert!(!refs.contains_key("HEAD"), "an unborn HEAD has no target");
        assert!(snap.files().contains_key("draft.md"));
    }

    #[test]
    fn missing_root_is_an_error() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");

        assert!(WorkspaceSnapshot::capture(&missing).is_err());
    }
}
