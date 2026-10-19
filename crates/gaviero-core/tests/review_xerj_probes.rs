use gaviero_core::repo_map::{edges::{extract_file, CallStyle}, graph_builder::build_graph_into, resolve::{FileScope, Lang, ResolveContext, Outcome}, store::GraphStore};

fn probe(files: &[(&str, &str)], caller: &str, method: &str) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='demo'\nversion='0.1.0'\nedition='2024'\n").unwrap();
    for (file, content) in files {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let store = GraphStore::open_memory().unwrap();
    build_graph_into(&store, dir.path(), &[]).unwrap();
    let ctx = ResolveContext::build(&store, dir.path()).unwrap();
    let source = files.iter().find(|(f, _)| *f == caller).unwrap().1;
    let extracted = extract_file("rs", source);
    let scope = FileScope::build(Lang::Rust, &extracted.refs);
    let r = extracted.refs.iter().find(|r| r.style == CallStyle::Method && r.target_name == method).unwrap();
    let outcome = ctx.resolve(caller, Lang::Rust, &scope, r);
    println!("{method}: receiver {:?}; outcome {outcome:?}", r.receiver_expr);
    outcome
}

fn assert_target(out: Outcome, expected: &str) {
    assert!(matches!(&out, Outcome::Files { targets, .. } if targets.iter().any(|(f, _)| f == expected)), "expected {expected}, got {out:?}");
}

#[test]
fn external_import_must_override_workspace_homonym() {
    let out = probe(&[
        ("src/lib.rs", "mod local; mod caller;"),
        ("src/local.rs", "pub struct Instant; impl Instant { pub fn elapsed(&self) {} }"),
        ("src/caller.rs", "use std::time::Instant; fn run(x: Instant) { x.elapsed(); }"),
    ], "src/caller.rs", "elapsed");
    assert!(!matches!(out, Outcome::Files { .. }), "external call got {out:?}");
}

#[test]
fn renamed_type_keeps_its_canonical_identity() {
    assert_target(probe(&[
        ("src/lib.rs", "mod a; mod b; mod caller;"),
        ("src/a.rs", "pub struct Widget; impl Widget { pub fn frobnicate(&self) {} }"),
        ("src/b.rs", "pub struct Other; impl Other { pub fn frobnicate(&self) {} }"),
        ("src/caller.rs", "use crate::a::Widget as W; fn run(x: W) { x.frobnicate(); }"),
    ], "src/caller.rs", "frobnicate"), "src/a.rs");
}

#[test]
fn inherent_return_wins_over_trait_return() {
    assert_target(probe(&[
        ("src/lib.rs", "mod holder; mod a; mod b; mod caller;"),
        ("src/a.rs", "pub struct A; impl A { pub fn frobnicate(&self) {} }"),
        ("src/b.rs", "pub struct B; impl B { pub fn frobnicate(&self) {} }"),
        ("src/holder.rs", "pub struct Holder; pub trait Tr { fn get(&self) -> crate::b::B; } impl Tr for Holder { fn get(&self) -> crate::b::B { crate::b::B } } impl Holder { pub fn get(&self) -> crate::a::A { crate::a::A } }"),
        ("src/caller.rs", "use crate::holder::Holder; fn run(x: Holder) { x.get().frobnicate(); }"),
    ], "src/caller.rs", "frobnicate"), "src/a.rs");
}

#[test]
fn for_binding_does_not_escape_loop() {
    assert_target(probe(&[
        ("src/lib.rs", "mod a; mod b; mod caller;"),
        ("src/a.rs", "pub struct A; impl A { pub fn frobnicate(&self) {} }"),
        ("src/b.rs", "pub struct B; impl B { pub fn frobnicate(&self) {} }"),
        ("src/caller.rs", "fn run(x: crate::a::A) { for x in 0..2 {} x.frobnicate(); }"),
    ], "src/caller.rs", "frobnicate"), "src/a.rs");
}

#[test]
fn distinct_types_with_same_name_do_not_share_methods() {
    assert_target(probe(&[
        ("src/lib.rs", "mod a; mod b; mod a_impl; mod caller;"),
        ("src/a.rs", "pub struct Widget;"),
        ("src/b.rs", "pub struct Widget; impl Widget { pub fn frobnicate(&self) {} }"),
        ("src/a_impl.rs", "impl crate::a::Widget { pub fn frobnicate(&self) {} }"),
        ("src/caller.rs", "fn run(x: crate::a::Widget) { x.frobnicate(); }"),
    ], "src/caller.rs", "frobnicate"), "src/a_impl.rs");
}

#[test]
fn self_in_result_return_survives_try() {
    assert_target(probe(&[
        ("src/lib.rs", "mod a; mod b; mod caller;"),
        ("src/a.rs", "pub struct A; impl A { pub fn open() -> Result<Self, ()> { Ok(Self) } pub fn frobnicate(&self) {} }"),
        ("src/b.rs", "pub struct B; impl B { pub fn frobnicate(&self) {} }"),
        ("src/caller.rs", "fn run() -> Result<(), ()> { let x = crate::a::A::open()?; x.frobnicate(); Ok(()) }"),
    ], "src/caller.rs", "frobnicate"), "src/a.rs");
}

#[test]
fn failed_incremental_edge_phase_is_retried() {
    use gaviero_core::repo_map::graph_builder::build_graph_at;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\nversion='0.1.0'\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "mod a; mod b; mod caller;").unwrap();
    std::fs::write(root.join("src/a.rs"), "pub fn alpha() {}").unwrap();
    std::fs::write(root.join("src/b.rs"), "pub fn beta() {}").unwrap();
    std::fs::write(root.join("src/caller.rs"), "fn run() { crate::a::alpha(); }").unwrap();
    let db = dir.path().join("graph.db");
    drop(build_graph_at(&root, &db, &[]).unwrap());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_calls BEFORE INSERT ON edges WHEN NEW.kind = 'Calls' BEGIN SELECT RAISE(ABORT, 'review injected failure'); END;").unwrap();
    std::fs::write(root.join("src/caller.rs"), "fn run() { crate::b::beta(); }").unwrap();
    assert!(build_graph_at(&root, &db, &[]).is_err());
    conn.execute_batch("DROP TRIGGER reject_calls;").unwrap();
    let (store, result) = build_graph_at(&root, &db, &[]).unwrap();
    let edges = store.file_edges().unwrap();
    println!("recovery: changed {} reresolved {}; edges {edges:?}", result.files_changed, result.files_reresolved);
    assert!(edges.iter().any(|(from, to, k)| from == "src/caller.rs" && to == "src/b.rs" && k.as_str() == "Calls"));
}

#[test]
fn transient_parse_status_is_retried() {
    use gaviero_core::repo_map::graph_builder::build_graph_at;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rs"), "pub fn alpha() {}").unwrap();
    let db = dir.path().join("graph.db");
    drop(build_graph_at(&root, &db, &[]).unwrap());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DELETE FROM nodes WHERE kind != 'File'; UPDATE file_extraction SET outcome = 'parse_timeout', symbol_count = 0;").unwrap();
    let (store, result) = build_graph_at(&root, &db, &[]).unwrap();
    println!("parse recovery: changed {}; status {:?}", result.files_changed, store.file_extraction("a.rs").unwrap());
    assert!(store.all_definitions().unwrap().iter().any(|d| d.name == "alpha"));
}

#[test]
fn typescript_default_reexport_reaches_definition() {
    let dir = tempfile::tempdir().unwrap();
    for (file, text) in [
        ("impl.ts", "export default function work() {}"),
        ("barrel.ts", "export { default } from './impl';"),
        ("caller.ts", "import work from './barrel'; work();"),
    ] {
        std::fs::write(dir.path().join(file), text).unwrap();
    }
    let store = GraphStore::open_memory().unwrap();
    build_graph_into(&store, dir.path(), &[]).unwrap();
    let ctx = ResolveContext::build(&store, dir.path()).unwrap();
    let extracted = extract_file("ts", "import work from './barrel'; work();");
    let scope = FileScope::build(Lang::TypeScript, &extracted.refs);
    let r = extracted.refs.iter().find(|r| r.kind == gaviero_core::repo_map::edges::RefKind::Call).unwrap();
    assert_target(ctx.resolve("caller.ts", Lang::TypeScript, &scope, r), "impl.ts");
}
