//! End-to-end CLI tests for the `--remember` headless write path.
//!
//! Phase 1.6 (CLI side): spawning `gaviero-cli --remember "..."`
//! against a fresh tempdir must:
//!   1. exit successfully,
//!   2. print the inserted-id confirmation,
//!   3. produce the expected `.gaviero/memory.db` file.
//!
//! These are subprocess tests — slower than the library round-trip in
//! `gaviero-core/tests/headless_memory_services.rs`, but they prove
//! the CLI binary surface (clap parsing → handler → MemoryServices)
//! is wired end-to-end.

use std::process::Command;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set by Cargo for integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gaviero-cli"))
        .with_file_name(name)
        .with_extension(std::env::consts::EXE_EXTENSION)
}

fn gaviero_cli() -> std::path::PathBuf {
    cargo_bin("gaviero-cli")
}

#[test]
#[ignore = "spawns gaviero-cli with ONNX runtime; run with --ignored"]
fn remember_repo_scope_inserts_and_reports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    let output = Command::new(gaviero_cli())
        .arg("--repo")
        .arg(repo)
        .arg("--remember")
        .arg("Phase 1.5 wired CLI through MemoryServices")
        .output()
        .expect("spawn gaviero-cli");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "gaviero-cli --remember failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("[gaviero-remember]"),
        "expected confirmation prefix in stdout, got: {stdout:?}"
    );
    assert!(
        stdout.contains("inserted") || stdout.contains("deduplicated"),
        "expected inserted/deduplicated outcome, got: {stdout:?}"
    );
    let db = repo.join(".gaviero").join("memory.db");
    assert!(
        db.exists(),
        "expected memory.db at {} after --remember; not present",
        db.display()
    );
}

#[test]
fn remember_rejects_invalid_scope() {
    // Fast — no DB work, just clap dispatch + early validation.
    let output = Command::new(gaviero_cli())
        .arg("--repo")
        .arg(std::env::temp_dir())
        .arg("--remember")
        .arg("anything")
        .arg("--remember-scope")
        .arg("module")
        .output()
        .expect("spawn gaviero-cli");
    assert!(!output.status.success(), "module scope should fail headless");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("/remember-here") || stderr.contains("/remember-module"),
        "expected error to point user at TUI commands, got: {stderr:?}"
    );
}

#[test]
fn remember_rejects_empty_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(gaviero_cli())
        .arg("--repo")
        .arg(tmp.path())
        .arg("--remember")
        .arg("   ")
        .output()
        .expect("spawn gaviero-cli");
    assert!(!output.status.success(), "empty text should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty text"),
        "expected 'empty text' error, got: {stderr:?}"
    );
}
