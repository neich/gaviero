//! T7 — `ValidationPipeline::fast_only` end-to-end against real Rust on disk.
//!
//! Pins the per-write tree-sitter syntax gate that fires after every
//! agent turn in `swarm::backend::runner`. Walks the full triangle
//! (gate trait → structural verifier → tree-sitter language registry)
//! against real files, which is the only way to catch breakage where
//! a refactor on one side leaves the others out of sync.

use std::path::PathBuf;

use gaviero_core::validation_gate::{ValidationPipeline, ValidationResult};

#[tokio::test]
async fn fast_only_passes_valid_rust_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();

    let src_dir = workdir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("ok.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
    std::fs::write(
        src_dir.join("module.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();

    let pipeline = ValidationPipeline::fast_only();
    let files = vec![
        PathBuf::from("src/ok.rs"),
        PathBuf::from("src/module.rs"),
    ];
    let result = pipeline.run(&files, workdir, /* fast_only */ true).await;

    assert!(
        result.is_none(),
        "valid Rust must produce no failure, got {result:?}"
    );
}

#[tokio::test]
async fn fast_only_fails_broken_rust_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();

    let src_dir = workdir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    // Unmatched paren in fn signature — tree-sitter must surface this.
    std::fs::write(
        src_dir.join("bad.rs"),
        "fn broken( {\n    println!(\"never closes\");\n}\n",
    )
    .unwrap();

    let pipeline = ValidationPipeline::fast_only();
    let result = pipeline
        .run(&[PathBuf::from("src/bad.rs")], workdir, true)
        .await;

    let (gate, outcome) = result.expect("broken Rust must produce a failure");
    assert_eq!(gate, "tree-sitter");
    match outcome {
        ValidationResult::Fail { message, suggestion } => {
            assert!(
                message.contains("bad.rs"),
                "failure message must reference the broken file, got: {message}"
            );
            assert!(
                message.contains("line"),
                "failure message must include a line number, got: {message}"
            );
            assert!(
                suggestion
                    .as_deref()
                    .is_some_and(|s| s.to_lowercase().contains("syntax")),
                "suggestion must mention syntax fix, got: {suggestion:?}"
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[tokio::test]
async fn fast_only_skips_unknown_extensions() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();

    std::fs::write(workdir.join("README.md"), "# title\n").unwrap();
    std::fs::write(workdir.join("data.bin"), [0u8, 1, 2, 3, 4]).unwrap();

    let pipeline = ValidationPipeline::fast_only();
    let result = pipeline
        .run(
            &[PathBuf::from("README.md"), PathBuf::from("data.bin")],
            workdir,
            true,
        )
        .await;

    assert!(
        result.is_none(),
        "unknown extensions must be skipped without a failure, got {result:?}"
    );
}

#[tokio::test]
async fn fast_only_short_circuits_on_first_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    let src_dir = workdir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    std::fs::write(src_dir.join("a.rs"), "fn ok() {}\n").unwrap();
    std::fs::write(src_dir.join("b.rs"), "fn broken( {\n").unwrap();

    let pipeline = ValidationPipeline::fast_only();
    let mut gates_seen: Vec<(String, bool)> = Vec::new();
    let result = pipeline
        .run_reporting(
            &[PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
            workdir,
            true,
            |gate, passed| gates_seen.push((gate.to_string(), passed)),
        )
        .await;

    let (gate, _) = result.expect("must report failure");
    assert_eq!(gate, "tree-sitter");
    // The fast_only pipeline has exactly one gate, so we expect a single
    // (name, passed=false) report.
    assert_eq!(gates_seen, vec![("tree-sitter".to_string(), false)]);
}
