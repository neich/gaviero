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

use super::builder::{SKIP_DIRS, is_excluded};
use super::edges::{
    extract_file, extract_references_for_extension, is_test_file, node_kind_from_ts, resolve_and_insert_edges,
};
use super::extraction::{EXTRACTION_POLICY_META_KEY, EXTRACTION_POLICY_VERSION, extract, read_source};
use super::resolve::{Lang, ResolveContext, ResolveStats, lang_of, reexports_of, resolve_and_insert};
use super::store::{EdgeKind, FileExtraction, GraphStore, NodeKind};

/// Result of an incremental build.
#[derive(Debug, Default)]
pub struct BuildResult {
    /// Number of files scanned.
    pub files_scanned: usize,
    /// Number of files that were re-indexed (changed since last build, or
    /// every file when the extraction policy changed).
    pub files_changed: usize,
    /// Number of files that were unchanged (skipped).
    pub files_unchanged: usize,
    /// Number of files removed from the graph (deleted from disk).
    pub files_removed: usize,
    /// Files that could not be read (or were over the read limit). Their
    /// old graph data is removed and they are retried on the next build.
    pub files_unreadable: usize,
    /// True when the stored extraction-policy version was missing or
    /// different, so unchanged files were re-extracted too.
    pub policy_rebuild: bool,
    /// Total nodes in the graph after build.
    pub total_nodes: usize,
    /// Total edges in the graph after build.
    pub total_edges: usize,
    /// Unchanged files whose references were resolved again because
    /// another file's definitions or re-exports changed.
    pub files_reresolved: usize,
    /// How this build's references resolved (structured languages only).
    pub resolve: ResolveStats,
}

#[cfg(test)]
thread_local! {
    /// Test hook: fail the build after phase 1 (nodes + statuses written,
    /// edges and the policy version not yet written).
    static FAIL_AFTER_PHASE_1: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// On-disk graph database for `workspace` (`{workspace}/.gaviero/code_graph.db`).
pub fn graph_db_path(workspace: &Path) -> PathBuf {
    workspace.join(".gaviero").join("code_graph.db")
}

/// Build or incrementally update the code knowledge graph for a workspace.
///
/// The graph database is stored at [`graph_db_path`].
/// `excludes` is a list of folder names or glob patterns to skip
/// (see [`crate::repo_map::builder::is_excluded`]).
pub fn build_graph(workspace: &Path, excludes: &[String]) -> Result<(GraphStore, BuildResult)> {
    build_graph_at(workspace, &graph_db_path(workspace), excludes)
}

/// Like [`build_graph`], but persists the graph at `db_path` instead of
/// `{workspace}/.gaviero/code_graph.db`.
///
/// One database holds one scan root: an incremental build deletes every
/// stored file the scan did not see, so two roots must never share a
/// `db_path`.
pub fn build_graph_at(
    workspace: &Path,
    db_path: &Path,
    excludes: &[String],
) -> Result<(GraphStore, BuildResult)> {
    let store = GraphStore::open(db_path)
        .with_context(|| format!("opening graph store at {}", db_path.display()))?;

    let result = incremental_build(&store, workspace, excludes)?;
    Ok((store, result))
}

/// Build into an existing (possibly in-memory) store.
pub fn build_graph_into(
    store: &GraphStore,
    workspace: &Path,
    excludes: &[String],
) -> Result<BuildResult> {
    incremental_build(store, workspace, excludes)
}

fn incremental_build(
    store: &GraphStore,
    workspace: &Path,
    excludes: &[String],
) -> Result<BuildResult> {
    let mut result = BuildResult::default();
    // Hashes, definitions, reference edges and policy metadata form one
    // snapshot. Roll back all of them on failure so the next build retries
    // changed files and still detects changes affecting unchanged callers.
    let transaction = store.build_transaction()?;

    // A graph extracted under another (or no recorded) policy is re-extracted
    // in full: unchanged hashes must not hide a cap or exclusion change.
    let policy_current = store.graph_meta(EXTRACTION_POLICY_META_KEY)?.as_deref()
        == Some(EXTRACTION_POLICY_VERSION);
    result.policy_rebuild = !policy_current;

    // 1. Collect all source files
    let mut source_files: Vec<(PathBuf, PathBuf)> = Vec::new(); // (abs_path, rel_path)
    collect_source_files(workspace, workspace, &mut source_files, excludes)?;
    result.files_scanned = source_files.len();

    // 2. Get existing file hashes
    let existing_hashes: std::collections::HashMap<String, String> =
        store.all_file_hashes()?.into_iter().collect();

    // 3. Detect deleted files (in store but not on disk). A path that was
    //    unreadable has a status row but no hash, so check both.
    let current_files: std::collections::HashSet<String> = source_files
        .iter()
        .map(|(_, rel)| rel.to_string_lossy().to_string())
        .collect();
    let mut stored: std::collections::BTreeSet<String> = existing_hashes.keys().cloned().collect();
    stored.extend(store.all_extraction_paths()?);
    for stored_file in &stored {
        if !current_files.contains(stored_file) {
            store.delete_file(stored_file)?;
            result.files_removed += 1;
        }
    }

    // 4. Phase 1: Index nodes for changed files
    let mut changed_files: Vec<(PathBuf, PathBuf, String)> = Vec::new(); // (abs, rel, content)
    // What each re-indexed file offered other files before this build
    // (definitions + re-exports). A removed file always counts as a change.
    let mut old_signatures: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut signature_changed = result.files_removed > 0;
    for (abs_path, rel_path) in &source_files {
        let rel_str = rel_path.to_string_lossy().to_string();

        // Read and hash file. An unreadable or oversized file must not leave
        // its previous symbols looking current: drop them, record why, and
        // leave no hash so the next build retries it.
        let content = match read_source(abs_path) {
            Ok(c) => c,
            Err(outcome) => {
                if !store.definition_signature(&rel_str)?.is_empty() {
                    signature_changed = true;
                }
                store.delete_file(&rel_str)?;
                store.set_file_extraction(&FileExtraction {
                    file_path: rel_str.clone(),
                    outcome: outcome.as_str().to_string(),
                    policy_version: EXTRACTION_POLICY_VERSION.to_string(),
                    content_hash: None,
                    content_bytes: std::fs::metadata(abs_path).ok().map(|m| m.len() as i64),
                    symbol_count: 0,
                })?;
                result.files_unreadable += 1;
                continue;
            }
        };
        let hash = hash_content(&content);
        let same_content = existing_hashes.get(&rel_str).map(|h| h.as_str()) == Some(hash.as_str());

        // Skip if unchanged under the current policy
        if same_content && policy_current {
            result.files_unchanged += 1;
            continue;
        }

        // Re-index. Unchanged content (a policy rebuild) keeps its
        // enrichment rows; changed content invalidates everything.
        old_signatures.insert(rel_str.clone(), store.definition_signature(&rel_str)?);
        if same_content {
            store.delete_file_graph(&rel_str)?;
        } else {
            store.delete_file(&rel_str)?;
        }
        store.set_file_hash(&rel_str, &hash)?;
        result.files_changed += 1;

        // Extract symbols and insert nodes
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let extraction = extract(ext, &content);
        let symbols = &extraction.symbols;
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
        for sym in symbols {
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

            if sym.owner.is_some() {
                store.set_node_owner(&qn, sym.owner.as_deref())?;
            }

            // Defines edge: file defines symbol
            store.insert_edge(EdgeKind::Defines, &rel_str, &qn, &rel_str, sym.line as i64)?;
        }
        if same_content {
            store.prune_symbol_docs_for(&rel_str)?;
        }
        store.set_file_extraction(&FileExtraction {
            file_path: rel_str.clone(),
            outcome: extraction.outcome.as_str().to_string(),
            policy_version: EXTRACTION_POLICY_VERSION.to_string(),
            content_hash: Some(hash),
            content_bytes: Some(content.len() as i64),
            symbol_count: symbols.len() as i64,
        })?;

        if extraction.outcome.allows_edges() {
            changed_files.push((abs_path.clone(), rel_path.clone(), content));
        }
    }

    #[cfg(test)]
    if FAIL_AFTER_PHASE_1.with(|f| f.get()) {
        anyhow::bail!("injected failure after phase 1");
    }

    // 5. Phase 2: Extract and resolve edges for changed files
    //    (done in a second pass so the symbol table is complete)
    // 5a. Extract every changed file's references and persist its
    //     re-exports and (Rust) type facts first: resolving one file can
    //     follow another's `pub use` or field types, so all of them must be
    //     in place before 5b.
    let mut pending: Vec<(String, Option<Lang>, Vec<super::edges::RawReference>)> = Vec::new();
    for (abs_path, rel_path, content) in &changed_files {
        let rel_str = rel_path.to_string_lossy().to_string();
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let extracted = extract_file(ext, content);
        let refs = extracted.refs;
        let lang = lang_of(&rel_str).filter(|l| structured(*l));
        if let Some(lang) = lang {
            store.replace_reexports(&rel_str, &reexports_of(lang, &refs))?;
        }
        if let Some(facts) = &extracted.facts {
            store.replace_type_facts(&rel_str, facts)?;
        }
        pending.push((rel_str, lang, refs));
    }

    // 5b. Staleness. An unchanged file's edges depend on other files'
    //     definitions and re-exports (a name can become unique, ambiguous,
    //     or move). When any of those changed, every unchanged file of a
    //     structured language is re-resolved — measured cheap enough on
    //     this repo to beat tracking who depended on what (see
    //     ARCHITECTURE.md). A content-only edit leaves others alone.
    if !signature_changed {
        for (file, old) in &old_signatures {
            if store.definition_signature(file)? != *old {
                signature_changed = true;
                break;
            }
        }
    }
    if signature_changed {
        for (abs_path, rel_path) in &source_files {
            let rel_str = rel_path.to_string_lossy().to_string();
            let Some(lang) = lang_of(&rel_str).filter(|l| structured(*l)) else {
                continue;
            };
            if old_signatures.contains_key(&rel_str) {
                continue;
            }
            let eligible = store
                .file_extraction(&rel_str)?
                .and_then(|r| super::extraction::ExtractionOutcome::parse(&r.outcome))
                .is_some_and(|o| o.allows_edges());
            if !eligible {
                continue;
            }
            let Ok(content) = read_source(abs_path) else {
                continue;
            };
            let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let refs = extract_references_for_extension(ext, &content);
            store.delete_reference_edges(&rel_str)?;
            result.files_reresolved += 1;
            pending.push((rel_str, Some(lang), refs));
        }
    }

    // 5c. Resolve.
    let ctx = if pending.iter().any(|(_, lang, _)| lang.is_some()) {
        Some(ResolveContext::build(store, workspace)?)
    } else {
        None
    };
    for (rel_str, lang, refs) in &pending {
        if refs.is_empty() {
            continue;
        }
        match (lang, &ctx) {
            (Some(lang), Some(ctx)) => {
                resolve_and_insert(store, ctx, rel_str, *lang, refs, &mut result.resolve)?;
            }
            _ => {
                resolve_and_insert_edges(store, rel_str, rel_str, refs)?;
            }
        }
    }
    if result.resolve.dropped_ambiguous > 0 {
        let mut top: Vec<(&String, &usize)> = result.resolve.dropped_names.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        top.truncate(10);
        tracing::debug!(
            dropped = result.resolve.dropped_ambiguous,
            top = ?top,
            "repo_map: ambiguous references dropped"
        );
    }

    // Publish the policy with the completed graph, never a partial build.
    store.set_graph_meta(EXTRACTION_POLICY_META_KEY, EXTRACTION_POLICY_VERSION)?;

    let (total_nodes, total_edges) = store.stats()?;
    result.total_nodes = total_nodes;
    result.total_edges = total_edges;
    transaction.commit()?;

    Ok(result)
}

/// Languages whose references go through [`super::resolve`] (the rest
/// keep bare-name resolution).
fn structured(lang: Lang) -> bool {
    matches!(lang, Lang::Rust | Lang::TypeScript | Lang::Python)
}

fn collect_source_files(
    dir: &Path,
    workspace: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
    excludes: &[String],
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

        let rel = path.strip_prefix(workspace).unwrap_or(&path);
        if is_excluded(file_name, rel, excludes) {
            continue;
        }

        if path.is_dir() {
            collect_source_files(&path, workspace, out, excludes)?;
        } else if path.is_file() {
            // Only index files with known extensions
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if language_for_extension(ext).is_some() {
                // Store keys and LLM-facing outputs (blast_radius,
                // impact texts) use `/`-separated rel paths on all
                // hosts; abs_path stays native for I/O.
                out.push((path.clone(), super::normalize_rel_path(rel)));
            }
        }
    }

    Ok(())
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
        let (_, result) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.files_changed, 0);
    }

    #[test]
    fn build_indexes_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn hello() {}\nfn internal() {}",
        )
        .unwrap();

        let (store, result) = build_graph(dir.path(), &[]).unwrap();
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
    fn build_graph_at_persists_outside_the_scan_root() {
        let src = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("lib.rs"), "pub fn hello() {}").unwrap();
        let db = state.path().join("graphs").join("code_graph.db");

        let (store, result) = build_graph_at(src.path(), &db, &[]).unwrap();
        assert_eq!(result.files_changed, 1);
        assert!(!store.nodes_for_file("lib.rs").unwrap().is_empty());
        assert!(db.is_file());
        assert!(!src.path().join(".gaviero").exists());
    }

    #[test]
    fn incremental_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();

        let (_, r1) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r1.files_changed, 1);

        // Second build: same content → should skip
        let (_, r2) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r2.files_changed, 0);
        assert_eq!(r2.files_unchanged, 1);
    }

    #[test]
    fn incremental_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();

        let (_, r1) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r1.files_changed, 1);

        // Modify the file
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}\nfn bar() {}").unwrap();

        let (store, r2) = build_graph(dir.path(), &[]).unwrap();
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

        let (_, r1) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r1.files_scanned, 2);

        // Delete b.rs
        std::fs::remove_file(dir.path().join("b.rs")).unwrap();

        let (store, r2) = build_graph(dir.path(), &[]).unwrap();
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
        std::fs::write(dir.path().join("b.rs"), "fn main() { let x = compute(); }").unwrap();

        let (store, result) = build_graph(dir.path(), &[]).unwrap();
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
        )
        .unwrap();

        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        let impact = store.impact_radius(&["lib.rs"], 3).unwrap();
        assert!(
            impact.affected_tests.iter().any(|t| t.contains("test_lib")),
            "test file should be in affected_tests: {:?}",
            impact
        );
    }

    // ── staleness of unchanged files (edge-resolution E6) ───────────

    #[test]
    fn a_content_only_edit_does_not_re_resolve_other_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() { 1; }\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() { alpha(); }\n").unwrap();
        drop(build_graph(dir.path(), &[]).unwrap());

        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() { 2; }\n").unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r.files_changed, 1);
        assert_eq!(r.files_reresolved, 0, "same definitions: b.rs keeps its edges as they are");
        let impact = store.impact_radius(&["a.rs"], 1).unwrap();
        assert!(impact.affected_files.contains(&"b.rs".to_string()));
    }

    #[test]
    fn a_new_definition_re_resolves_unchanged_callers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() { gv_late(); }\n").unwrap();
        drop(build_graph(dir.path(), &[]).unwrap());

        std::fs::write(dir.path().join("b.rs"), "pub fn gv_late() {}\n").unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r.files_changed, 1);
        assert_eq!(r.files_reresolved, 1, "a.rs is unchanged but its call now resolves");
        let impact = store.impact_radius(&["b.rs"], 1).unwrap();
        assert!(impact.affected_files.contains(&"a.rs".to_string()), "{impact:?}");

        // Removing the definition drops the edge again.
        std::fs::remove_file(dir.path().join("b.rs")).unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(r.files_reresolved, 1);
        assert!(store.file_edges().unwrap().is_empty(), "{:?}", store.file_edges().unwrap());
    }

    fn edges_from(store: &GraphStore, file: &str) -> Vec<(String, EdgeKind)> {
        let mut out: Vec<_> = store
            .file_edges()
            .unwrap()
            .into_iter()
            .filter(|(src, _, _)| src == file)
            .map(|(_, tgt, kind)| (tgt, kind))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.as_str().cmp(b.1.as_str())));
        out.dedup();
        out
    }

    #[test]
    fn import_resolved_edges_survive_a_homonym_and_drop_on_rename() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        std::fs::write(src.join("lib.rs"), "pub mod a;\npub mod b;\npub mod d;\n").unwrap();
        std::fs::write(src.join("a.rs"), "pub fn helper() {}\n").unwrap();
        std::fs::write(src.join("b.rs"), "use crate::a::helper;\npub fn f() { helper(); }\n").unwrap();
        std::fs::write(src.join("d.rs"), "pub fn other() {}\n").unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        let expected = vec![
            ("src/a.rs".to_string(), EdgeKind::Calls),
            ("src/a.rs".to_string(), EdgeKind::Imports),
        ];
        assert_eq!(edges_from(&store, "src/b.rs"), expected);

        // A second `helper` elsewhere changes the definition set, so b.rs is
        // re-resolved, but its import still decides.
        std::fs::write(src.join("d.rs"), "pub fn other() {}\npub fn helper() {}\n").unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert!(r.files_reresolved >= 1);
        assert_eq!(edges_from(&store, "src/b.rs"), expected, "no edge to d.rs");

        // Renaming the imported definition removes the stale edge from the
        // unchanged importer (and does not fall back to d.rs's homonym).
        std::fs::write(src.join("a.rs"), "pub fn helper_renamed() {}\n").unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(edges_from(&store, "src/b.rs"), vec![]);
    }

    // ── staleness through type facts (receiver-typing R6) ──────────

    fn demo_crate(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        for (p, src) in files {
            std::fs::write(dir.path().join("src").join(p), src).unwrap();
        }
        dir
    }

    fn calls_from(store: &GraphStore, file: &str) -> Vec<String> {
        edges_from(store, file)
            .into_iter()
            .filter(|(_, k)| *k == EdgeKind::Calls)
            .map(|(t, _)| t)
            .collect()
    }

    const TWO_TARGETS: [(&str, &str); 3] = [
        ("lib.rs", "pub mod a;\npub mod b;\npub mod c;\npub mod d;\n"),
        ("c.rs", "pub struct P;\nimpl P { pub fn hit(&self) {} }\n"),
        ("d.rs", "pub struct Q;\nimpl Q { pub fn hit(&self) {} }\n"),
    ];

    #[test]
    fn a_field_type_change_moves_an_unchanged_callers_edge() {
        let mut files = TWO_TARGETS.to_vec();
        files.push(("a.rs", "use crate::c::P;\npub struct A { pub f: P }\nimpl A { pub fn m(&self) -> u8 { 1 } }\n"));
        files.push(("b.rs", "use crate::a::A;\npub fn g(x: &A) { x.f.hit(); }\n"));
        let dir = demo_crate(&files);
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(calls_from(&store, "src/b.rs"), ["src/c.rs"]);

        // A body-only edit to a.rs: nothing else is re-resolved.
        std::fs::write(
            dir.path().join("src/a.rs"),
            "use crate::c::P;\npub struct A { pub f: P }\nimpl A { pub fn m(&self) -> u8 { 2 } }\n",
        )
        .unwrap();
        let (_, r) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!((r.files_changed, r.files_reresolved), (1, 0), "same type facts");

        // The field's type changes: b.rs is unchanged but its call moves.
        std::fs::write(dir.path().join("src/a.rs"), "use crate::d::Q;\npub struct A { pub f: Q }\n").unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert!(r.files_reresolved >= 1);
        assert_eq!(calls_from(&store, "src/b.rs"), ["src/d.rs"]);
    }

    #[test]
    fn an_import_that_types_a_field_re_resolves_its_users() {
        let mut files = TWO_TARGETS.to_vec();
        // Two types named `P`, and a.rs imports neither: the field's type
        // cannot be resolved, and `hit` by name spans two files.
        files[2] = ("d.rs", "pub struct P;\nimpl P { pub fn hit(&self) {} }\n");
        files.push(("a.rs", "pub struct A { pub f: P }\n"));
        files.push(("b.rs", "use crate::a::A;\npub fn g(x: &A) { x.f.hit(); }\n"));
        let dir = demo_crate(&files);
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert!(!calls_from(&store, "src/b.rs").contains(&"src/c.rs".to_string()));

        std::fs::write(dir.path().join("src/a.rs"), "use crate::c::P;\npub struct A { pub f: P }\n").unwrap();
        let (store, r) = build_graph(dir.path(), &[]).unwrap();
        assert!(r.files_reresolved >= 1, "the new `use` is part of a.rs's signature");
        assert_eq!(calls_from(&store, "src/b.rs"), ["src/c.rs"]);
    }

    // ── extraction status + policy invalidation (X6) ────────────────

    fn status(store: &GraphStore, path: &str) -> Option<String> {
        store.file_extraction(path).unwrap().map(|r| r.outcome)
    }

    fn policy(store: &GraphStore) -> Option<String> {
        store.graph_meta(EXTRACTION_POLICY_META_KEY).unwrap()
    }

    #[test]
    fn statuses_persist_and_distinguish_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}\nfn x() {}\n").unwrap();
        std::fs::write(dir.path().join("mods.rs"), "mod a;\n// nothing defined\n").unwrap();
        let (store, result) = build_graph(dir.path(), &[]).unwrap();
        assert!(result.policy_rebuild, "a fresh graph has no recorded policy");
        assert_eq!(policy(&store).as_deref(), Some(EXTRACTION_POLICY_VERSION));
        drop(store);

        let store = GraphStore::open(&graph_db_path(dir.path())).unwrap();
        let lib = store.file_extraction("lib.rs").unwrap().expect("survives reopen");
        assert_eq!(lib.outcome, "symbols");
        assert_eq!(lib.policy_version, EXTRACTION_POLICY_VERSION);
        assert_eq!(lib.symbol_count, 2);
        assert!(lib.content_hash.is_some());
        assert_eq!(status(&store, "mods.rs").as_deref(), Some("no_definitions"));
        assert_eq!(status(&store, "README.md"), None, "never indexed → unknown/not_indexed");
    }

    #[test]
    fn a_policy_change_re_extracts_unchanged_files_and_keeps_their_enrichment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() { alpha(); }\n").unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        store
            .upsert_symbol_doc(&super::super::store::SymbolDoc {
                qualified_name: "a.rs::alpha".into(),
                file_path: "a.rs".into(),
                file_hash: None,
                signature: "pub fn alpha()".into(),
                bounds: String::new(),
                doc: String::new(),
                role_summary: String::new(),
                embedding: None,
            })
            .unwrap();
        let (_, second) = build_graph(dir.path(), &[]).unwrap();
        assert!(!second.policy_rebuild);
        assert_eq!(second.files_changed, 0);

        store.set_graph_meta(EXTRACTION_POLICY_META_KEY, "0").unwrap();
        let (store, bumped) = build_graph(dir.path(), &[]).unwrap();
        assert!(bumped.policy_rebuild);
        assert_eq!(bumped.files_changed, 2, "unchanged hashes must not skip extraction");
        assert_eq!(bumped.files_unchanged, 0);
        assert_eq!(policy(&store).as_deref(), Some(EXTRACTION_POLICY_VERSION));
        assert!(
            store.symbol_doc("a.rs::alpha").unwrap().is_some(),
            "unchanged content keeps its enrichment"
        );
        let impact = store.impact_radius(&["a.rs"], 2).unwrap();
        assert!(impact.affected_files.contains(&"b.rs".to_string()), "edges rebuilt");

        // Changed content still invalidates enrichment.
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() { }\n").unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert!(store.symbol_doc("a.rs::alpha").unwrap().is_none());
    }

    #[test]
    fn a_legacy_database_reads_unknown_then_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        let db = graph_db_path(dir.path());
        drop(build_graph(dir.path(), &[]).unwrap());
        // Make it look like a database from before this change.
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "DROP TABLE file_extraction; DELETE FROM graph_meta WHERE key = 'extraction_policy_version';",
        )
        .unwrap();
        drop(conn);

        let store = GraphStore::open(&db).unwrap();
        assert_eq!(status(&store, "a.rs"), None, "legacy rows are unknown, not inferred");
        assert!(!store.nodes_for_file("a.rs").unwrap().is_empty(), "old graph still served");
        drop(store);

        let (store, result) = build_graph(dir.path(), &[]).unwrap();
        assert!(result.policy_rebuild);
        assert_eq!(result.files_changed, 1);
        assert_eq!(status(&store, "a.rs").as_deref(), Some("symbols"));
    }

    #[test]
    fn a_failed_build_does_not_claim_the_policy_and_the_next_one_recovers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() { alpha(); }\n").unwrap();
        FAIL_AFTER_PHASE_1.with(|f| f.set(true));
        let failed = build_graph(dir.path(), &[]);
        FAIL_AFTER_PHASE_1.with(|f| f.set(false));
        assert!(failed.is_err());
        let store = GraphStore::open(&graph_db_path(dir.path())).unwrap();
        assert_eq!(policy(&store), None, "an incomplete build must not record the policy");
        drop(store);

        let (store, result) = build_graph(dir.path(), &[]).unwrap();
        assert!(result.policy_rebuild);
        assert_eq!(result.files_changed, 2, "both files re-extracted despite stored hashes");
        assert_eq!(policy(&store).as_deref(), Some(EXTRACTION_POLICY_VERSION));
        let impact = store.impact_radius(&["a.rs"], 2).unwrap();
        assert!(impact.affected_files.contains(&"b.rs".to_string()), "edges completed");
    }

    #[test]
    fn failed_incremental_phase_one_rolls_back_on_a_reused_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "pub fn alpha() {}").unwrap();
        let store = GraphStore::open_memory().unwrap();
        build_graph_into(&store, dir.path(), &[]).unwrap();
        let hashes = store.all_file_hashes().unwrap();
        std::fs::write(&file, "pub fn beta() {}").unwrap();

        FAIL_AFTER_PHASE_1.with(|f| f.set(true));
        let failed = build_graph_into(&store, dir.path(), &[]);
        FAIL_AFTER_PHASE_1.with(|f| f.set(false));
        assert!(failed.is_err());
        assert_eq!(store.all_file_hashes().unwrap(), hashes);
        assert!(!store.resolve_symbol("alpha").unwrap().is_empty());
        assert!(store.resolve_symbol("beta").unwrap().is_empty());

        let result = build_graph_into(&store, dir.path(), &[]).unwrap();
        assert!(!result.policy_rebuild);
        assert_eq!(result.files_changed, 1);
        assert!(!store.resolve_symbol("beta").unwrap().is_empty());
    }

    #[test]
    fn an_unreadable_file_drops_stale_symbols_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert!(!store.nodes_for_file("a.rs").unwrap().is_empty());

        std::fs::write(&file, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let (store, result) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(result.files_unreadable, 1);
        assert!(store.nodes_for_file("a.rs").unwrap().is_empty(), "no stale symbols");
        let rec = store.file_extraction("a.rs").unwrap().unwrap();
        assert_eq!(rec.outcome, "read_failed");
        assert!(rec.content_hash.is_none());
        assert_eq!(rec.content_bytes, Some(4));
        assert!(!store.all_file_hashes().unwrap().iter().any(|(p, _)| p == "a.rs"));

        // A second build retries (no hash to match) and still reports it.
        let (_, again) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(again.files_unreadable, 1);

        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let (store, healed) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(healed.files_changed, 1);
        assert_eq!(status(&store, "a.rs").as_deref(), Some("symbols"));
        assert!(!store.nodes_for_file("a.rs").unwrap().is_empty());
    }

    #[test]
    fn deleting_a_file_cleans_its_status_hash_and_nodes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(dir.path().join("bad.rs"), [0xffu8, 0xfe]).unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert!(status(&store, "a.rs").is_some() && status(&store, "bad.rs").is_some());

        std::fs::remove_file(dir.path().join("a.rs")).unwrap();
        std::fs::remove_file(dir.path().join("bad.rs")).unwrap();
        let (store, result) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(result.files_removed, 2, "the hashless unreadable path is removed too");
        for p in ["a.rs", "bad.rs"] {
            assert_eq!(status(&store, p), None, "{p}");
            assert!(store.nodes_for_file(p).unwrap().is_empty(), "{p}");
        }
        assert!(store.all_file_hashes().unwrap().is_empty());
    }

    /// A deeply nested Rust file survives the whole build (symbol walk and
    /// query-based reference extraction) on a small stack.
    #[test]
    fn a_deeply_nested_file_does_not_abort_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let depth = 60_000;
        std::fs::write(
            dir.path().join("deep.rs"),
            format!("pub fn deep() -> i32 {{ {}1{} }}\n", "(".repeat(depth), ")".repeat(depth)),
        )
        .unwrap();
        let root = dir.path().to_path_buf();
        let outcome = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let (store, _) = build_graph(&root, &[]).unwrap();
                store.file_extraction("deep.rs").unwrap().map(|r| r.outcome)
            })
            .unwrap()
            .join()
            .expect("build must not overflow");
        assert_eq!(outcome.as_deref(), Some("symbols"));
    }

    #[test]
    fn generated_sources_are_kept_out_of_symbols_and_edges() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("helper.rs"), "pub fn gv_gen_helper() {}\n").unwrap();
        std::fs::write(
            dir.path().join("gen.rs"),
            "// @generated by a code generator\npub fn gv_gen_caller() { gv_gen_helper(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("hand.rs"),
            "pub fn gv_hand_caller() { gv_gen_helper(); }\n",
        )
        .unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(status(&store, "gen.rs").as_deref(), Some("generated_excluded"));
        let nodes = store.nodes_for_file("gen.rs").unwrap();
        assert_eq!(nodes.len(), 1, "file identity kept");
        assert_eq!(nodes[0].kind, "File");
        let impact = store.impact_radius(&["helper.rs"], 1).unwrap();
        assert!(impact.affected_files.contains(&"hand.rs".to_string()));
        assert!(
            !impact.affected_files.contains(&"gen.rs".to_string()),
            "edge parsing must not bypass the generated exclusion"
        );
    }

    #[test]
    fn the_parse_cap_governs_symbols_not_edges() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("helper.rs"), "pub fn gv_big_helper() {}\n").unwrap();
        let mut big = String::from("pub fn gv_big_caller() { gv_big_helper(); }\n");
        while big.len() <= super::super::extraction::MAX_PARSE_BYTES {
            big.push_str("// padding padding padding padding padding padding\n");
        }
        std::fs::write(dir.path().join("big.rs"), &big).unwrap();
        let (store, _) = build_graph(dir.path(), &[]).unwrap();
        assert_eq!(status(&store, "big.rs").as_deref(), Some("parse_size_excluded"));
        let nodes = store.nodes_for_file("big.rs").unwrap();
        assert!(nodes.iter().all(|n| n.kind == "File"), "file identity kept, no symbols");
        assert_eq!(nodes.len(), 1);
        let impact = store.impact_radius(&["helper.rs"], 1).unwrap();
        assert!(
            impact.affected_files.contains(&"big.rs".to_string()),
            "reference extraction still parses files over the symbol cap"
        );
    }
}
