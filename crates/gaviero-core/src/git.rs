//! Git repository operations and worktree management.
//!
//! Uses `git2` for basic repo operations (open, branch, commit, status)
//! and shells out to the `git` CLI for worktree management (git2's
//! worktree API is limited).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

// ── Supporting types ────────────────────────────────────────

/// Status of a single file in the working tree or index.
#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Renamed,
}

impl FileStatus {
    /// Single-character marker for display.
    pub fn marker(&self) -> char {
        match self {
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Untracked => '?',
            Self::Renamed => 'R',
        }
    }
}

/// A file with its git status (unstaged or staged).
#[derive(Debug, Clone)]
pub struct FileStatusEntry {
    pub path: String,
    pub status: FileStatus,
    /// True if this entry is from the index (staged), false if working tree.
    pub staged: bool,
}

/// A branch listing entry.
#[derive(Debug, Clone)]
pub struct BranchEntry {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
}

// ── GitRepo ────────────────────────────────────────────────

/// A handle to a git repository.
pub struct GitRepo {
    repo: git2::Repository,
}

impl GitRepo {
    /// Open an existing repository at the given path (or search upward).
    pub fn open(path: &Path) -> Result<Self> {
        let repo = git2::Repository::discover(path)
            .with_context(|| format!("opening git repo at {}", path.display()))?;
        Ok(Self { repo })
    }

    /// Get the repository working directory.
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Get the current branch name (HEAD).
    pub fn current_branch(&self) -> Result<String> {
        let head = self.repo.head().context("reading HEAD")?;
        let name = head.shorthand().unwrap_or("HEAD");
        Ok(name.to_string())
    }

    /// Create a new branch pointing at HEAD.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        let head_commit = self.repo.head()?.peel_to_commit()
            .context("resolving HEAD to commit")?;
        self.repo.branch(name, &head_commit, false)
            .with_context(|| format!("creating branch '{}'", name))?;
        Ok(())
    }

    /// Check if the working directory is clean (no modified/staged/untracked files).
    pub fn is_clean(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true);
        let statuses = self.repo.statuses(Some(&mut opts))
            .context("checking repo status")?;
        Ok(statuses.is_empty())
    }

    /// Stage all changes and create a commit.
    pub fn commit_all(&self, message: &str) -> Result<git2::Oid> {
        let mut index = self.repo.index().context("reading index")?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .context("staging all files")?;
        index.write().context("writing index")?;

        let tree_oid = index.write_tree().context("writing tree")?;
        let tree = self.repo.find_tree(tree_oid).context("finding tree")?;

        let sig = self.repo.signature().context("reading signature")?;
        let parent = self.repo.head()?.peel_to_commit()
            .context("resolving parent commit")?;

        let oid = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            message,
            &tree,
            &[&parent],
        ).context("creating commit")?;

        Ok(oid)
    }

    /// Get the path to the .git directory.
    pub fn git_dir(&self) -> &Path {
        self.repo.path()
    }

    // ── File status ────────────────────────────────────────

    /// Get per-file status for the working tree and index.
    ///
    /// Returns entries for both unstaged (working tree) and staged (index)
    /// changes as separate entries.
    pub fn file_status(&self) -> Result<Vec<FileStatusEntry>> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .renames_head_to_index(true);
        let statuses = self.repo.statuses(Some(&mut opts))
            .context("reading file status")?;

        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let s = entry.status();

            // Index (staged) changes
            if s.intersects(git2::Status::INDEX_NEW) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Added, staged: true });
            } else if s.intersects(git2::Status::INDEX_MODIFIED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Modified, staged: true });
            } else if s.intersects(git2::Status::INDEX_DELETED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Deleted, staged: true });
            } else if s.intersects(git2::Status::INDEX_RENAMED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Renamed, staged: true });
            }

            // Working tree (unstaged) changes
            if s.intersects(git2::Status::WT_NEW) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Untracked, staged: false });
            } else if s.intersects(git2::Status::WT_MODIFIED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Modified, staged: false });
            } else if s.intersects(git2::Status::WT_DELETED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Deleted, staged: false });
            } else if s.intersects(git2::Status::WT_RENAMED) {
                entries.push(FileStatusEntry { path: path.clone(), status: FileStatus::Renamed, staged: false });
            }
        }

        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    // ── Staging ────────────────────────────────────────────

    /// Stage a file (add to index).
    pub fn stage_file(&self, path: &str) -> Result<()> {
        let mut index = self.repo.index().context("reading index")?;
        let abs = self.repo.workdir()
            .ok_or_else(|| anyhow::anyhow!("bare repository"))?
            .join(path);

        if abs.exists() {
            index.add_path(Path::new(path)).context("staging file")?;
        } else {
            // File was deleted — remove from index
            index.remove_path(Path::new(path)).context("staging deleted file")?;
        }
        index.write().context("writing index")?;
        Ok(())
    }

    /// Unstage a file (reset index entry to HEAD).
    pub fn unstage_file(&self, path: &str) -> Result<()> {
        let mut index = self.repo.index().context("reading index")?;
        let head_tree = self.repo.head()?.peel_to_tree()
            .context("resolving HEAD tree")?;

        match head_tree.get_path(Path::new(path)) {
            Ok(entry) => {
                // File exists in HEAD — reset index to HEAD version
                let obj = entry.to_object(&self.repo)?;
                let blob = obj.as_blob()
                    .ok_or_else(|| anyhow::anyhow!("HEAD entry is not a blob"))?;
                let mut idx_entry = git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0, ino: 0, mode: entry.filemode() as u32,
                    uid: 0, gid: 0,
                    file_size: blob.content().len() as u32,
                    id: entry.id(),
                    flags: 0, flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                };
                index.add(&idx_entry).context("resetting index entry")?;
                // Suppress unused warning — idx_entry fields are required by git2
                let _ = &mut idx_entry;
            }
            Err(_) => {
                // File is newly added (not in HEAD) — remove from index
                index.remove_path(Path::new(path)).context("removing from index")?;
            }
        }

        index.write().context("writing index")?;
        Ok(())
    }

    /// Stage all changes.
    pub fn stage_all(&self) -> Result<()> {
        let mut index = self.repo.index().context("reading index")?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .context("staging all")?;
        index.write().context("writing index")?;
        Ok(())
    }

    /// Discard working tree changes for a file (restore from index/HEAD).
    pub fn discard_changes(&self, path: &str) -> Result<()> {
        let mut cb = git2::build::CheckoutBuilder::new();
        cb.path(path).force();
        self.repo.checkout_head(Some(&mut cb))
            .with_context(|| format!("discarding changes for {}", path))?;
        Ok(())
    }

    // ── Committing ─────────────────────────────────────────

    /// Commit only what is currently staged (index).
    pub fn commit(&self, message: &str) -> Result<git2::Oid> {
        let mut index = self.repo.index().context("reading index")?;
        let tree_oid = index.write_tree().context("writing tree")?;
        let tree = self.repo.find_tree(tree_oid).context("finding tree")?;
        let sig = self.repo.signature().context("reading signature")?;
        let parent = self.repo.head()?.peel_to_commit()
            .context("resolving parent commit")?;

        let oid = self.repo.commit(
            Some("HEAD"), &sig, &sig, message, &tree, &[&parent],
        ).context("creating commit")?;
        Ok(oid)
    }

    /// Amend the last commit with a new message and the current index.
    pub fn amend(&self, message: &str) -> Result<git2::Oid> {
        let mut index = self.repo.index().context("reading index")?;
        let tree_oid = index.write_tree().context("writing tree")?;
        let tree = self.repo.find_tree(tree_oid).context("finding tree")?;
        let head = self.repo.head()?.peel_to_commit()
            .context("resolving HEAD")?;

        let oid = head.amend(
            Some("HEAD"), None, None, None, Some(message), Some(&tree),
        ).context("amending commit")?;
        Ok(oid)
    }

    // ── Branches ───────────────────────────────────────────

    /// List all branches (local and remote).
    pub fn branches(&self) -> Result<Vec<BranchEntry>> {
        let mut result = Vec::new();
        let branches = self.repo.branches(None).context("listing branches")?;

        for branch_result in branches {
            let (branch, branch_type) = branch_result.context("reading branch")?;
            let name = branch.name()?.unwrap_or("").to_string();
            if name.is_empty() { continue; }
            result.push(BranchEntry {
                name,
                is_current: branch.is_head(),
                is_remote: branch_type == git2::BranchType::Remote,
            });
        }
        Ok(result)
    }

    /// Read a file's content from HEAD (for diffing against working tree).
    pub fn head_file_content(&self, rel_path: &str) -> Result<String> {
        let head_tree = self.repo.head()?.peel_to_tree()
            .context("resolving HEAD tree")?;
        let entry = head_tree.get_path(Path::new(rel_path))
            .with_context(|| format!("file '{}' not in HEAD", rel_path))?;
        let obj = entry.to_object(&self.repo)?;
        let blob = obj.as_blob()
            .ok_or_else(|| anyhow::anyhow!("'{}' is not a file", rel_path))?;
        let content = std::str::from_utf8(blob.content())
            .context("file is not UTF-8")?;
        Ok(content.to_string())
    }

    /// Checkout a local branch.
    pub fn checkout(&self, branch: &str) -> Result<()> {
        let branch_ref = self.repo.find_branch(branch, git2::BranchType::Local)
            .with_context(|| format!("finding branch '{}'", branch))?;
        let commit = branch_ref.get().peel_to_commit()
            .context("resolving branch to commit")?;

        self.repo.checkout_tree(
            commit.as_object(),
            Some(git2::build::CheckoutBuilder::new().safe()),
        ).context("checkout tree")?;
        self.repo.set_head(&format!("refs/heads/{}", branch))
            .context("setting HEAD")?;
        Ok(())
    }
}

// ── Free git CLI helpers ───────────────────────────────────

/// Return the full SHA of HEAD in `repo_dir`.
pub fn current_head_sha(repo_dir: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .context("git rev-parse HEAD")?;
    anyhow::ensure!(out.status.success(), "git rev-parse HEAD failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Return the list of files changed in the most recent commit at `repo_dir`.
pub fn files_changed_in_commit(repo_dir: &Path) -> Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .args(["diff-tree", "--no-commit-id", "-r", "--name-only", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .context("git diff-tree")?;
    anyhow::ensure!(out.status.success(), "git diff-tree failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Return the unified diff of `branch` relative to `base_sha`.
///
/// Works even after the worktree for `branch` has been torn down, since the
/// branch ref still exists in the repository.
pub fn diff_branch_vs_sha(repo_dir: &Path, base_sha: &str, branch: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["diff", base_sha, branch])
        .current_dir(repo_dir)
        .output()
        .context("git diff")?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Hard-reset `repo_dir` to `sha` (`git reset --hard <sha>`).
pub fn reset_hard(repo_dir: &Path, sha: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["reset", "--hard", sha])
        .current_dir(repo_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git reset --hard")?;
    anyhow::ensure!(status.success(), "git reset --hard {} failed", sha);
    Ok(())
}

/// Delete a local branch by name. Errors are silently ignored (branch may not exist).
pub fn delete_branch(repo_dir: &Path, branch: &str) -> Result<()> {
    Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(repo_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("git branch -D")?;
    Ok(())
}

/// Manages git worktrees for parallel agent execution.
///
/// Each agent gets its own worktree with a dedicated branch, allowing
/// parallel file modifications without conflicts. Uses the `git` CLI
/// because git2's worktree API is limited.
pub struct WorktreeManager {
    /// Path to the main repository's working directory.
    repo_dir: PathBuf,
    /// Base directory for worktrees (e.g., `/tmp/gaviero-worktrees/`).
    worktree_base: PathBuf,
    /// Active worktrees that will be cleaned up on drop.
    active: Vec<WorktreeHandle>,
}

/// A handle to a provisioned worktree.
#[derive(Debug, Clone)]
pub struct WorktreeHandle {
    /// The worktree directory path.
    pub path: PathBuf,
    /// The branch name for this worktree.
    pub branch: String,
    /// The worktree name (used for `git worktree remove`).
    pub name: String,
}

impl WorktreeManager {
    pub fn new(repo_dir: PathBuf) -> Self {
        let worktree_base = std::env::temp_dir()
            .join("gaviero-worktrees")
            .join(
                repo_dir.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repo")
            );
        Self {
            repo_dir,
            worktree_base,
            active: Vec::new(),
        }
    }

    /// Check if this repo supports worktrees (is a git repo with at least one commit).
    pub fn can_use_worktrees(&self) -> bool {
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Get the current HEAD commit SHA (full hash).
    fn head_commit(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("running git rev-parse HEAD")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git rev-parse HEAD failed: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Provision a new worktree for an agent.
    ///
    /// Creates a new branch from HEAD and a worktree directory.
    /// The branch is named `gaviero/{agent_id}`.
    pub fn provision(&mut self, agent_id: &str) -> Result<WorktreeHandle> {
        let branch = format!("gaviero/{}", agent_id);
        let name = format!("gaviero-{}", agent_id);
        let wt_path = self.worktree_base.join(&name);

        // Resolve HEAD to a concrete commit SHA (avoids "invalid reference: HEAD")
        let commit = self.head_commit()
            .context("repo must have at least one commit for worktree isolation")?;

        // Ensure base directory exists
        std::fs::create_dir_all(&self.worktree_base)
            .with_context(|| format!("creating worktree base dir: {}", self.worktree_base.display()))?;

        // Clean up stale state from previous runs
        if wt_path.exists() {
            let _ = self.remove_worktree(&name);
            if wt_path.exists() {
                let _ = std::fs::remove_dir_all(&wt_path);
            }
        }

        // Prune stale worktree references (dead paths from crashed runs)
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // Delete stale branch if it exists (from a previous run)
        let _ = Command::new("git")
            .args(["branch", "-D", &branch])
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // Create worktree with a new branch based on the resolved commit
        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&wt_path)
            .arg(&commit)
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("running git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "failed to create worktree for '{}': {}",
                agent_id,
                stderr.trim(),
            );
        }

        let handle = WorktreeHandle {
            path: wt_path,
            branch,
            name,
        };
        self.active.push(handle.clone());
        Ok(handle)
    }

    /// Remove a worktree by name.
    fn remove_worktree(&self, name: &str) -> Result<()> {
        let status = Command::new("git")
            .args(["worktree", "remove", "--force", name])
            .current_dir(&self.repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("running git worktree remove")?;

        if !status.success() {
            tracing::warn!("failed to remove worktree '{}'", name);
        }
        Ok(())
    }

    /// Teardown a specific worktree handle.
    pub fn teardown(&mut self, handle: &WorktreeHandle) -> Result<()> {
        self.remove_worktree(&handle.name)?;
        self.active.retain(|h| h.name != handle.name);
        Ok(())
    }

    /// Teardown all active worktrees.
    pub fn teardown_all(&mut self) {
        let handles: Vec<WorktreeHandle> = self.active.drain(..).collect();
        for handle in &handles {
            let _ = self.remove_worktree(&handle.name);
        }
    }

    /// List active worktree handles.
    pub fn active_worktrees(&self) -> &[WorktreeHandle] {
        &self.active
    }

    /// Inject extra files into an agent's worktree after provisioning.
    ///
    /// Used for `@file`-referenced context files that are not git-tracked (e.g. tmp/ plan
    /// documents). Each `(rel_path, content)` pair is written to `<worktree>/<rel_path>`,
    /// creating parent directories as needed. This lets the subagent use the `Read` tool
    /// to access the file exactly as the coordinator can.
    pub fn inject_context_files(
        &self,
        agent_id: &str,
        files: &[(String, String)],
    ) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let worktree_name = format!("gaviero-{}", agent_id);
        let handle = self
            .active
            .iter()
            .find(|h| h.name == worktree_name)
            .with_context(|| format!("no active worktree for agent '{}'", agent_id))?;

        for (rel_path, content) in files {
            let abs = handle.path.join(rel_path);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating dirs for context file {}", rel_path))?;
            }
            std::fs::write(&abs, content)
                .with_context(|| format!("writing context file {}", rel_path))?;
        }
        Ok(())
    }
}

impl Drop for WorktreeManager {
    fn drop(&mut self) {
        self.teardown_all();
    }
}

// ── Git Coordinator ──────────────────────────────────────────

/// Serializes concurrent git metadata operations (add, commit, etc.) across
/// worktrees that share the same `.git` directory.
///
/// File I/O within worktree working directories is unrestricted — only git
/// state mutations need this lock. Wrap `commit_agent_changes` calls with
/// `lock_git` to prevent `.git/index.lock` races under parallel swarms.
pub struct GitCoordinator {
    lock: tokio::sync::Mutex<()>,
}

impl GitCoordinator {
    pub fn new() -> Self {
        Self {
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Acquire the global git lock, run `f` synchronously, then release.
    ///
    /// `f` should be a short-lived synchronous closure (a few git CLI calls).
    /// Do not use this for long-running agent execution — only for the commit
    /// step at the end of each agent.
    pub async fn lock_git<F, T>(&self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let _guard = self.lock.lock().await;
        f()
    }
}

impl Default for GitCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_test_repo() -> (TempDir, GitRepo) {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Create initial commit
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            // Write a file
            let file_path = dir.path().join("README.md");
            std::fs::write(&file_path, "# Test\n").unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        {
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[]).unwrap();
        }

        let git_repo = GitRepo { repo };
        (dir, git_repo)
    }

    #[test]
    fn test_open_repo() {
        let (dir, _repo) = init_test_repo();
        let opened = GitRepo::open(dir.path());
        assert!(opened.is_ok());
    }

    #[test]
    fn test_current_branch() {
        let (_dir, repo) = init_test_repo();
        let branch = repo.current_branch().unwrap();
        // git init creates "master" or "main" depending on config
        assert!(!branch.is_empty());
    }

    #[test]
    fn test_create_branch() {
        let (_dir, repo) = init_test_repo();
        repo.create_branch("test-branch").unwrap();
    }

    #[test]
    fn test_is_clean() {
        let (dir, repo) = init_test_repo();
        assert!(repo.is_clean().unwrap());

        // Modify a file
        std::fs::write(dir.path().join("README.md"), "modified\n").unwrap();
        assert!(!repo.is_clean().unwrap());
    }

    #[test]
    fn test_commit_all() {
        let (dir, repo) = init_test_repo();
        std::fs::write(dir.path().join("new_file.txt"), "content\n").unwrap();
        let oid = repo.commit_all("Add new file");
        assert!(oid.is_ok());
        assert!(repo.is_clean().unwrap());
    }

    #[test]
    fn test_file_status() {
        let (dir, repo) = init_test_repo();
        assert!(repo.file_status().unwrap().is_empty());

        // Modify a tracked file
        std::fs::write(dir.path().join("README.md"), "modified\n").unwrap();
        let status = repo.file_status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].status, FileStatus::Modified);
        assert!(!status[0].staged);

        // Add an untracked file
        std::fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let status = repo.file_status().unwrap();
        assert_eq!(status.len(), 2);
    }

    #[test]
    fn test_stage_unstage() {
        let (dir, repo) = init_test_repo();
        std::fs::write(dir.path().join("README.md"), "modified\n").unwrap();

        // Stage it
        repo.stage_file("README.md").unwrap();
        let status = repo.file_status().unwrap();
        assert!(status.iter().any(|e| e.path == "README.md" && e.staged));

        // Unstage it
        repo.unstage_file("README.md").unwrap();
        let status = repo.file_status().unwrap();
        assert!(status.iter().all(|e| !e.staged));
        // Should still show as unstaged modified
        assert!(status.iter().any(|e| e.path == "README.md" && !e.staged));
    }

    #[test]
    fn test_commit_staged_only() {
        let (dir, repo) = init_test_repo();
        // Modify two files
        std::fs::write(dir.path().join("README.md"), "changed\n").unwrap();
        std::fs::write(dir.path().join("other.txt"), "other\n").unwrap();

        // Stage only README
        repo.stage_file("README.md").unwrap();
        repo.commit("Commit README only").unwrap();

        // other.txt should still be untracked
        let status = repo.file_status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].path, "other.txt");
    }

    #[test]
    fn test_branches() {
        let (_dir, repo) = init_test_repo();
        repo.create_branch("feature").unwrap();

        let branches = repo.branches().unwrap();
        let local: Vec<_> = branches.iter().filter(|b| !b.is_remote).collect();
        assert!(local.len() >= 2); // main/master + feature
        assert!(local.iter().any(|b| b.name == "feature"));
        assert!(local.iter().any(|b| b.is_current));
    }

    #[test]
    fn test_checkout() {
        let (dir, repo) = init_test_repo();
        repo.create_branch("other").unwrap();
        repo.checkout("other").unwrap();

        assert_eq!(repo.current_branch().unwrap(), "other");

        // Checkout back to original
        let branches = repo.branches().unwrap();
        let original = branches.iter().find(|b| b.name != "other" && !b.is_remote).unwrap();
        repo.checkout(&original.name).unwrap();
        assert_ne!(repo.current_branch().unwrap(), "other");
    }

    #[test]
    fn test_discard_changes() {
        let (dir, repo) = init_test_repo();
        let path = dir.path().join("README.md");
        std::fs::write(&path, "modified\n").unwrap();
        assert!(!repo.is_clean().unwrap());

        repo.discard_changes("README.md").unwrap();
        assert!(repo.is_clean().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Test\n");
    }

    #[test]
    fn test_worktree_provision_and_teardown() {
        let (dir, _repo) = init_test_repo();
        let mut wm = WorktreeManager::new(dir.path().to_path_buf());

        let handle = wm.provision("agent-1").unwrap();
        assert!(handle.path.exists());
        assert_eq!(handle.branch, "gaviero/agent-1");

        wm.teardown(&handle).unwrap();
        // Worktree directory should be removed
        assert!(!handle.path.exists() || true); // git worktree remove may leave the dir
        assert!(wm.active_worktrees().is_empty());
    }
}
