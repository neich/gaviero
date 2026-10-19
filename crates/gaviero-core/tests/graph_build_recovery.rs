use gaviero_core::repo_map::{
    graph_builder::build_graph_into,
    store::GraphStore,
};

#[test]
fn failed_edge_insertion_rolls_back_and_retries_on_the_same_store() {
    // Exercise both structured resolution and the bare-name fallback.
    for (ext, alpha, beta, before, after) in [
        ("rs", "pub fn alpha() {}", "pub fn beta() {}",
         "fn run() { alpha(); }", "fn run() { beta(); }"),
        ("js", "function alpha() {}", "function beta() {}",
         "function run() { alpha(); }", "function run() { beta(); }"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("src");
        std::fs::create_dir(&root).unwrap();
        let a = format!("a.{ext}");
        let b = format!("b.{ext}");
        let caller = format!("caller.{ext}");
        std::fs::write(root.join(&a), alpha).unwrap();
        std::fs::write(root.join(&b), beta).unwrap();
        std::fs::write(root.join(&caller), before).unwrap();
        let db = dir.path().join("graph.db");
        let store = GraphStore::open(&db).unwrap();
        build_graph_into(&store, &root, &[]).unwrap();
        let hashes = store.all_file_hashes().unwrap();
        let edges = store.file_edges().unwrap();
        assert!(edges.iter().any(|(from, to, kind)|
            from == &caller && to == &a && kind.as_str() == "Calls"), "{ext}: {edges:?}");

        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_calls BEFORE INSERT ON edges
            WHEN NEW.kind = 'Calls' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
        std::fs::write(root.join(&caller), after).unwrap();
        assert!(build_graph_into(&store, &root, &[]).is_err());
        assert_eq!(store.all_file_hashes().unwrap(), hashes);
        assert_eq!(store.file_edges().unwrap(), edges);
        conn.execute_batch("DROP TRIGGER reject_calls;").unwrap();

        let result = build_graph_into(&store, &root, &[]).unwrap();
        assert_eq!(result.files_changed, 1);
        assert!(store.file_edges().unwrap().iter().any(|(from, to, kind)|
            from == &caller && to == &b && kind.as_str() == "Calls"));
        assert_eq!(build_graph_into(&store, &root, &[]).unwrap().files_changed, 0);
    }
}

#[test]
fn failed_definition_move_preserves_deleted_files_and_retries_unchanged_callers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("src");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.rs"), "pub fn alpha() {}").unwrap();
    std::fs::write(root.join("caller.rs"), "fn run() { alpha(); }").unwrap();
    let db = dir.path().join("graph.db");
    let store = GraphStore::open(&db).unwrap();
    build_graph_into(&store, &root, &[]).unwrap();
    let hashes = store.all_file_hashes().unwrap();
    let edges = store.file_edges().unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_calls BEFORE INSERT ON edges
        WHEN NEW.kind = 'Calls' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    std::fs::remove_file(root.join("a.rs")).unwrap();
    std::fs::write(root.join("b.rs"), "pub fn alpha() {}").unwrap();
    assert!(build_graph_into(&store, &root, &[]).is_err());
    assert_eq!(store.all_file_hashes().unwrap(), hashes);
    assert_eq!(store.file_edges().unwrap(), edges);
    assert!(!store.nodes_for_file("a.rs").unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER reject_calls;").unwrap();
    drop(store);

    let store = GraphStore::open(&db).unwrap();
    let result = build_graph_into(&store, &root, &[]).unwrap();
    assert_eq!(result.files_removed, 1);
    assert_eq!(result.files_reresolved, 1);
    assert!(store.nodes_for_file("a.rs").unwrap().is_empty());
    assert!(store.file_edges().unwrap().iter().any(|(from, to, kind)|
        from == "caller.rs" && to == "b.rs" && kind.as_str() == "Calls"));
}
