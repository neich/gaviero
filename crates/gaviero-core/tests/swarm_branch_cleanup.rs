//! T1 — Integration test for `swarm::pipeline::cleanup_gaviero_branches`.
//!
//! Exercises the new `--cleanup-branches` CLI surface (PR #119) end-to-end
//! against a real git repo: dry-run preview, force-delete, and protection
//! of the currently checked-out branch. Uses `git2` to assert post-state
//! so the test never depends on shelling out beyond what the function
//! under test already does.
//!
//! The test creates short-lived branches in a tempdir; nothing leaks past
//! the tempdir's `Drop`.

use std::path::Path;
use std::process::Command;

use gaviero_core::swarm::pipeline::{BranchCleanupReport, cleanup_gaviero_branches};

/// Run a `git` command in `repo`, panicking on failure with stderr in the
/// message. Tests-only helper — production code uses `git2`.
fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    if !out.status.success() {
        panic!(
            "git {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Initialise a repo with one commit, an `initial` branch (renamed from
/// the default to avoid `init.defaultBranch` differences across CI hosts),
/// and the supplied list of additional branches all pointing at HEAD.
fn make_repo_with_branches(extra: &[&str]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();

    // Some CI environments default to `master`, others `main`. Pin our
    // branch name explicitly so the test's "initial branch survives"
    // assertion can use a known string.
    git(repo, &["init", "-q", "--initial-branch=initial"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    std::fs::write(repo.join("README.md"), "seed\n").unwrap();
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-q", "-m", "seed"]);

    for branch in extra {
        git(repo, &["branch", branch]);
    }
    tmp
}

fn list_local_branches(repo: &Path) -> Vec<String> {
    let out = Command::new("git")
        .args(["branch", "--list", "--format=%(refname:short)"])
        .current_dir(repo)
        .output()
        .expect("git branch --list");
    assert!(out.status.success(), "git branch --list failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn dry_run_lists_gaviero_branches_without_deleting() {
    let tmp = make_repo_with_branches(&["gaviero/foo", "gaviero/bar", "feature/x"]);
    let repo = tmp.path();

    let report: BranchCleanupReport =
        cleanup_gaviero_branches(repo, /* dry_run */ true).expect("cleanup dry-run");

    let mut matched = report.matched.clone();
    matched.sort();
    assert_eq!(matched, vec!["gaviero/bar", "gaviero/foo"]);
    assert!(report.deleted.is_empty(), "dry-run must delete nothing");
    assert!(
        report.skipped_current.is_empty(),
        "no current-branch protection trips on `initial`"
    );

    // No branches were actually removed.
    let after = list_local_branches(repo);
    assert!(after.iter().any(|b| b == "gaviero/foo"));
    assert!(after.iter().any(|b| b == "gaviero/bar"));
    assert!(after.iter().any(|b| b == "feature/x"));
    assert!(after.iter().any(|b| b == "initial"));
}

#[test]
fn force_run_deletes_only_gaviero_prefixed_branches() {
    let tmp = make_repo_with_branches(&["gaviero/foo", "gaviero/bar", "feature/x"]);
    let repo = tmp.path();

    let report =
        cleanup_gaviero_branches(repo, /* dry_run */ false).expect("cleanup force");

    let mut matched = report.matched.clone();
    matched.sort();
    let mut deleted = report.deleted.clone();
    deleted.sort();
    assert_eq!(matched, vec!["gaviero/bar", "gaviero/foo"]);
    assert_eq!(deleted, vec!["gaviero/bar", "gaviero/foo"]);
    assert!(report.skipped_current.is_empty());

    let after = list_local_branches(repo);
    assert!(
        !after.iter().any(|b| b.starts_with("gaviero/")),
        "no `gaviero/*` branch should remain, got {after:?}"
    );
    // Untouched branches survive.
    assert!(after.iter().any(|b| b == "feature/x"), "feature branch must survive");
    assert!(after.iter().any(|b| b == "initial"), "initial branch must survive");
}

#[test]
fn force_run_skips_currently_checked_out_branch() {
    let tmp = make_repo_with_branches(&["gaviero/keep-me", "gaviero/delete-me"]);
    let repo = tmp.path();

    // Check out the branch we expect to be protected.
    git(repo, &["checkout", "-q", "gaviero/keep-me"]);

    let report =
        cleanup_gaviero_branches(repo, /* dry_run */ false).expect("cleanup force");

    assert_eq!(report.skipped_current, vec!["gaviero/keep-me"]);
    assert_eq!(report.deleted, vec!["gaviero/delete-me"]);

    let after = list_local_branches(repo);
    assert!(
        after.iter().any(|b| b == "gaviero/keep-me"),
        "current branch must remain after force-cleanup"
    );
    assert!(
        !after.iter().any(|b| b == "gaviero/delete-me"),
        "non-current gaviero/ branch must be deleted"
    );
}

#[test]
fn no_match_yields_empty_report() {
    let tmp = make_repo_with_branches(&["feature/x", "topic/y"]);
    let repo = tmp.path();

    let report = cleanup_gaviero_branches(repo, /* dry_run */ true).expect("cleanup");
    assert!(report.matched.is_empty());
    assert!(report.deleted.is_empty());
    assert!(report.skipped_current.is_empty());
}
