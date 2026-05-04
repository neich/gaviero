//! T3 — `WorktreeManager::provision` + `Drop` lifecycle integration test.
//!
//! Verifies the swarm's per-agent worktree provisioning end-to-end:
//! - A worktree directory and matching `gaviero/*` branch exist after `provision`.
//! - The branch is reachable from the parent repo's branch list.
//! - Dropping the manager removes the worktree directory (Drop → teardown_all).
//!
//! Uses real git CLI / git2 — the function under test shells out, so any
//! mock would substitute the exact behaviour we want to verify.

use std::path::Path;
use std::process::Command;

use gaviero_core::git::{WorktreeManager, list_local_branches_with_prefix};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn make_seeded_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init", "-q", "--initial-branch=initial"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    std::fs::write(repo.join("README.md"), "seed\n").unwrap();
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-q", "-m", "seed"]);
    tmp
}

#[test]
fn provision_creates_worktree_dir_and_branch() {
    let tmp = make_seeded_repo();
    let repo = tmp.path().to_path_buf();

    let mut wm = WorktreeManager::new(repo.clone());
    let handle = wm.provision("agent-x").expect("provision");

    assert!(
        handle.path.exists(),
        "worktree dir must exist after provision: {}",
        handle.path.display()
    );
    assert!(
        handle.path.join(".git").exists(),
        "worktree must be a real git checkout (`.git` present)"
    );
    assert_eq!(handle.branch, "gaviero/agent-x");

    let branches = list_local_branches_with_prefix(&repo, "gaviero/").unwrap();
    assert!(
        branches.iter().any(|b| b == "gaviero/agent-x"),
        "branch `gaviero/agent-x` must be visible in parent repo, got {branches:?}"
    );

    // active_worktrees() should report one provisioned worktree.
    assert_eq!(wm.active_worktrees().len(), 1);
    assert_eq!(wm.active_worktrees()[0].branch, "gaviero/agent-x");

    // Avoid Drop-cleanup race: explicit teardown so the assertion below
    // about disk state is deterministic, then we let Drop be a no-op.
    drop(wm);
    assert!(
        !handle.path.exists(),
        "worktree dir must be removed after WorktreeManager drop: {}",
        handle.path.display()
    );
}

#[test]
fn drop_cleans_up_all_active_worktrees() {
    let tmp = make_seeded_repo();
    let repo = tmp.path().to_path_buf();

    let mut wm = WorktreeManager::new(repo.clone());
    let h1 = wm.provision("agent-a").expect("provision a");
    let h2 = wm.provision("agent-b").expect("provision b");
    assert!(h1.path.exists());
    assert!(h2.path.exists());
    assert_eq!(wm.active_worktrees().len(), 2);

    drop(wm);

    assert!(
        !h1.path.exists(),
        "first worktree dir must be removed by Drop"
    );
    assert!(
        !h2.path.exists(),
        "second worktree dir must be removed by Drop"
    );
}

#[test]
fn provision_fails_cleanly_on_repo_without_commits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "--initial-branch=initial"]);

    let mut wm = WorktreeManager::new(repo);
    let err = wm
        .provision("agent-orphan")
        .expect_err("provision must error without a HEAD commit");
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("commit"),
        "error must mention the missing commit, got: {msg}"
    );
}
