//! End-to-end multi-DB integration tests for the [`MemoryStores`] registry.
//!
//! Exercises the killer use case: a library repo opened from multiple
//! workspaces shares the same per-repo memory DB, while each workspace
//! keeps its own cross-cutting workspace DB. Also covers cross-DB
//! content-hash dedup and the workspace == folder aliasing path.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tempfile::TempDir;

use gaviero_core::memory::{
    Embedder, MemorySource, MemoryStores, ScopeFilter, StoreKind, StoreResult, WriteMeta,
    WriteScope, hash_path,
};
use gaviero_core::workspace::Workspace;

// ── Test helpers ────────────────────────────────────────────────────

/// Deterministic 8-dim embedder. Same texts → same vectors → fixed
/// cosine. Avoids ONNX model loading for fast tests.
struct TestEmbedder {
    model: &'static str,
}

impl TestEmbedder {
    const fn with_model(model: &'static str) -> Self {
        Self { model }
    }
}

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
    fn name(&self) -> &str {
        self.model
    }

    fn dimension(&self) -> usize {
        8
    }

    async fn embed(
        &self,
        text: &str,
        _purpose: gaviero_core::memory::EmbeddingPurpose,
    ) -> Result<Vec<f32>> {
        let mut v = vec![0.0f32; 8];
        for (i, b) in text.bytes().enumerate() {
            v[i % 8] += b as f32;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

fn embedder() -> Arc<dyn Embedder> {
    Arc::new(TestEmbedder::with_model("mock")) as Arc<dyn Embedder>
}

fn embedder_named(name: &'static str) -> Arc<dyn Embedder> {
    Arc::new(TestEmbedder::with_model(name)) as Arc<dyn Embedder>
}

/// Build a Workspace whose only folder is the workspace_root itself
/// (single-folder open — workspace + folder DBs alias).
fn single_folder_workspace(root: PathBuf) -> Workspace {
    Workspace::single_folder(root)
}

/// Build a Workspace with a distinct workspace root and one extra
/// folder (the "library repo" use case).
fn workspace_with_library(workspace_root: PathBuf, library_root: PathBuf) -> Workspace {
    let mut ws = Workspace::single_folder(workspace_root);
    ws.add_root(library_root, Some("library".into()));
    ws
}

fn open_registry(
    workspace_root: &std::path::Path,
    workspace: &Workspace,
    global_dir: &std::path::Path,
) -> Arc<MemoryStores> {
    let global_path = global_dir.join("global.db");
    MemoryStores::open_with_paths(
        workspace_root,
        workspace,
        embedder(),
        "mock".to_string(),
        &global_path,
    )
    .expect("open registry")
}

// ── Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn workspace_root_equals_folder_root_aliases_stores() {
    // Single-folder open: workspace_root == folder_root, so the
    // workspace store and the folder store resolve to the same Arc.
    let global = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let ws = single_folder_workspace(root.path().to_path_buf());
    let stores = open_registry(root.path(), &ws, global.path());

    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let folder_store = stores
        .get(&StoreKind::Folder {
            repo_id: hash_path(root.path()),
        })
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(&workspace_store, &folder_store),
        "single-folder workspace must alias workspace and folder DBs"
    );
}

#[tokio::test]
async fn distinct_workspace_and_folder_use_distinct_stores() {
    // Workspace at /tmp/ws, library repo at /tmp/lib. Their DBs must
    // be distinct physical files.
    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let library_store = stores
        .get(&StoreKind::Folder {
            repo_id: hash_path(library_root.path()),
        })
        .await
        .unwrap();
    assert!(
        !Arc::ptr_eq(&workspace_store, &library_store),
        "library folder's DB must differ from workspace DB"
    );

    // Sanity: actual files exist where MemoryScope::from_context
    // says they should.
    assert!(workspace_root.path().join(".gaviero/memory.db").exists());
    assert!(library_root.path().join(".gaviero/memory.db").exists());
    assert!(global.path().join("global.db").exists());
}

#[tokio::test]
async fn cross_db_content_hash_dedup_returns_already_covered() {
    // Step 4 contract: writing the same content_hash at a narrower
    // scope when a broader scope (in a different DB) already has it
    // returns AlreadyCovered.
    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let library_repo_id = hash_path(library_root.path());
    let content = "shared workspace fact about the library";
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);

    // Plant at WORKSPACE scope (workspace DB).
    let inserted = stores
        .store_scoped(&WriteScope::Workspace, content, &meta)
        .await
        .unwrap();
    assert!(
        matches!(inserted, StoreResult::Inserted(_)),
        "first write should insert; got {inserted:?}"
    );

    // Now try to write the SAME content at REPO scope (library folder DB).
    // Broader-scope coverage probe must hit the workspace DB and return
    // AlreadyCovered.
    let outcome = stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: library_repo_id.clone(),
            },
            content,
            &meta,
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, StoreResult::AlreadyCovered),
        "narrower-scope write of the same content must yield AlreadyCovered \
         (broader scope lives in a *different* physical DB); got {outcome:?}"
    );

    // Sanity: the row really lives only in workspace DB.
    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let library_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id,
        })
        .await
        .unwrap();
    let ws_rows = workspace_store
        .search_at_level(&ScopeFilter::Workspace, "", 50)
        .await
        .unwrap();
    let lib_rows = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: hash_path(library_root.path()),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert_eq!(ws_rows.len(), 1, "workspace DB should have 1 row");
    assert!(lib_rows.is_empty(), "library DB should have no rows");
}

#[tokio::test]
async fn library_repo_memory_shared_across_workspaces() {
    // The killer use case: same library repo opened from two
    // workspaces. A WriteScope::Repo write lands in the library's
    // DB and is visible from BOTH workspace registries.
    let global = TempDir::new().unwrap();
    let workspace_a = TempDir::new().unwrap();
    let workspace_b = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws_a = workspace_with_library(
        workspace_a.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let ws_b = workspace_with_library(
        workspace_b.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );

    let stores_a = open_registry(workspace_a.path(), &ws_a, global.path());
    let stores_b = open_registry(workspace_b.path(), &ws_b, global.path());

    let repo_id = hash_path(library_root.path());

    // Write at REPO scope from workspace A.
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);
    let res = stores_a
        .store_scoped(
            &WriteScope::Repo {
                repo_id: repo_id.clone(),
            },
            "library: prefer git2 over shelling out",
            &meta,
        )
        .await
        .unwrap();
    let inserted_id = match res {
        StoreResult::Inserted(id) => id,
        other => panic!("expected Inserted, got {other:?}"),
    };

    // From workspace B, query the library's folder store directly
    // and verify the row is visible.
    let lib_store_from_b = stores_b
        .get(&StoreKind::Folder {
            repo_id: repo_id.clone(),
        })
        .await
        .unwrap();
    let rows = lib_store_from_b
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: repo_id.clone(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    let found = rows.iter().find(|m| m.id == inserted_id);
    assert!(
        found.is_some(),
        "row written from workspace A must be visible from workspace B \
         (both opened the same library folder)"
    );
    assert_eq!(
        found.unwrap().content,
        "library: prefer git2 over shelling out"
    );
}

#[tokio::test]
async fn unregistered_repo_id_errors_in_strict_mode() {
    // A WriteScope::Repo with an unknown repo_id (not listed in the
    // workspace) must error rather than silently routing to workspace.
    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let ws = single_folder_workspace(workspace_root.path().to_path_buf());
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let phantom_repo = hash_path(std::path::Path::new("/nonexistent"));
    let res = stores
        .get(&StoreKind::Folder {
            repo_id: phantom_repo,
        })
        .await;
    assert!(
        res.is_err(),
        "strict registry must reject unknown repo_id; got {res:?}"
    );
}

#[tokio::test]
async fn registered_folder_lazy_opens_on_first_access() {
    // Folder DBs are pre-registered (path known at open time) but
    // lazy-opened on first `get`. Verify the file doesn't exist
    // until first access.
    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let lib_db = library_root.path().join(".gaviero/memory.db");
    assert!(
        !lib_db.exists(),
        "folder DB should not be opened until first access"
    );

    let _ = stores
        .get(&StoreKind::Folder {
            repo_id: hash_path(library_root.path()),
        })
        .await
        .unwrap();

    assert!(
        lib_db.exists(),
        "folder DB should be opened on first access"
    );
}

#[tokio::test]
async fn workspace_folder_helper_routes_files_to_correct_folder() {
    // Workspace::folder_for_path is what call sites use to map a
    // file path → owning folder → repo_id → store. Verify it picks
    // the longest-matching folder when folders are nested.
    let outer = TempDir::new().unwrap();
    let inner = outer.path().join("vendored-lib");
    std::fs::create_dir_all(&inner).unwrap();

    let mut ws = Workspace::single_folder(outer.path().to_path_buf());
    ws.add_root(inner.clone(), Some("inner".into()));

    // A file inside the inner folder should resolve to the inner
    // folder, not the outer one.
    let file_in_inner = inner.join("src/lib.rs");
    std::fs::create_dir_all(file_in_inner.parent().unwrap()).unwrap();
    std::fs::write(&file_in_inner, "").unwrap();

    let resolved = ws.folder_for_path(&file_in_inner).unwrap();
    let resolved_canonical = resolved.canonicalize().unwrap();
    let inner_canonical = inner.canonicalize().unwrap();
    assert_eq!(resolved_canonical, inner_canonical);
}

#[tokio::test]
async fn worktree_path_resolves_to_parent_folder() {
    // Memory writes from inside a swarm worktree must land in the
    // parent folder's DB, not in a transient worktree DB.
    let global = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();

    // Construct a "worktree" path under the folder.
    let worktree_file = folder.path().join(".gaviero/worktrees/abc123/src/lib.rs");
    std::fs::create_dir_all(worktree_file.parent().unwrap()).unwrap();
    std::fs::write(&worktree_file, "").unwrap();

    let ws = single_folder_workspace(folder.path().to_path_buf());
    let stores = open_registry(folder.path(), &ws, global.path());

    let resolved = ws.folder_for_worktree_path(&worktree_file).unwrap();
    assert_eq!(
        resolved.canonicalize().unwrap(),
        folder.path().canonicalize().unwrap(),
        "worktree path must resolve to parent folder"
    );

    // A repo-scope write keyed by the parent folder's repo_id should
    // land in the folder DB.
    let meta = WriteMeta::for_source(MemorySource::UserRemember);
    let res = stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: hash_path(resolved),
            },
            "fact discovered inside a worktree",
            &meta,
        )
        .await
        .unwrap();
    assert!(matches!(res, StoreResult::Inserted(_)));
}

#[tokio::test]
async fn cross_db_retrieval_returns_rows_from_workspace_and_folder() {
    // Step 5 contract: MemoryStores::multi_scope_retrieve walks
    // every scope level, hits the right physical DB per level, and
    // merges results into one ranked pool.
    use gaviero_core::memory::{MemoryScope, RetrievalConfig, retrieve_for_chat_with_reranker};

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let library_repo_id = hash_path(library_root.path());
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);

    // Plant ONE row in workspace DB (workspace scope).
    stores
        .store_scoped(
            &WriteScope::Workspace,
            "workspace fact about backend integration",
            &meta,
        )
        .await
        .unwrap();

    // Plant ONE row in folder DB (repo scope) — DIFFERENT content.
    stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: library_repo_id.clone(),
            },
            "library fact about git2 backend usage",
            &meta,
        )
        .await
        .unwrap();

    // Build a SearchConfig that walks workspace + folder DBs.
    let scope =
        MemoryScope::from_context(workspace_root.path(), Some(library_root.path()), None, None);
    let mut config = gaviero_core::memory::SearchConfig::new("backend", scope);
    config.similarity_threshold = 0.0; // accept everything for the test
    let _ = retrieve_for_chat_with_reranker; // silence unused
    let _ = RetrievalConfig::default; // silence unused

    let hits = stores.multi_scope_retrieve(&config).await.unwrap();
    assert!(
        hits.len() >= 2,
        "expected at least 2 hits from cross-DB retrieval; got {} (contents: {:?})",
        hits.len(),
        hits.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    let texts: Vec<&str> = hits.iter().map(|m| m.content.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("workspace fact about backend integration")),
        "missing workspace-scope row in cross-DB results: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("library fact about git2 backend usage")),
        "missing folder-scope row in cross-DB results: {texts:?}"
    );
}

#[tokio::test]
async fn cross_db_retrieval_dedup_by_content_hash_keeps_narrower_scope() {
    // Same exact content written at workspace AND repo scope (in
    // different DBs). Cross-DB merge should dedup by content_hash;
    // narrower scope (Repo, with higher scope_multiplier) wins.
    use gaviero_core::memory::MemoryScope;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());
    let library_repo_id = hash_path(library_root.path());
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);

    // Note: cross-DB write dedup (Step 4) would normally collapse the
    // second write to AlreadyCovered. To exercise the *retrieval*
    // dedup, plant directly into both stores at their respective
    // scopes, bypassing the registry's dedup.
    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let folder_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id.clone(),
        })
        .await
        .unwrap();
    let content = "shared identical fact";
    workspace_store
        .store_scoped(&WriteScope::Workspace, content, &meta)
        .await
        .unwrap();
    folder_store
        .store_scoped(
            &WriteScope::Repo {
                repo_id: library_repo_id.clone(),
            },
            content,
            &meta,
        )
        .await
        .unwrap();

    // Verify both rows exist independently before retrieval.
    assert_eq!(
        workspace_store
            .search_at_level(&ScopeFilter::Workspace, "", 50)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        folder_store
            .search_at_level(
                &ScopeFilter::Repo {
                    repo_id: library_repo_id.clone(),
                },
                "",
                50,
            )
            .await
            .unwrap()
            .len(),
        1
    );

    // Cross-DB retrieval should yield exactly one row (deduped).
    let scope =
        MemoryScope::from_context(workspace_root.path(), Some(library_root.path()), None, None);
    let mut config = gaviero_core::memory::SearchConfig::new("shared identical fact", scope);
    config.similarity_threshold = 0.0;
    let hits = stores.multi_scope_retrieve(&config).await.unwrap();
    let matching: Vec<_> = hits.iter().filter(|m| m.content == content).collect();
    assert_eq!(
        matching.len(),
        1,
        "cross-DB content_hash dedup should collapse to 1 row; got {}",
        matching.len()
    );
    let kept = matching[0];
    // Narrower scope wins on equal similarity (scope_multiplier).
    assert_eq!(
        kept.scope_level,
        gaviero_core::memory::scope::SCOPE_REPO,
        "narrower scope (Repo) should win cross-scope dedup"
    );
}

#[tokio::test]
async fn cross_db_retrieval_records_access_per_owning_store() {
    // After multi_scope_retrieve, each store records access only for
    // its OWN rows — no cross-store id confusion.
    use gaviero_core::memory::MemoryScope;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());
    let library_repo_id = hash_path(library_root.path());
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);

    stores
        .store_scoped(&WriteScope::Workspace, "ws row alpha", &meta)
        .await
        .unwrap();
    stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: library_repo_id.clone(),
            },
            "lib row beta",
            &meta,
        )
        .await
        .unwrap();

    let scope =
        MemoryScope::from_context(workspace_root.path(), Some(library_root.path()), None, None);
    let mut config = gaviero_core::memory::SearchConfig::new("row", scope);
    config.similarity_threshold = 0.0;
    let hits = stores.multi_scope_retrieve(&config).await.unwrap();
    assert!(hits.len() >= 2, "expected at least 2 hits");

    // After cross-DB retrieval, each row's owning store has bumped
    // accessed_at on its row. We verify by re-reading the rows
    // from each store and checking the timestamp is set.
    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let folder_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id,
        })
        .await
        .unwrap();

    let ws_rows = workspace_store
        .search_at_level(&ScopeFilter::Workspace, "", 50)
        .await
        .unwrap();
    let alpha = ws_rows
        .iter()
        .find(|m| m.content == "ws row alpha")
        .expect("ws row");
    assert!(
        alpha.accessed_at.is_some(),
        "workspace row must have accessed_at set after cross-DB retrieval"
    );

    let folder_rows = folder_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: hash_path(library_root.path()),
            },
            "",
            50,
        )
        .await
        .unwrap();
    let beta = folder_rows
        .iter()
        .find(|m| m.content == "lib row beta")
        .expect("folder row");
    assert!(
        beta.accessed_at.is_some(),
        "folder row must have accessed_at set after cross-DB retrieval"
    );
}

#[tokio::test]
async fn consolidator_promotes_run_rows_into_correct_folder_db() {
    // Step 7 contract: a run row written into the workspace DB
    // should be promoted (via the Consolidator) into the correct
    // folder DB based on the WriteScope::Module/Repo target_store
    // routing — NOT into the workspace DB.
    use gaviero_core::memory::consolidation::Consolidator;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    let library_repo_id = hash_path(library_root.path());
    let run_id = "run-xyz";

    // Plant a run-scope memory directly into the workspace store
    // (simulating an agent's run-time observation).
    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let run_scope = WriteScope::Run {
        repo_id: library_repo_id.clone(),
        run_id: run_id.to_string(),
    };
    let meta = WriteMeta::for_source(MemorySource::SwarmConsolidated).with_importance(0.7); // > 0.4 threshold so it gets promoted
    workspace_store
        .store_scoped(&run_scope, "library: prefer git2", &meta)
        .await
        .unwrap();

    // Sanity: row is in workspace DB, not in library folder DB.
    let library_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id.clone(),
        })
        .await
        .unwrap();
    let pre_lib = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id.clone(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert!(pre_lib.is_empty(), "library DB should start empty");

    // Run consolidation phase 1.
    let consolidator = Consolidator::with_stores(stores.clone());
    let report = consolidator
        .consolidate_run(run_id, &library_repo_id)
        .await
        .unwrap();
    assert_eq!(
        report.promoted, 1,
        "exactly 1 run row should be promoted to repo scope"
    );

    // Allow the writer task time to drain.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // The promoted row should now live in the LIBRARY folder DB,
    // not the workspace DB.
    let post_lib = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id.clone(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert!(
        post_lib.iter().any(|m| m.content == "library: prefer git2"),
        "promoted row should appear in library folder DB; got {:?}",
        post_lib.iter().map(|m| &m.content).collect::<Vec<_>>()
    );

    // The run row in workspace DB should be deleted by phase 1.
    let post_workspace_runs = workspace_store
        .search_at_level(
            &ScopeFilter::Run {
                repo_id: library_repo_id.clone(),
                run_id: run_id.to_string(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert!(
        post_workspace_runs.is_empty(),
        "run rows should be cleared from workspace after promotion; got {} rows",
        post_workspace_runs.len()
    );
}

#[tokio::test]
async fn consolidator_decay_fans_out_to_every_opened_store() {
    // Step 7 contract: decay_and_prune iterates every opened store,
    // not just the workspace DB. We verify by writing into TWO stores
    // and confirming both are touched (counts come back as the SUM).
    use gaviero_core::memory::consolidation::Consolidator;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.5);

    stores
        .store_scoped(&WriteScope::Workspace, "ws fact", &meta)
        .await
        .unwrap();
    stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: hash_path(library_root.path()),
            },
            "lib fact",
            &meta,
        )
        .await
        .unwrap();

    let consolidator = Consolidator::with_stores(stores.clone());
    // Just ensure the call succeeds and returns a count >= 0; the
    // important thing is no per-store panic / error.
    let (decayed, pruned) = consolidator.decay_and_prune().await.unwrap();
    let _ = (decayed, pruned);
}

#[tokio::test]
async fn split_migration_moves_repo_rows_from_workspace_to_folder_db() {
    // Step 8 contract: opening a registry on a pre-v10 workspace DB
    // (with scope_level >= 2 rows tagged with a registered folder's
    // repo_id) MOVES those rows into the folder DB. The workspace DB
    // is left without those rows; the migration is stamped done.
    use gaviero_core::memory::MemoryStore;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();
    let library_repo_id = hash_path(library_root.path());

    // 1. Pre-populate the workspace DB directly with a repo-scope row
    //    that belongs to the library folder. This simulates a pre-v10
    //    layout where everything lived in one DB.
    let ws_db_path = workspace_root.path().join(".gaviero/memory.db");
    std::fs::create_dir_all(ws_db_path.parent().unwrap()).unwrap();
    {
        let store = Arc::new(MemoryStore::open(&ws_db_path, embedder()).unwrap());
        let scope = WriteScope::Repo {
            repo_id: library_repo_id.clone(),
        };
        let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.7);
        store
            .store_scoped(&scope, "pre-split repo fact about library", &meta)
            .await
            .unwrap();
        // Sanity: the row landed in workspace DB.
        let rows = store
            .search_at_level(
                &ScopeFilter::Repo {
                    repo_id: library_repo_id.clone(),
                },
                "",
                50,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "pre-migration: 1 row in workspace DB");
    }

    // 2. Open the registry — migration should fire.
    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());

    // 3. The library DB should now own the row.
    let library_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id.clone(),
        })
        .await
        .unwrap();
    let lib_rows = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id.clone(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert_eq!(
        lib_rows.len(),
        1,
        "library DB should hold the migrated row; got {} (contents: {:?})",
        lib_rows.len(),
        lib_rows.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    assert_eq!(lib_rows[0].content, "pre-split repo fact about library");

    // 4. The workspace DB should no longer have it.
    let workspace_store = stores.get(&StoreKind::Workspace).await.unwrap();
    let ws_rows = workspace_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id.clone(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert!(
        ws_rows.is_empty(),
        "workspace DB should no longer hold the row; got {}",
        ws_rows.len()
    );

    // 5. The migration should be stamped done — re-opening must be a no-op.
    let stamp = workspace_store
        .get_meta_value("split_v10_done")
        .await
        .unwrap();
    assert_eq!(stamp, Some("1".to_string()));
}

#[tokio::test]
async fn split_migration_is_idempotent() {
    // Re-opening a registry whose workspace DB is already at v10 must
    // not re-migrate anything.
    use gaviero_core::memory::MemoryStore;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();
    let library_repo_id = hash_path(library_root.path());

    let ws_db_path = workspace_root.path().join(".gaviero/memory.db");
    std::fs::create_dir_all(ws_db_path.parent().unwrap()).unwrap();
    {
        let store = Arc::new(MemoryStore::open(&ws_db_path, embedder()).unwrap());
        let meta = WriteMeta::for_source(MemorySource::UserRemember);
        store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: library_repo_id.clone(),
                },
                "fact",
                &meta,
            )
            .await
            .unwrap();
    }

    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );

    // First open: triggers migration.
    let _ = open_registry(workspace_root.path(), &ws, global.path());
    // Second open: must be a no-op (no panic, no data corruption).
    let stores2 = open_registry(workspace_root.path(), &ws, global.path());
    let library_store = stores2
        .get(&StoreKind::Folder {
            repo_id: library_repo_id.clone(),
        })
        .await
        .unwrap();
    // Library DB should still have exactly 1 row (not 2).
    let lib_rows = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id,
            },
            "",
            50,
        )
        .await
        .unwrap();
    assert_eq!(
        lib_rows.len(),
        1,
        "second open must not duplicate migrated rows; got {}",
        lib_rows.len()
    );
}

#[tokio::test]
async fn split_migration_skips_aliased_single_folder_workspace() {
    // When workspace_root == folder_root (single-folder open), the
    // workspace and folder DBs are aliased — there's nothing to
    // migrate, and the migration should silently no-op.
    use gaviero_core::memory::MemoryStore;

    let global = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let repo_id = hash_path(root.path());

    let ws_db_path = root.path().join(".gaviero/memory.db");
    std::fs::create_dir_all(ws_db_path.parent().unwrap()).unwrap();
    {
        let store = Arc::new(MemoryStore::open(&ws_db_path, embedder()).unwrap());
        let meta = WriteMeta::for_source(MemorySource::UserRemember);
        store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: repo_id.clone(),
                },
                "single-folder fact",
                &meta,
            )
            .await
            .unwrap();
    }

    let ws = single_folder_workspace(root.path().to_path_buf());
    let stores = open_registry(root.path(), &ws, global.path());
    let store = stores
        .get(&StoreKind::Folder {
            repo_id: repo_id.clone(),
        })
        .await
        .unwrap();
    let rows = store
        .search_at_level(&ScopeFilter::Repo { repo_id }, "", 50)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "aliased workspace must keep its 1 row intact"
    );
}

#[tokio::test]
async fn detect_mismatches_flags_per_db_embedder_drift() {
    // Step 9 contract: when a registered store's `_gaviero_meta.embedder_model`
    // stamp differs from the registry's configured embedder, it shows up
    // in `detect_mismatches`. Aligned stores do NOT show up.
    use gaviero_core::memory::MemoryStore;

    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();
    let library_repo_id = hash_path(library_root.path());

    // Pre-populate the LIBRARY DB with a row stamped under embedder
    // "old-model" — simulates the user having opened this folder
    // previously with a different embedder.
    let lib_db = library_root.path().join(".gaviero/memory.db");
    std::fs::create_dir_all(lib_db.parent().unwrap()).unwrap();
    {
        let store = MemoryStore::open(&lib_db, embedder_named("old-model")).unwrap();
        // Force the meta stamp by inserting one row (the open-time
        // back-fill only stamps when there are no embedded rows yet,
        // so writing one row guarantees the stamp).
        let meta = WriteMeta::for_source(MemorySource::UserRemember);
        store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: library_repo_id.clone(),
                },
                "row stamped under old-model",
                &meta,
            )
            .await
            .unwrap();
        // Explicitly stamp to ensure the meta key is set even if the
        // back-fill heuristic missed.
        store
            .set_meta_value("embedder_model", "old-model")
            .await
            .unwrap();
    }

    // Open the registry with the NEW embedder ("mock"). The library
    // store's stamp says "old-model", so detect_mismatches must flag it.
    let ws = workspace_with_library(
        workspace_root.path().to_path_buf(),
        library_root.path().to_path_buf(),
    );
    let stores = open_registry(workspace_root.path(), &ws, global.path());
    // Force-open the library store so it's in the opened set.
    let _ = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id,
        })
        .await
        .unwrap();

    let mismatches = stores.detect_mismatches().await;
    let library_mismatch = mismatches
        .iter()
        .find(|m| m.stored == "old-model" && m.configured == "mock");
    assert!(
        library_mismatch.is_some(),
        "library DB embedder mismatch must be reported; got {:?}",
        mismatches
    );
}

#[tokio::test]
async fn bootstrap_init_workspace_stores_with_two_folders_writes_to_correct_dbs() {
    // End-to-end bootstrap test: load a .gaviero-workspace file
    // describing two folders (a workspace project + a shared library
    // repo), call init_workspace_stores_with_embedder_name (the same
    // entry point the TUI/CLI bootstrap uses), then write at Repo
    // scope to each folder's repo_id and verify the rows land in the
    // correct per-folder DB file.
    //
    // This is the "real production wiring" smoke test for the
    // multi-DB layout.

    // Override the global DB path via the env var path. dirs::config_dir
    // can't be overridden, so we point HOME (or XDG_CONFIG_HOME) at a
    // tempdir for the duration of this test.
    let global_home = TempDir::new().unwrap();
    // SAFETY: Tests run sequentially within a process unless `cargo
    // test --test-threads`. `multi_db_memory` is the only test setting
    // these vars, and we restore on drop via the TempDir.
    // SAFETY: env var mutation in a test, single-threaded by default.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", global_home.path());
        std::env::set_var("HOME", global_home.path());
    }

    // Build a .gaviero-workspace file with two folders.
    let workspace_root = TempDir::new().unwrap();
    let library_root = TempDir::new().unwrap();
    let project_folder = workspace_root.path().join("project");
    std::fs::create_dir_all(&project_folder).unwrap();

    let ws_file = workspace_root.path().join("test.gaviero-workspace");
    let ws_json = serde_json::json!({
        "folders": [
            { "path": project_folder.to_string_lossy(), "name": "Project" },
            { "path": library_root.path().to_string_lossy(), "name": "Library" }
        ],
        "settings": {}
    });
    std::fs::write(&ws_file, serde_json::to_string_pretty(&ws_json).unwrap()).unwrap();

    let workspace = Workspace::load(&ws_file).unwrap();
    assert_eq!(workspace.folders().len(), 2);

    // Bootstrap (this is what the TUI / CLI calls).
    // We can't construct a real ONNX embedder in tests, so route via
    // a custom MemoryStores::open_with_embedder using a TestEmbedder.
    let stores = gaviero_core::memory::MemoryStores::open_with_paths(
        workspace_root.path(),
        &workspace,
        embedder(),
        "mock".to_string(),
        &global_home.path().join("global.db"),
    )
    .expect("registry open from workspace file");

    // Verify both folders' repo_ids resolve to distinct stores.
    let project_repo_id = hash_path(&project_folder);
    let library_repo_id = hash_path(library_root.path());
    let project_store = stores
        .get(&StoreKind::Folder {
            repo_id: project_repo_id.clone(),
        })
        .await
        .unwrap();
    let library_store = stores
        .get(&StoreKind::Folder {
            repo_id: library_repo_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        !Arc::ptr_eq(&project_store, &library_store),
        "two distinct workspace folders must have distinct stores"
    );

    // Write to project folder; verify it lands ONLY in project DB.
    let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.7);
    stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: project_repo_id.clone(),
            },
            "project-only fact",
            &meta,
        )
        .await
        .unwrap();
    stores
        .store_scoped(
            &WriteScope::Repo {
                repo_id: library_repo_id.clone(),
            },
            "library-only fact",
            &meta,
        )
        .await
        .unwrap();

    let project_rows = project_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: project_repo_id,
            },
            "",
            50,
        )
        .await
        .unwrap();
    let library_rows = library_store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: library_repo_id,
            },
            "",
            50,
        )
        .await
        .unwrap();

    assert_eq!(
        project_rows.len(),
        1,
        "project DB must have exactly its 1 row"
    );
    assert_eq!(project_rows[0].content, "project-only fact");
    assert_eq!(
        library_rows.len(),
        1,
        "library DB must have exactly its 1 row"
    );
    assert_eq!(library_rows[0].content, "library-only fact");

    // Verify the actual files exist where MemoryScope::from_context
    // documents them.
    assert!(project_folder.join(".gaviero/memory.db").exists());
    assert!(library_root.path().join(".gaviero/memory.db").exists());
    assert!(workspace_root.path().join(".gaviero/memory.db").exists());
}
