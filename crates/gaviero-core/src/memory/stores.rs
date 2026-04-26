//! Multi-DB memory registry.
//!
//! Owns the three physical SQLite stores (global / workspace / per-folder)
//! and dispatches operations by [`StoreKind`]. Pre-registers folder paths
//! from the [`Workspace`] at open time; folder stores are lazy-opened on
//! first access. When `workspace_root == folder_root` (single-folder
//! workspaces), the corresponding folder slot aliases to the workspace
//! store to preserve the collapsed-DB invariant from
//! [`MemoryScope::from_context`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

use super::embedder::Embedder;
use super::schema;
use super::scope::{
    StoreKind, StoreResult, WriteMeta, WriteScope, hash_path, store_kind_for_scope,
};
use super::scoring::{ScoredMemory, SearchConfig};
use super::store::{MemoryStore, embedding_to_blob_pub};
use crate::workspace::Workspace;

/// One mismatched embedder stamp (per DB).
#[derive(Debug, Clone)]
pub struct EmbedderMismatch {
    pub db_path: Option<PathBuf>,
    pub stored: String,
    pub configured: String,
}

/// Registry of the three physical memory stores.
pub struct MemoryStores {
    embedder: Arc<dyn Embedder>,
    embedder_name: String,
    global: Arc<MemoryStore>,
    workspace: Arc<MemoryStore>,
    /// Canonical workspace folder path. Used to detect aliasing with a
    /// folder whose canonical path matches.
    workspace_path: Option<PathBuf>,
    /// `repo_id` → canonical folder root, populated eagerly from
    /// [`Workspace::folders`] at open time.
    folder_paths: HashMap<String, PathBuf>,
    /// `repo_id` → opened store. Lazy-populated on first `get`.
    folders: Mutex<HashMap<String, Arc<MemoryStore>>>,
    /// Single-store fallback mode: when `true`, any unknown `repo_id`
    /// in a `StoreKind::Folder` lookup returns the workspace store.
    /// Set by [`Self::from_single_store`] and [`Self::for_tests_in_memory`]
    /// so legacy single-DB call sites keep working without registering
    /// folders.
    single_store_fallback: bool,
}

impl std::fmt::Debug for MemoryStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStores")
            .field("embedder_name", &self.embedder_name)
            .field("workspace_path", &self.workspace_path)
            .field("folder_paths", &self.folder_paths)
            .finish()
    }
}

impl MemoryStores {
    /// Open the registry.
    ///
    /// Eagerly opens the global store at `~/.config/gaviero/memory.db`
    /// and the workspace store at `<workspace_root>/.gaviero/memory.db`.
    /// Pre-registers folder paths from `workspace.folders()`; folder
    /// stores are opened on first [`Self::get`] for that `repo_id`.
    pub fn open(
        workspace_root: &Path,
        workspace: &Workspace,
        embedder_name: &str,
    ) -> Result<Arc<Self>> {
        let embedder = super::build_embedder_by_name(embedder_name)
            .context("building embedder for memory registry")?;
        Self::open_with_embedder(
            workspace_root,
            workspace,
            embedder,
            embedder_name.to_string(),
        )
    }

    /// Variant that accepts an already-built embedder. Used by tests and
    /// by call sites that need to share the embedder with the rest of
    /// the process.
    pub fn open_with_embedder(
        workspace_root: &Path,
        workspace: &Workspace,
        embedder: Arc<dyn Embedder>,
        embedder_name: String,
    ) -> Result<Arc<Self>> {
        let global_path = super::global_db_path()?;
        Self::open_with_paths(
            workspace_root,
            workspace,
            embedder,
            embedder_name,
            &global_path,
        )
    }

    /// Most-explicit constructor: caller supplies the global DB path.
    /// Used by tests that want to point the global store at a tempdir
    /// instead of `~/.config/gaviero/memory.db`.
    ///
    /// Also runs the v10 split migration: any pre-v10 workspace DB
    /// holding `scope_level >= SCOPE_REPO` rows that belong to a
    /// registered folder will have those rows moved into the folder
    /// DB. Idempotent via `_gaviero_meta.split_v10_done`.
    pub fn open_with_paths(
        workspace_root: &Path,
        workspace: &Workspace,
        embedder: Arc<dyn Embedder>,
        embedder_name: String,
        global_path: &Path,
    ) -> Result<Arc<Self>> {
        if let Some(parent) = global_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let workspace_path = workspace_root.join(".gaviero/memory.db");
        if let Some(parent) = workspace_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let global = Arc::new(MemoryStore::open(global_path, embedder.clone())?);
        let workspace_store = Arc::new(MemoryStore::open(&workspace_path, embedder.clone())?);

        let workspace_canonical = canonicalize(workspace_root);
        let mut folder_paths = HashMap::new();
        for folder in workspace.folders() {
            let canonical = canonicalize(&folder.path);
            let repo_id = hash_path(&folder.path);
            folder_paths.insert(repo_id, canonical);
        }

        // v10 split migration: move qualifying rows out of the
        // workspace DB into each registered folder's DB. Aliased
        // folders (workspace_root == folder_root) are skipped.
        // Folder DBs are materialised on demand inside the migration
        // function to preserve lazy-open semantics for unaffected
        // folders.
        let _migration_report = run_split_migration_v10(
            &workspace_path,
            &folder_paths,
            &workspace_canonical,
            embedder.clone(),
        )
        .with_context(|| {
            format!(
                "running v10 split migration on workspace DB {}",
                workspace_path.display()
            )
        })?;

        Ok(Arc::new(Self {
            embedder,
            embedder_name,
            global,
            workspace: workspace_store,
            workspace_path: Some(workspace_canonical),
            folder_paths,
            folders: Mutex::new(HashMap::new()),
            single_store_fallback: false,
        }))
    }

    /// Build a registry whose three slots all alias to a single
    /// in-memory store. Used by tests that pre-date the multi-DB split.
    /// Unknown `repo_id` lookups also resolve to the same store
    /// (single-store fallback).
    pub fn for_tests_in_memory(embedder: Arc<dyn Embedder>) -> Result<Arc<Self>> {
        let store = Arc::new(MemoryStore::in_memory(embedder.clone())?);
        Ok(Arc::new(Self {
            embedder_name: embedder.model_id().to_string(),
            embedder,
            global: store.clone(),
            workspace: store.clone(),
            workspace_path: None,
            folder_paths: HashMap::new(),
            folders: Mutex::new(HashMap::new()),
            single_store_fallback: true,
        }))
    }

    /// Build a registry that wraps a single, already-opened store —
    /// behaves identically to the pre-registry single-store world.
    /// Used by transitional call sites and by [`super::init_workspace`]
    /// which still returns one store today. Unknown `repo_id` lookups
    /// resolve to the wrapped store (single-store fallback).
    pub fn from_single_store(store: Arc<MemoryStore>) -> Arc<Self> {
        let embedder = store.embedder().clone();
        Arc::new(Self {
            embedder_name: embedder.model_id().to_string(),
            embedder,
            global: store.clone(),
            workspace: store.clone(),
            workspace_path: None,
            folder_paths: HashMap::new(),
            folders: Mutex::new(HashMap::new()),
            single_store_fallback: true,
        })
    }

    /// The shared embedder.
    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    /// The configured embedder name (settings string, not the model id).
    pub fn embedder_name(&self) -> &str {
        &self.embedder_name
    }

    /// The global store. Always available.
    pub fn global(&self) -> &Arc<MemoryStore> {
        &self.global
    }

    /// The workspace store. Always available.
    pub fn workspace(&self) -> &Arc<MemoryStore> {
        &self.workspace
    }

    /// Resolve a [`StoreKind`] to its store, lazy-opening folder stores
    /// on first access.
    ///
    /// When the folder's canonical path equals the workspace root's
    /// canonical path, the workspace store is returned (single-folder
    /// aliasing — preserves the collapsed-DB invariant).
    ///
    /// Returns an error for `Folder { repo_id }` when the `repo_id` is
    /// not registered in the workspace's folder list (defensive — every
    /// `WriteScope::Repo` should originate from a known folder).
    pub async fn get(&self, kind: &StoreKind) -> Result<Arc<MemoryStore>> {
        match kind {
            StoreKind::Global => Ok(self.global.clone()),
            StoreKind::Workspace => Ok(self.workspace.clone()),
            StoreKind::Folder { repo_id } => self.get_folder(repo_id).await,
        }
    }

    async fn get_folder(&self, repo_id: &str) -> Result<Arc<MemoryStore>> {
        let folder_path = match self.folder_paths.get(repo_id).cloned() {
            Some(p) => p,
            None if self.single_store_fallback => return Ok(self.workspace.clone()),
            None => {
                return Err(anyhow::anyhow!(
                    "unknown repo_id `{repo_id}` — not registered in workspace"
                ));
            }
        };

        // Aliasing: folder canonical path matches workspace canonical path
        // ⇒ return the workspace store (preserves collapsed-DB invariant).
        if let Some(ws_canonical) = &self.workspace_path
            && &folder_path == ws_canonical
        {
            return Ok(self.workspace.clone());
        }

        let mut folders = self.folders.lock().await;
        if let Some(existing) = folders.get(repo_id) {
            return Ok(existing.clone());
        }
        let db_path = folder_path.join(".gaviero/memory.db");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let store = Arc::new(MemoryStore::open(&db_path, self.embedder.clone())?);
        folders.insert(repo_id.to_string(), store.clone());
        Ok(store)
    }

    /// Cross-DB-aware scoped write.
    ///
    /// 1. Computes the content hash and probes every ancestor scope
    ///    that lives in a *different* physical DB than the target.
    ///    Hit ⇒ [`StoreResult::AlreadyCovered`].
    /// 2. If no cross-DB coverage, delegates to the target store's
    ///    [`MemoryStore::store_scoped`], which performs the same-DB
    ///    dedup (content-hash + semantic-similarity at ≥ 0.95).
    ///
    /// **Trade-off**: semantic-similarity dedup at *broader* scopes
    /// is best-effort and only catches cases where the broader scope
    /// happens to live in the *same* DB as the target (the target's
    /// own broader-scope semantic check). Across DBs we only catch
    /// content-hash matches; near-paraphrases at a broader scope in a
    /// different DB will result in a duplicate row. The 0.95 cosine
    /// floor is strict — occasional misses cost an extra row, not
    /// correctness.
    pub async fn store_scoped(
        &self,
        scope: &WriteScope,
        content: &str,
        meta: &WriteMeta,
    ) -> Result<StoreResult> {
        let target_kind = scope.target_store();
        let hash = schema::content_hash(content);
        for (level, path, ancestor_kind) in scope.ancestors() {
            if ancestor_kind == target_kind {
                // Same physical DB as the target — the target's own
                // store_scoped will catch this in its broader-scope
                // probe. Skip to avoid double-locking the same store.
                continue;
            }
            let ancestor_store = self.get(&ancestor_kind).await?;
            if ancestor_store.has_content_hash(level, &path, &hash).await? {
                tracing::debug!(
                    target: "memory_writer",
                    %path,
                    target_kind = ?target_kind,
                    ancestor_kind = ?ancestor_kind,
                    "cross-DB store_scoped: already covered at broader scope"
                );
                return Ok(StoreResult::AlreadyCovered);
            }
        }
        let target = self.get(&target_kind).await?;
        target.store_scoped(scope, content, meta).await
    }

    /// Cross-DB-aware multi-scope retrieval (Step 5).
    ///
    /// Embeds the query once, then walks `config.scope.levels()` —
    /// for each level, looks up the owning store via
    /// [`super::scope::ScopeFilter::target_store`] and runs the
    /// per-level vec + FTS search on that store. Candidates are merged
    /// into a single pool with cross-scope dedup-by-content_hash
    /// (narrower scope wins on ties), then sorted and truncated.
    ///
    /// After the merge, access-logging (`last_accessed_at` +
    /// `retrieval_use`) is dispatched per-store using the row's
    /// scope_level / repo_id to recover its owning store.
    pub async fn multi_scope_retrieve(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .await
            .context("computing query embedding for multi-scope retrieve")?;
        let query_blob = embedding_to_blob_pub(&query_embedding);

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut by_hash: HashMap<String, usize> = HashMap::new();

        for level in config.scope.levels() {
            let kind = level.target_store();
            let store = self.get(&kind).await?;
            let level_results = store
                .retrieve_at_level(&config.query, &query_blob, &level, config)
                .await?;

            for mem in level_results {
                // Cross-scope dedup: keep the higher-scoring entry on
                // a content_hash collision. With scope_multiplier, this
                // typically retains the narrower scope at equal sim.
                match by_hash.get(&mem.content_hash) {
                    Some(&idx) if accumulated[idx].final_score >= mem.final_score => {}
                    Some(&idx) => {
                        accumulated[idx] = mem;
                    }
                    None => {
                        by_hash.insert(mem.content_hash.clone(), accumulated.len());
                        accumulated.push(mem);
                    }
                }
            }
        }

        accumulated.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        accumulated.truncate(config.max_results);

        // Access logging — group ids by which store they live in so
        // each store touches only its own rows.
        let mut by_kind: HashMap<StoreKind, Vec<i64>> = HashMap::new();
        for mem in &accumulated {
            let repo_id_ref = mem.repo_id.as_deref();
            if let Some(kind) = store_kind_for_scope(mem.scope_level, repo_id_ref) {
                by_kind.entry(kind).or_default().push(mem.id);
            }
        }
        for (kind, ids) in by_kind {
            if let Ok(store) = self.get(&kind).await {
                let _ = store.record_access(&ids, &config.scope).await;
            }
        }

        Ok(accumulated)
    }

    /// Check the embedder stamp on every currently-opened store and
    /// return one [`EmbedderMismatch`] per DB whose stamp differs from
    /// the configured embedder.
    pub async fn detect_mismatches(&self) -> Vec<EmbedderMismatch> {
        let configured = self.embedder.model_id().to_string();
        let mut out = Vec::new();
        for store in self.opened_stores().await {
            if let Some(stored) = store.detect_embedder_mismatch().await {
                out.push(EmbedderMismatch {
                    db_path: store.db_path().map(|p| p.to_path_buf()),
                    stored,
                    configured: configured.clone(),
                });
            }
        }
        out
    }

    /// Snapshot of every store currently materialised (global +
    /// workspace + every lazily-opened folder). Used by maintenance
    /// passes (decay/prune, promotion analysis) that need to fan out
    /// across every physical DB.
    pub async fn opened_stores(&self) -> Vec<Arc<MemoryStore>> {
        let mut out = vec![self.global.clone(), self.workspace.clone()];
        let folders = self.folders.lock().await;
        for store in folders.values() {
            // Skip aliases: the workspace store may already be in `out`.
            if !out.iter().any(|s| Arc::ptr_eq(s, store)) {
                out.push(store.clone());
            }
        }
        out
    }

    /// Snapshot of every folder store currently materialised. Used by
    /// promotion analysis (module → repo) which only operates on
    /// folder DBs.
    pub async fn opened_folder_stores(&self) -> Vec<Arc<MemoryStore>> {
        let folders = self.folders.lock().await;
        folders.values().cloned().collect()
    }

    /// Eagerly open every registered folder store. Useful for
    /// maintenance passes that want a complete snapshot rather than
    /// just whatever was lazy-opened.
    pub async fn open_all_folders(&self) -> Result<()> {
        let repo_ids: Vec<String> = self.folder_paths.keys().cloned().collect();
        for repo_id in repo_ids {
            let _ = self.get_folder(&repo_id).await?;
        }
        Ok(())
    }
}

fn canonicalize(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Outcome of the v10 split migration.
#[derive(Debug, Default, Clone)]
pub struct SplitMigrationReport {
    /// Number of folder DBs that received migrated rows.
    pub folders_migrated: usize,
    /// Total rows moved from workspace DB into folder DBs.
    pub rows_migrated: usize,
    /// Whether the workspace DB was already at v10 (no-op).
    pub already_done: bool,
    /// Path of the workspace `.bak` file taken before migration, if any.
    pub backup_path: Option<PathBuf>,
}

/// Run the v10 split migration on `workspace_db` against the given
/// folder roots (keyed by `repo_id`). Idempotent: stamped via
/// `_gaviero_meta.split_v10_done = '1'` on the workspace DB.
///
/// For each folder whose canonical path differs from
/// `workspace_canonical`, moves rows where `scope_level >= SCOPE_REPO`
/// AND `repo_id` matches the folder's `repo_id`, plus the
/// corresponding `vec_memories_scoped` and legacy `vec_memories`
/// rows. Source rows are deleted from the workspace DB after a
/// successful copy.
///
/// **Safety**: takes a `.bak-v10` copy of the workspace DB before
/// running. Cross-DB copy is row-by-row (not ATTACH) to avoid
/// virtual-table interactions across attached schemas.
///
/// **Lazy folder DBs**: folder DBs are only materialised when the
/// workspace DB actually contains rows for that folder's `repo_id`.
/// Unaffected folders stay un-opened.
fn run_split_migration_v10(
    workspace_db: &Path,
    folder_roots: &HashMap<String, PathBuf>,
    workspace_canonical: &Path,
    embedder: Arc<dyn Embedder>,
) -> Result<SplitMigrationReport> {
    if !workspace_db.exists() {
        return Ok(SplitMigrationReport {
            already_done: true,
            ..Default::default()
        });
    }
    let probe =
        Connection::open(workspace_db).context("opening workspace DB for migration probe")?;
    let already_done: bool = probe
        .query_row(
            "SELECT value FROM _gaviero_meta WHERE key = 'split_v10_done'",
            [],
            |r| r.get::<_, String>(0).map(|v| v == "1"),
        )
        .unwrap_or(false);
    drop(probe);
    if already_done {
        return Ok(SplitMigrationReport {
            already_done: true,
            ..Default::default()
        });
    }

    let mut report = SplitMigrationReport::default();

    // Take a backup before mutating.
    let backup = workspace_db.with_extension("db.bak-v10");
    if std::fs::copy(workspace_db, &backup).is_ok() {
        report.backup_path = Some(backup);
    }

    let workspace_canonical_buf = workspace_canonical.to_path_buf();

    for (repo_id, folder_root) in folder_roots {
        // Skip aliased folders (folder root == workspace root).
        let folder_canonical = canonicalize(folder_root);
        if folder_canonical == workspace_canonical_buf {
            continue;
        }
        // Skip if there are no rows for this repo_id — keeps the
        // folder DB un-opened (lazy semantics preserved).
        let probe2 = Connection::open(workspace_db).context("opening workspace DB to count")?;
        let row_count: i64 = probe2
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE scope_level >= ?1 AND repo_id = ?2",
                rusqlite::params![super::scope::SCOPE_REPO, repo_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        drop(probe2);
        if row_count == 0 {
            continue;
        }

        // Materialise the folder DB so its schema is in place before
        // the migration INSERTs — only happens when there's data to
        // migrate.
        let folder_db = folder_root.join(".gaviero/memory.db");
        if let Some(parent) = folder_db.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let _ = MemoryStore::open(&folder_db, embedder.clone())
            .with_context(|| format!("opening folder DB {} for migration", folder_db.display()))?;
        // Drop the MemoryStore — we re-open the connection inside
        // migrate_one_folder to avoid lock contention.
        let moved = migrate_one_folder(workspace_db, &folder_db, repo_id)
            .with_context(|| format!("migrating repo_id `{repo_id}` to folder DB"))?;
        if moved > 0 {
            report.folders_migrated += 1;
            report.rows_migrated += moved;
        }
    }

    // Stamp completion on workspace DB.
    let stamp = Connection::open(workspace_db).context("opening workspace DB for stamp")?;
    let _ = stamp.execute(
        "INSERT INTO _gaviero_meta(key, value) VALUES ('split_v10_done', '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    );

    Ok(report)
}

fn migrate_one_folder(workspace_db: &Path, folder_db: &Path, repo_id: &str) -> Result<usize> {
    use super::scope::SCOPE_REPO;

    // Open both connections fresh; load sqlite-vec extension via
    // MemoryStore's static initializer (already done at app start).
    let mut ws = Connection::open(workspace_db).context("open workspace DB")?;
    let folder = Connection::open(folder_db).context("open folder DB")?;

    // Collect rows to migrate.
    let rows: Vec<(
        i64,             // id
        String,          // namespace
        String,          // key
        String,          // content
        Option<Vec<u8>>, // embedding (may be NULL on legacy rows)
        Option<String>,  // model_id
        i32,             // scope_level
        String,          // scope_path
        Option<String>,  // module_path
        Option<String>,  // run_id
        String,          // content_hash
        String,          // memory_type
        String,          // trust
        Option<String>,  // tag
        f32,             // importance
        String,          // privacy
        String,          // source
        f32,             // trust_score
    )> = {
        let mut stmt = ws.prepare(
            "SELECT id, namespace, key, content, embedding, model_id,
                    scope_level, scope_path, module_path, run_id,
                    content_hash, memory_type, trust, tag,
                    importance, privacy, source, trust_score
             FROM memories
             WHERE scope_level >= ?1 AND repo_id = ?2",
        )?;
        let mut q = stmt.query(rusqlite::params![SCOPE_REPO, repo_id])?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get(14)?,
                r.get(15)?,
                r.get(16)?,
                r.get(17)?,
            ));
        }
        out
    };
    if rows.is_empty() {
        return Ok(0);
    }

    // Collect vec rows so we can re-insert them on the destination.
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    let vec_scoped_rows: Vec<(i64, Vec<u8>, i32)> = {
        let sql = format!(
            "SELECT memory_id, embedding, scope_level
             FROM vec_memories_scoped WHERE memory_id IN ({placeholders})"
        );
        let mut stmt = ws.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut q = stmt.query(rusqlite::params_from_iter(params.iter()))?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push((r.get(0)?, r.get(1)?, r.get(2)?));
        }
        out
    };

    let vec_legacy_rows: Vec<(i64, Vec<u8>)> = {
        let sql = format!(
            "SELECT memory_id, embedding FROM vec_memories WHERE memory_id IN ({placeholders})"
        );
        let mut stmt = ws.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut q = stmt.query(rusqlite::params_from_iter(params.iter()))?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }
        out
    };

    // Insert into folder DB.
    {
        let tx = folder.unchecked_transaction()?;
        for r in &rows {
            // Preserve content_hash + scope_path uniqueness via
            // INSERT OR IGNORE — if the folder DB was populated by
            // another workspace, collisions are expected.
            tx.execute(
                "INSERT OR IGNORE INTO memories (
                    namespace, key, content, embedding, model_id,
                    scope_level, scope_path, repo_id, module_path, run_id,
                    content_hash, memory_type, trust, tag,
                    importance, privacy, source, trust_score
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                          ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                rusqlite::params![
                    r.1, r.2, r.3, r.4, r.5, r.6, r.7, repo_id, r.8, r.9, r.10, r.11, r.12, r.13,
                    r.14, r.15, r.16, r.17,
                ],
            )?;
            // Map old id → new id via content_hash (which is unique
            // within (content_hash, scope_path)).
            let new_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM memories WHERE content_hash = ?1 AND scope_path = ?2",
                    rusqlite::params![r.10, r.7],
                    |row| row.get(0),
                )
                .ok();
            if let (Some(new_id), Some(emb)) = (new_id, r.4.as_ref()) {
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO vec_memories_scoped(memory_id, embedding, scope_level)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![new_id, emb, r.6],
                );
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
                    rusqlite::params![new_id, emb],
                );
            } else if let Some(new_id) = new_id {
                // Try the legacy vec rows we collected (they're keyed
                // by old id; only useful for migrating the embedding
                // when it wasn't on the row but lived in the vec table).
                if let Some((_, emb, _)) = vec_scoped_rows.iter().find(|(oid, _, _)| *oid == r.0) {
                    let _ = tx.execute(
                        "INSERT OR REPLACE INTO vec_memories_scoped(memory_id, embedding, scope_level)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![new_id, emb, r.6],
                    );
                }
                if let Some((_, emb)) = vec_legacy_rows.iter().find(|(oid, _)| *oid == r.0) {
                    let _ = tx.execute(
                        "INSERT OR REPLACE INTO vec_memories(memory_id, embedding)
                         VALUES (?1, ?2)",
                        rusqlite::params![new_id, emb],
                    );
                }
            }
        }
        tx.commit()?;
    }

    // Delete migrated rows from the workspace DB.
    {
        let tx = ws.transaction()?;
        // Clean vec tables first (FK-like dependency).
        let del_sql_scoped =
            format!("DELETE FROM vec_memories_scoped WHERE memory_id IN ({placeholders})");
        let del_sql_legacy =
            format!("DELETE FROM vec_memories WHERE memory_id IN ({placeholders})");
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let _ = tx.execute(&del_sql_scoped, rusqlite::params_from_iter(params.iter()));
        let _ = tx.execute(&del_sql_legacy, rusqlite::params_from_iter(params.iter()));
        tx.execute(
            "DELETE FROM memories WHERE scope_level >= ?1 AND repo_id = ?2",
            rusqlite::params![SCOPE_REPO, repo_id],
        )?;
        tx.commit()?;
    }

    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::scope::WriteScope;

    struct MockEmbedder;
    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        fn name(&self) -> &str {
            "mock"
        }

        fn dimension(&self) -> usize {
            8
        }

        async fn embed(
            &self,
            text: &str,
            _purpose: crate::memory::embedder::EmbeddingPurpose,
        ) -> Result<Vec<f32>> {
            let mut vec = vec![0.0f32; 8];
            for (i, byte) in text.bytes().enumerate() {
                vec[i % 8] += byte as f32;
            }
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }
        fn dimensions(&self) -> usize {
            8
        }
        fn model_id(&self) -> &str {
            "mock"
        }
    }

    fn test_embedder() -> Arc<dyn Embedder> {
        Arc::new(MockEmbedder) as Arc<dyn Embedder>
    }

    #[tokio::test]
    async fn for_tests_in_memory_aliases_all_three_slots() {
        let stores = MemoryStores::for_tests_in_memory(test_embedder()).unwrap();
        let g = stores.get(&StoreKind::Global).await.unwrap();
        let w = stores.get(&StoreKind::Workspace).await.unwrap();
        // Global and workspace should be the same Arc.
        assert!(Arc::ptr_eq(&g, &w));
    }

    #[tokio::test]
    async fn for_tests_in_memory_falls_back_to_workspace_for_unknown_repo() {
        // for_tests_in_memory enables single-store fallback: any
        // Folder { repo_id } resolves to the same in-memory store so
        // legacy single-DB tests that hand-craft scopes keep working.
        let stores = MemoryStores::for_tests_in_memory(test_embedder()).unwrap();
        let folder = stores
            .get(&StoreKind::Folder {
                repo_id: "deadbeef".into(),
            })
            .await
            .unwrap();
        let workspace = stores.get(&StoreKind::Workspace).await.unwrap();
        assert!(Arc::ptr_eq(&folder, &workspace));
    }

    #[tokio::test]
    async fn open_registry_errors_for_unregistered_folder() {
        // The strict (non-fallback) constructor used by production
        // bootstrap must reject unknown repo_ids — every WriteScope::Repo
        // should originate from a folder listed in the workspace.
        let store = Arc::new(MemoryStore::in_memory(test_embedder()).unwrap());
        let strict = Arc::new(MemoryStores {
            embedder_name: "mock".into(),
            embedder: test_embedder(),
            global: store.clone(),
            workspace: store.clone(),
            workspace_path: None,
            folder_paths: HashMap::new(),
            folders: Mutex::new(HashMap::new()),
            single_store_fallback: false,
        });
        let res = strict
            .get(&StoreKind::Folder {
                repo_id: "deadbeef".into(),
            })
            .await;
        assert!(res.is_err(), "unknown repo_id should error in strict mode");
    }

    #[tokio::test]
    async fn write_scope_target_store_dispatches() {
        let stores = MemoryStores::for_tests_in_memory(test_embedder()).unwrap();
        // The aliased test registry returns the same store regardless,
        // but we verify get() doesn't panic and the WriteScope's
        // target_store() is a valid input.
        let scopes = [
            WriteScope::Global,
            WriteScope::Workspace,
            WriteScope::Run {
                repo_id: "x".into(),
                run_id: "r".into(),
            },
        ];
        for ws in scopes {
            let kind = ws.target_store();
            // Run scope routes to Workspace store, not Folder — so it works
            // even though "x" isn't registered.
            assert!(stores.get(&kind).await.is_ok(), "kind {kind:?} failed");
        }
    }
}
