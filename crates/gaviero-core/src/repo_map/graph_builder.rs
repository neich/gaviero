//! Incremental code knowledge graph builder.
//!
//! Scans a workspace, compares file hashes against the stored graph, and
//! re-indexes only changed files. For each file:
//! 1. Extract symbols (functions, structs, traits, etc.) → nodes
//! 2. Extract references (use, calls, impl) → raw refs
//! 3. Resolve raw refs against the symbol table → edges
//!
//! The graph is persisted in SQLite via [`GraphStore`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::tree_sitter::language_for_extension;

use super::builder::{extract_symbols, SKIP_DIRS};
use super::edges::{
    extract_rust_references, is_test_file, node_kind_from_ts, resolve_and_insert_edges,
};
use super::store::{EdgeKind, GraphStore, NodeKind};

/// Result of an incremental build.
#[derive(Debug, Default)]
pub struct BuildResult {
    /// Number of files scanned.
    pub files_scanned: usize,
    /// Number of files that were re-indexed (changed since last build).
    pub files_changed: usize,
    /// Number of files that were unchanged (skipped).
    pub files_unchanged: usize,
    /// Number of files removed from the graph (deleted from disk).
    pub files_removed: usize,
    /// Total nodes in the graph after build.
    pub total_nodes: usize,
    /// Total edges in the graph after build.
    pub total_edges: usize,
}

/// Maximum file size to index (1 MB).
const MAX_FILE_BYTES: u64 = 1_000_000;

/// Build or incrementally update the code knowledge graph for a workspace.
///
/// The graph database is stored at `{workspace}/.gaviero/code_graph.db`.
pub fn build_graph(workspace: &Path) -> Result<(GraphStore, BuildResult)> {
    let db_dir = workspace.join(".gaviero");
    let db_path = db_dir.join("code_graph.db");
    let store = GraphStore::open(&db_path)
        .with_context(|| format!("opening graph store at {}", db_path.display()))?;

    let result = incremental_build(&store, workspace)?;
    Ok((store, result))
}

/// Build into an existing (possibly in-memory) store.
pub fn build_graph_into(store: &GraphStore, workspace: &Path) -> Result<BuildResult> {
    incremental_build(store, workspace)
}

fn incremental_build(store: &GraphStore, workspace: &Path) -> Result<BuildResult> {
    let mut result = BuildResult::default();

    // 1. Collect all source files
    let mut source_files: Vec<(PathBuf, PathBuf)> = Vec::new(); // (abs_path, rel_path)
    collect_source_files(workspace, workspace, &mut source_files)?;
    result.files_scanned = source_files.len();

    // 2. Get existing file hashes
    let existing_hashes: std::collections::HashMap<String, String> = store
        .all_file_hashes()?
        .into_iter()
        .collect();

    // 3. Detect deleted files (in store but not on disk)
    let current_files: std::collections::HashSet<String> = source_files
        .iter()
        .map(|(_, rel)| rel.to_string_lossy().to_string())
        .collect();
    for (stored_file, _) in &existing_hashes {
        if !current_files.contains(stored_file) {
            store.delete_file(stored_file)?;
            result.files_removed += 1;
        }
    }

    // 4. Phase 1: Index nodes for changed files
    store.begin_transaction()?;

    let mut changed_files: Vec<(PathBuf, PathBuf, String)> = Vec::new(); // (abs, rel, content)
    for (abs_path, rel_path) in &source_files {
        let rel_str = rel_path.to_string_lossy().to_string();

        // Read and hash file
        let content = match read_source(abs_path) {
            Some(c) => c,
            None => continue,
        };
        let hash = hash_content(&content);

        // Skip if unchanged
        if existing_hashes.get(&rel_str).map(|h| h.as_str()) == Some(hash.as_str()) {
            result.files_unchanged += 1;
            continue;
        }

        // File changed — delete old data and re-index
        store.delete_file(&rel_str)?;
        store.set_file_hash(&rel_str, &hash)?;
        result.files_changed += 1;

        // Extract symbols and insert nodes
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let symbols = extract_symbols(ext, &content);
        let language = crate::tree_sitter::language_name_for_extension(ext);
        let is_test = is_test_file(&rel_str);

        // Insert a File node
        store.upsert_node(
            NodeKind::File,
            &rel_str,
            &rel_str,
            &rel_str,
            None,
            None,
            language,
            Some(&hash),
        )?;

        // Insert symbol nodes
        for sym in &symbols {
            let qn = format!("{}::{}", rel_str, sym.name);
            let kind = if is_test {
                NodeKind::Test
            } else {
                node_kind_from_ts(&sym.kind, &rel_str)
            };
            store.upsert_node(
                kind,
                &sym.name,
                &qn,
                &rel_str,
                Some(sym.line as i64),
                None,
                language,
                Some(&hash),
            )?;

            // Contains edge: file contains symbol
            store.insert_edge(EdgeKind::Contains, &rel_str, &qn, &rel_str, sym.line as i64)?;
        }

        changed_files.push((abs_path.clone(), rel_path.clone(), content));
    }

    store.commit()?;

    // 5. Phase 2: Extract and resolve edges for changed files
    //    (done in a second pass so the symbol table is complete)
    store.begin_transaction()?;

    for (abs_path, rel_path, content) in &changed_files {
        let rel_str = rel_path.to_string_lossy().to_string();
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        // Currently only Rust edge extraction is implemented
        if ext == "rs" {
            let refs = extract_rust_references(content);
            let _ = resolve_and_insert_edges(store, &rel_str, &rel_str, &refs);
        }
    }

    store.commit()?;

    let (total_nodes, total_edges) = store.stats()?;
    result.total_nodes = total_nodes;
    result.total_edges = total_edges;

    Ok(result)
}

fn collect_source_files(
    dir: &Path,
    workspace: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if file_name.starts_with('.') || SKIP_DIRS.contains(&file_name) {
            continue;
        }

        if path.is_dir() {
            collect_source_files(&path, workspace, out)?;
        } else if path.is_file() {
            // Only index files with known extensions
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if language_for_extension(ext).is_some() {
                let rel = path.strip_prefix(workspace).unwrap_or(&path).to_path_buf();
                out.push((path.clone(), rel));
            }
        }
    }

    Ok(())
}

fn read_source(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn hash_content(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    format!("{:x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let (_, result) = build_graph(dir.path()).unwrap();
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.files_changed, 0);
    }

    #[test]
    fn build_indexes_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn hello() {}\nfn internal() {}",
        ).unwrap();

        let (store, result) = build_graph(dir.path()).unwrap();
        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.files_changed, 1);
        assert!(result.total_nodes >= 3); // File + 2 functions

        // Verify nodes exist
        let nodes = store.nodes_for_file("lib.rs").unwrap();
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"hello"), "nodes: {:?}", names);
        assert!(names.contains(&"internal"), "nodes: {:?}", names);
    }

    #[test]
    fn incremental_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();

        let (_, r1) = build_graph(dir.path()).unwrap();
        assert_eq!(r1.files_changed, 1);

        // Second build: same content → should skip
        let (_, r2) = build_graph(dir.path()).unwrap();
        assert_eq!(r2.files_changed, 0);
        assert_eq!(r2.files_unchanged, 1);
    }

    #[test]
    fn incremental_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();

        let (_, r1) = build_graph(dir.path()).unwrap();
        assert_eq!(r1.files_changed, 1);

        // Modify the file
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}\nfn bar() {}").unwrap();

        let (store, r2) = build_graph(dir.path()).unwrap();
        assert_eq!(r2.files_changed, 1);
        assert_eq!(r2.files_unchanged, 0);

        let nodes = store.nodes_for_file("lib.rs").unwrap();
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"bar"), "nodes after change: {:?}", names);
    }

    #[test]
    fn incremental_handles_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn b() {}").unwrap();

        let (_, r1) = build_graph(dir.path()).unwrap();
        assert_eq!(r1.files_scanned, 2);

        // Delete b.rs
        std::fs::remove_file(dir.path().join("b.rs")).unwrap();

        let (store, r2) = build_graph(dir.path()).unwrap();
        assert_eq!(r2.files_scanned, 1);
        assert_eq!(r2.files_removed, 1);
        assert!(store.nodes_for_file("b.rs").unwrap().is_empty());
    }

    #[test]
    fn cross_file_edges_created() {
        let dir = tempfile::tempdir().unwrap();

        // a.rs defines `compute`
        std::fs::write(dir.path().join("a.rs"), "pub fn compute() -> i32 { 42 }").unwrap();

        // b.rs calls `compute`
        std::fs::write(
            dir.path().join("b.rs"),
            "fn main() { let x = compute(); }",
        ).unwrap();

        let (store, result) = build_graph(dir.path()).unwrap();
        assert!(result.total_edges > 0, "expected some edges, got 0");

        // Changing a.rs should affect b.rs
        let impact = store.impact_radius(&["a.rs"], 3).unwrap();
        assert!(
            impact.affected_files.contains(&"b.rs".to_string()),
            "b.rs should be affected: {:?}",
            impact.affected_files
        );
    }

    #[test]
    fn test_files_detected_in_impact() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tests")).unwrap();

        std::fs::write(dir.path().join("lib.rs"), "pub fn process() {}").unwrap();
        std::fs::write(
            dir.path().join("tests").join("test_lib.rs"),
            "fn test_process() { process(); }",
        ).unwrap();

        let (store, _) = build_graph(dir.path()).unwrap();
        let impact = store.impact_radius(&["lib.rs"], 3).unwrap();
        assert!(
            impact.affected_tests.iter().any(|t| t.contains("test_lib")),
            "test file should be in affected_tests: {:?}",
            impact
        );
    }
}
