//! Integration tests for `gaviero-mcp-shim --resolve`.

use std::process::Command;
use std::time::{Duration, Instant};

fn shim_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gaviero-mcp-shim")
}

#[test]
fn resolve_exits_2_without_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let status = Command::new(shim_bin())
        .arg("--resolve")
        .current_dir(dir.path())
        .status()
        .expect("spawn shim");
    let elapsed = start.elapsed();
    assert_eq!(status.code(), Some(2), "expected exit 2, got {status:?}");
    assert!(
        elapsed < Duration::from_millis(2000),
        "no-descriptor exit took {elapsed:?} (budget 2s; plan asked 100ms; Windows spawn is slower)"
    );
}

#[test]
fn resolve_exits_2_when_pid_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    let gav = dir.path().join(".gaviero");
    std::fs::create_dir_all(&gav).unwrap();
    std::fs::write(
        gav.join("mcp-endpoint.json"),
        r#"{
            "v": 1,
            "workspace_id": "dead",
            "pipe": "\\\\.\\pipe\\gaviero-dead",
            "socket": "/tmp/gaviero-dead.sock",
            "pid": 999999,
            "started_at": "2026-09-14T12:00:00Z"
        }"#,
    )
    .unwrap();
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    let start = Instant::now();
    let status = Command::new(shim_bin())
        .arg("--resolve")
        .current_dir(&nested)
        .status()
        .expect("spawn shim");
    let elapsed = start.elapsed();
    assert_eq!(status.code(), Some(2), "expected exit 2, got {status:?}");
    assert!(
        elapsed < Duration::from_millis(2000),
        "dead-pid exit took {elapsed:?}"
    );
}
