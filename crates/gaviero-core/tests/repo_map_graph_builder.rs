//! T8 — `repo_map::graph_builder::build_graph` integration test.
//!
//! Walks the full builder pipeline against a real tempdir:
//! tree-sitter symbol extraction → reference extraction → SQLite
//! persistence → blast-radius query. Backs `gaviero-cli --graph` and
//! the MCP `blast_radius` tool. Per-component unit tests don't exercise
//! the join, so any future refactor that breaks an interface between
//! `builder::extract_symbols`, `edges::resolve_and_insert_edges`, and
//! the SQLite store would only surface end-to-end here.

use std::path::Path;
use std::process::Command;

use gaviero_core::repo_map::graph_builder::build_graph;

/// Initialize a tiny Rust crate at `root` with three files and inter-file
/// references so the builder has nodes + edges to materialize:
///
/// - `Cargo.toml` — bare manifest (gives the workspace a Rust signal).
/// - `src/lib.rs` — declares `mod helper;` and exports a `run()` that
///   calls `gv_helper_greet()` (defined in `helper.rs`).
/// - `src/helper.rs` — exports the called function.
/// - `src/orphan.rs` — declared but never called; the builder still emits a node.
fn seed_rust_workspace(root: &Path) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    // `lib.rs` calls `gv_helper_greet` (defined in `helper.rs`) by bare
    // name so the cross-file edge resolver can match `target_name`
    // unambiguously. Globally unique names avoid collisions with any
    // other symbol the workspace scan might pick up.
    std::fs::write(
        src.join("lib.rs"),
        "pub mod helper;\npub mod orphan;\nuse helper::gv_helper_greet;\n\
         pub fn gv_lib_run() { gv_helper_greet(); }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("helper.rs"),
        "pub fn gv_helper_greet() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("orphan.rs"),
        "pub fn gv_orphan_unused() { /* nobody calls me */ }\n",
    )
    .unwrap();
}

#[test]
fn build_graph_indexes_real_workspace_and_persists_to_sqlite() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    seed_rust_workspace(workspace);

    let (store, result) = build_graph(workspace, &[]).expect("build_graph");

    assert!(
        result.files_scanned >= 3,
        "expected ≥3 source files scanned, got {}",
        result.files_scanned
    );
    assert!(
        result.files_changed >= 3,
        "first build must re-index every source file, got {}",
        result.files_changed
    );
    assert_eq!(result.files_unchanged, 0);
    assert!(
        result.total_nodes > 0,
        "graph must contain at least one node, got {}",
        result.total_nodes
    );
    assert!(
        result.total_edges > 0,
        "graph must contain at least one edge, got {}",
        result.total_edges
    );

    // Stats accessor mirrors result counts (sanity for the SQLite read path).
    let (nodes, edges) = store.stats().expect("stats");
    assert_eq!(nodes, result.total_nodes);
    assert_eq!(edges, result.total_edges);

    // SQLite db file landed at the documented path.
    let db_path = workspace.join(".gaviero").join("code_graph.db");
    assert!(
        db_path.exists(),
        "graph DB must be persisted at {}",
        db_path.display()
    );
}

#[test]
fn build_graph_is_incremental_on_unchanged_workspace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    seed_rust_workspace(workspace);

    // First build: every file is "changed" (none in the store yet).
    let (_store, first) = build_graph(workspace, &[]).expect("first build");
    assert!(first.files_changed >= 3);

    // Second build over an untouched workspace: no file should re-index.
    let (_store, second) = build_graph(workspace, &[]).expect("second build");
    assert_eq!(
        second.files_changed, 0,
        "incremental rebuild on unchanged source must touch nothing, got {}",
        second.files_changed
    );
    assert!(
        second.files_unchanged >= 3,
        "incremental rebuild must mark all files unchanged, got {}",
        second.files_unchanged
    );
}

#[test]
fn impact_radius_finds_caller_of_changed_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    seed_rust_workspace(workspace);

    let (store, _) = build_graph(workspace, &[]).expect("build_graph");

    // `helper.rs` defines `greet()`; `lib.rs` calls it. Changing helper
    // should mark lib as affected.
    let impact = store
        .impact_radius(&["src/helper.rs"], 3)
        .expect("impact_radius");

    assert_eq!(impact.changed_files, vec!["src/helper.rs"]);
    assert!(
        impact
            .affected_files
            .iter()
            .any(|f| f == "src/lib.rs"),
        "lib.rs must be marked as affected by changes to helper.rs, got {:?}",
        impact.affected_files,
    );
}

#[test]
fn build_graph_handles_added_then_removed_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    seed_rust_workspace(workspace);

    let (_store, first) = build_graph(workspace, &[]).expect("first build");
    let initial_files = first.files_scanned;

    // Add a new file and rebuild.
    let extra = workspace.join("src").join("extra.rs");
    std::fs::write(&extra, "pub fn extra() {}\n").unwrap();
    let (_store, with_extra) = build_graph(workspace, &[]).expect("rebuild after add");
    assert_eq!(with_extra.files_scanned, initial_files + 1);
    assert_eq!(
        with_extra.files_changed, 1,
        "only the new file should re-index, got {}",
        with_extra.files_changed
    );

    // Remove the file and rebuild — the store must drop it.
    std::fs::remove_file(&extra).unwrap();
    let (_store, after_remove) = build_graph(workspace, &[]).expect("rebuild after remove");
    assert_eq!(after_remove.files_scanned, initial_files);
    assert!(
        after_remove.files_removed >= 1,
        "deleted file must be reported as removed, got {}",
        after_remove.files_removed
    );

    // Avoid an unused-import warning if `Command` ever becomes needed.
    let _ = Command::new("true");
}
