//! Headless [`MemoryServices`] round-trip integration test.
//!
//! Phase 1.6 acceptance: with the writer task spawned via
//! `MemoryServices::open` against a single-folder workspace, a
//! `/remember`-style write through the writer must be visible to a
//! subsequent retrieval call. This is the headless equivalent of the
//! TUI's chat-write-then-recall flow — and the regression target for
//! the "CLI silently has no memory" gap diagnosed in the tier review.

use std::sync::Arc;

use gaviero_core::memory::scope::{MemoryScope, WriteScope};
use gaviero_core::memory::{MemoryServices, ServicesOpts, WriteResult, hash_path};
use gaviero_core::workspace::Workspace;

/// Open `MemoryServices` against a tempdir workspace, enqueue a
/// `/remember` write through the writer, then search the workspace
/// store and confirm the row landed at Repo scope and surfaces in
/// retrieval.
///
/// Skipped on machines without ONNX runtime / model cache; on
/// CI-style boxes the model download dominates wall time, so we
/// don't gate the rest of the suite on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires ONNX runtime + embedder model; run with `cargo test --test headless_memory_services -- --ignored`"]
async fn user_remember_round_trip_through_writer_task() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    // The single-folder constructor doubles as workspace root.
    let workspace = Workspace::single_folder(repo.clone());

    let services: Arc<MemoryServices> = tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || MemoryServices::open(&repo, &workspace, ServicesOpts::default())
    })
    .await
    .expect("blocking task")
    .expect("MemoryServices::open");

    // Phase 1.5 invariant: every headless write goes through the
    // writer handle. No direct MemoryStore access from the test.
    let scope = WriteScope::Repo {
        repo_id: hash_path(&repo),
    };
    let result = services
        .writer
        .user_remember_scoped(scope.clone(), "test fact: the foundation is wired")
        .await
        .expect("user_remember_scoped");
    let memory_id = match result {
        WriteResult::Inserted(id) => id,
        WriteResult::Deduplicated(id) => id,
        other => panic!("unexpected write result: {other:?}"),
    };
    assert!(memory_id > 0, "writer task returned id={memory_id}");

    // Read-side: the stores registry routes Repo-scope reads to the
    // folder store keyed by repo_id. We use the high-level
    // `search_scoped` API so the test mirrors what the chat path does.
    let target_store = services
        .stores
        .get(&scope.target_store())
        .await
        .expect("get folder store");
    let cfg = gaviero_core::memory::SearchConfig {
        query: "foundation".to_string(),
        max_results: 5,
        per_level_limit: 5,
        similarity_threshold: 0.0,
        confidence_threshold: 0.0,
        use_fts: true,
        // Single-folder workspace: folder == workspace_root, so the
        // Repo-scope cascade resolves to the same physical store.
        scope: MemoryScope::from_context(&repo, Some(&repo), None, None),
    };
    let hits = target_store
        .search_scoped(&cfg)
        .await
        .expect("search_scoped");
    assert!(
        hits.iter().any(|m| m.id == memory_id),
        "wrote id={memory_id} but search_scoped did not surface it; got {} hits",
        hits.len()
    );
}

/// Sanity-check the in-memory test bootstrap — guards against drift
/// in `MemoryServices::for_tests_in_memory`. Fast, no ONNX needed.
#[tokio::test]
async fn for_tests_in_memory_writer_is_alive() {
    let services = MemoryServices::for_tests_in_memory().expect("test bootstrap");
    assert!(services.writer.is_alive());
    assert_eq!(services.writer.queue_depth(), 0);
}
