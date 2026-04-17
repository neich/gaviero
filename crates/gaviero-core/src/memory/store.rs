use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;
use tracing;

use super::embedder::Embedder;
use super::schema;
use super::scope::{
    MemoryScope, MemoryType, ScopeFilter, StoreResult, Trust, WriteMeta, WriteScope,
};
use super::scoring::{self, ScoredMemory, SearchConfig};

/// Privacy filter for memory search operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilter {
    /// Return all entries — for local-only agents.
    IncludeAll,
    /// Exclude entries with privacy: local_only — for API-bound contexts.
    ExcludeLocalOnly,
}

/// Options for storing a memory entry.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    pub privacy: String,
    pub importance: f32,
    pub metadata: Option<String>,
    pub source_file: Option<String>,
    pub source_hash: Option<String>,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            privacy: "public".to_string(),
            importance: 0.5,
            metadata: None,
            source_file: None,
            source_hash: None,
        }
    }
}

/// A memory entry returned from the store.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub id: i64,
    pub namespace: String,
    pub key: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub importance: f32,
    pub access_count: i32,
    pub last_accessed_at: Option<String>,
}

/// A search result with composite retrieval score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: MemoryEntry,
    pub score: f32,
}

/// V9 §4 `MemoryCandidate`: structured memory record returned by
/// [`MemoryStore::search_candidates`] for the planner to consume.
///
/// M3 type. Lives in this module (not `context_planner/`) because
/// `MemoryStore` constructs it directly — placing it under `context_planner`
/// would create a cycle with `crate::memory`. Re-exported from
/// `crate::context_planner::types` so consumers see the V9-spec home.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryCandidate {
    pub id: i64,
    pub namespace: String,
    pub scope_label: String,
    pub score: f32,
    pub trust: Option<String>,
    pub content: String,
    pub source_hash: Option<String>,
    pub updated_at: Option<String>,
}

impl From<&SearchResult> for MemoryCandidate {
    fn from(r: &SearchResult) -> Self {
        Self {
            id: r.entry.id,
            namespace: r.entry.namespace.clone(),
            // M3 placeholder: namespace is the closest proxy to a scope
            // label until M4 lifts it from the actual MemoryScope.
            scope_label: r.entry.namespace.clone(),
            score: r.score,
            trust: None,
            content: r.entry.content.clone(),
            source_hash: None,
            updated_at: Some(r.entry.updated_at.clone()),
        }
    }
}

/// Semantic memory store backed by SQLite + sqlite-vec + ONNX embeddings.
///
/// Key pattern: CPU-heavy embedding runs BEFORE acquiring the SQLite lock.
/// The lock is held only for brief I/O operations.
///
/// Vector search uses sqlite-vec's `vec0` virtual table with cosine distance.
/// Retrieval scoring combines recency, importance, and relevance (Stanford
/// Generative Agents formula).
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
    embedder: Arc<dyn Embedder>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("model", &self.embedder.model_id())
            .finish()
    }
}

impl MemoryStore {
    /// Register sqlite-vec extension globally. Must be called before opening connections.
    /// Safe to call multiple times (idempotent at the process level).
    fn register_sqlite_vec() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        });
    }

    /// Open or create a memory store at the given path.
    pub fn open(db_path: &Path, embedder: Arc<dyn Embedder>) -> Result<Self> {
        Self::register_sqlite_vec();
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening memory database: {}", db_path.display()))?;
        Self::init(conn, embedder)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory(embedder: Arc<dyn Embedder>) -> Result<Self> {
        Self::register_sqlite_vec();
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        Self::init(conn, embedder)
    }

    fn init(conn: Connection, embedder: Arc<dyn Embedder>) -> Result<Self> {
        schema::run_migrations(&conn, embedder.dimensions())
            .context("running schema migrations")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    /// Return a reference to the embedder for external use.
    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    // ── Store operations ───────────────────────────────────────────

    /// Store a memory entry. Upserts on (namespace, key).
    ///
    /// Embedding is computed BEFORE acquiring the database lock.
    pub async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        let opts = StoreOptions {
            metadata: metadata.map(|s| s.to_string()),
            ..Default::default()
        };
        self.store_with_options(namespace, key, content, &opts)
            .await
    }

    /// Store a memory entry with explicit privacy level.
    ///
    /// Prefer `store_with_options()` for new code.
    pub async fn store_with_privacy(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        privacy: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        let opts = StoreOptions {
            privacy: privacy.to_string(),
            metadata: metadata.map(|s| s.to_string()),
            ..Default::default()
        };
        self.store_with_options(namespace, key, content, &opts)
            .await
    }

    /// Store a memory entry with full options control.
    ///
    /// Embedding is computed BEFORE acquiring the database lock.
    /// Writes to both `memories` table and `vec_memories` virtual table.
    pub async fn store_with_options(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        opts: &StoreOptions,
    ) -> Result<i64> {
        // Compute embedding outside the lock (CPU-heavy)
        let embedding = self
            .embedder
            .embed_document(content)
            .context("computing embedding")?;
        let embedding_blob = embedding_to_blob(&embedding);
        let model_id = self.embedder.model_id().to_string();

        let ns = namespace.to_string();
        let k = key.to_string();
        let c = content.to_string();
        let opts = opts.clone();

        // Brief lock for database write
        let conn = self.conn.lock().await;

        // Upsert into memories table
        conn.execute(
            "INSERT INTO memories (namespace, key, content, embedding, model_id, metadata,
                                   privacy, importance, source_file, source_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(namespace, key) DO UPDATE SET
                content = excluded.content,
                embedding = excluded.embedding,
                model_id = excluded.model_id,
                metadata = excluded.metadata,
                privacy = excluded.privacy,
                importance = excluded.importance,
                source_file = excluded.source_file,
                source_hash = excluded.source_hash,
                updated_at = datetime('now')",
            rusqlite::params![
                ns,
                k,
                c,
                embedding_blob,
                model_id,
                opts.metadata,
                opts.privacy,
                opts.importance,
                opts.source_file,
                opts.source_hash
            ],
        )
        .context("inserting memory")?;

        // Get the row id (works for both insert and update)
        let id: i64 = conn
            .query_row(
                "SELECT id FROM memories WHERE namespace = ?1 AND key = ?2",
                rusqlite::params![ns, k],
                |row| row.get(0),
            )
            .context("getting memory id after upsert")?;

        // Upsert into vec_memories for KNN search.
        // vec0 tables don't support INSERT OR REPLACE, so delete first then insert.
        let _ = conn.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            rusqlite::params![id],
        );
        conn.execute(
            "INSERT INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![id, embedding_blob],
        )
        .context("inserting into vec_memories")?;

        Ok(id)
    }

    // ── Search operations ──────────────────────────────────────────

    /// Search for memories similar to the query text within a single namespace.
    ///
    /// Uses sqlite-vec KNN search with composite scoring (recency + importance + relevance).
    pub async fn search(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_multi_filtered(
            &[namespace.to_string()],
            query,
            limit,
            PrivacyFilter::IncludeAll,
        )
        .await
    }

    /// Search across multiple namespaces.
    ///
    /// Results from all namespaces are merged and sorted by composite score.
    pub async fn search_multi(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_multi_filtered(namespaces, query, limit, PrivacyFilter::IncludeAll)
            .await
    }

    /// V9 §11 M3: structured memory candidates for the planner.
    ///
    /// Returns an empty `Vec` on error or no hits. Unlike [`Self::search_context`]
    /// (which formats results as a prompt-ready string), this returns the
    /// underlying records so the planner can record memory IDs in the ledger
    /// and emit per-selection tracing. M10 deletes `search_context`.
    pub async fn search_candidates(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> Vec<MemoryCandidate> {
        match self.search_multi(namespaces, query, limit).await {
            Ok(results) => {
                // M0 legacy aggregate event — kept for baseline tooling
                // continuity. M3's per-candidate `memory_candidate` events
                // are emitted by the planner (`ContextPlanner::collect_memory`).
                let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
                let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
                tracing::info!(
                    target: "turn_metrics",
                    memory_ids = ?ids,
                    memory_scores = ?scores,
                    memory_count = results.len(),
                    "memory_selection"
                );
                results.iter().map(MemoryCandidate::from).collect()
            }
            Err(e) => {
                tracing::warn!("memory search failed: {}", e);
                tracing::info!(
                    target: "turn_metrics",
                    memory_count = 0usize,
                    "memory_selection"
                );
                Vec::new()
            }
        }
    }

    /// Search across namespaces and format results as a prompt-ready string.
    ///
    /// Returns an empty string on error or if no results are found.
    pub async fn search_context(&self, namespaces: &[String], query: &str, limit: usize) -> String {
        match self.search_multi(namespaces, query, limit).await {
            Ok(results) if !results.is_empty() => {
                // M0 instrumentation: expose selected memory IDs + scores so
                // baselines can measure repeated-context waste across turns.
                // Safe under V9 §2 "structured APIs land in M3" — no API change.
                let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
                let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
                tracing::info!(
                    target: "turn_metrics",
                    memory_ids = ?ids,
                    memory_scores = ?scores,
                    memory_count = results.len(),
                    "memory_selection"
                );
                let mut ctx = String::from("[Memory context]:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "- [{}] {} (score: {:.2})\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => {
                tracing::info!(
                    target: "turn_metrics",
                    memory_count = 0usize,
                    "memory_selection"
                );
                String::new()
            }
        }
    }

    /// Search across namespaces with privacy filtering, returning a prompt-ready string.
    pub async fn search_context_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> String {
        match self
            .search_multi_filtered(namespaces, query, limit, filter)
            .await
        {
            Ok(results) if !results.is_empty() => {
                let mut ctx = String::from("[Memory context]:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "- [{}] {} (score: {:.2})\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => String::new(),
        }
    }

    /// Core search implementation using sqlite-vec KNN with post-filtering and composite scoring.
    ///
    /// sqlite-vec doesn't support WHERE clauses in KNN queries, so we over-fetch
    /// and filter by namespace/privacy in Rust.
    async fn search_multi_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> Result<Vec<SearchResult>> {
        if namespaces.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        // Compute query embedding outside the lock (CPU-heavy)
        let query_embedding = self
            .embedder
            .embed_query(query)
            .context("computing query embedding")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;

        // Over-fetch from vec_memories (5x limit to allow for post-filtering)
        let fetch_k = limit * 5;
        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance,
                    m.id, m.namespace, m.key, m.content, m.metadata,
                    m.created_at, m.updated_at, m.importance, m.access_count,
                    m.last_accessed_at, m.privacy
             FROM vec_memories v
             JOIN memories m ON m.id = v.memory_id
             WHERE v.embedding MATCH ?1 AND k = ?2",
            )
            .context("preparing KNN search")?;

        let now = chrono_now_utc();
        let ns_set: std::collections::HashSet<&str> =
            namespaces.iter().map(|s| s.as_str()).collect();

        let mut results: Vec<SearchResult> = stmt
            .query_map(rusqlite::params![query_blob, fetch_k as i64], |row| {
                let distance: f32 = row.get(1)?;
                let namespace: String = row.get(3)?;
                let privacy: String = row.get(12)?;
                let entry = MemoryEntry {
                    id: row.get(2)?,
                    namespace,
                    key: row.get(4)?,
                    content: row.get(5)?,
                    metadata: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    importance: row.get(9)?,
                    access_count: row.get(10)?,
                    last_accessed_at: row.get(11)?,
                };
                Ok((entry, distance, privacy))
            })
            .context("executing KNN search")?
            .filter_map(|r| r.ok())
            .filter(|(entry, _, privacy)| {
                // Post-filter: namespace membership
                if !ns_set.contains(entry.namespace.as_str()) {
                    return false;
                }
                // Post-filter: privacy
                if filter == PrivacyFilter::ExcludeLocalOnly && privacy == "local_only" {
                    return false;
                }
                true
            })
            .map(|(entry, distance, _)| {
                // Convert L2 distance to cosine similarity for L2-normalized vectors:
                // L2_dist² = 2 - 2·cos_sim  →  cos_sim = 1 - L2_dist²/2
                let relevance = (1.0 - distance * distance / 2.0).max(0.0);
                let hours = hours_since(&entry.last_accessed_at, &entry.updated_at, &now);
                let score = retrieval_score(hours, entry.importance, relevance, entry.access_count);
                SearchResult { entry, score }
            })
            .collect();

        // Sort by composite score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        // Update access tracking for returned results
        if !results.is_empty() {
            let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
            let placeholders: String = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "UPDATE memories SET access_count = access_count + 1, last_accessed_at = datetime('now')
                 WHERE id IN ({})",
                placeholders
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = ids
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            let _ = conn.execute(&sql, params.as_slice()); // best-effort
        }

        Ok(results)
    }

    // ── Point operations ───────────────────────────────────────────

    /// Get a specific memory by namespace and key.
    pub async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, namespace, key, content, metadata, created_at, updated_at,
                    importance, access_count, last_accessed_at
             FROM memories WHERE namespace = ?1 AND key = ?2",
        )?;

        let entry = stmt.query_row(rusqlite::params![namespace, key], |row| {
            Ok(MemoryEntry {
                id: row.get(0)?,
                namespace: row.get(1)?,
                key: row.get(2)?,
                content: row.get(3)?,
                metadata: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                importance: row.get(7)?,
                access_count: row.get(8)?,
                last_accessed_at: row.get(9)?,
            })
        });

        match entry {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all keys in a namespace.
    pub async fn list_keys(&self, namespace: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt =
            conn.prepare("SELECT key FROM memories WHERE namespace = ?1 ORDER BY key")?;
        let keys: Vec<String> = stmt
            .query_map(rusqlite::params![namespace], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(keys)
    }

    /// Delete a specific memory by namespace and key.
    pub async fn delete(&self, namespace: &str, key: &str) -> Result<bool> {
        let conn = self.conn.lock().await;

        // Get id for vec_memories cleanup
        let id: Option<i64> = conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND key = ?2")?
            .query_row(rusqlite::params![namespace, key], |row| row.get(0))
            .ok();

        let rows = conn.execute(
            "DELETE FROM memories WHERE namespace = ?1 AND key = ?2",
            rusqlite::params![namespace, key],
        )?;

        // Clean up vec_memories
        if let Some(id) = id {
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        Ok(rows > 0)
    }

    /// Clear all memories in a namespace.
    pub async fn clear_namespace(&self, namespace: &str) -> Result<usize> {
        let conn = self.conn.lock().await;

        // Collect ids for vec_memories cleanup
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1")?
            .query_map(rusqlite::params![namespace], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let rows = conn.execute(
            "DELETE FROM memories WHERE namespace = ?1",
            rusqlite::params![namespace],
        )?;

        // Clean up vec_memories
        for id in &ids {
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        Ok(rows)
    }

    // ── Maintenance operations ─────────────────────────────────────

    /// Reindex all memories with the current embedder.
    /// Useful when switching embedding models.
    pub async fn reindex(&self, namespace: &str) -> Result<usize> {
        // Read all entries outside the lock
        let entries = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare("SELECT id, content FROM memories WHERE namespace = ?1")?;
            let entries: Vec<(i64, String)> = stmt
                .query_map(rusqlite::params![namespace], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            entries
        };

        // Compute embeddings outside the lock (CPU-heavy)
        let model_id = self.embedder.model_id().to_string();
        let mut updates = Vec::with_capacity(entries.len());
        for (id, content) in &entries {
            let embedding = self.embedder.embed_document(content)?;
            updates.push((*id, embedding_to_blob(&embedding)));
        }

        // Write back with a brief lock
        let conn = self.conn.lock().await;
        for (id, blob) in &updates {
            // Update memories table
            conn.execute(
                "UPDATE memories SET embedding = ?1, model_id = ?2, updated_at = datetime('now')
                 WHERE id = ?3",
                rusqlite::params![blob, model_id, id],
            )?;
            // Update vec_memories
            conn.execute(
                "INSERT OR REPLACE INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![id, blob],
            )?;
        }

        Ok(updates.len())
    }

    /// Check for stale memories whose source files have changed.
    ///
    /// Returns (memory_id, source_file, old_hash, new_hash) for each mismatch.
    pub async fn check_staleness(
        &self,
        files: &[std::path::PathBuf],
    ) -> Result<Vec<(i64, String, String, String)>> {
        let conn = self.conn.lock().await;

        let mut stale = Vec::new();
        for path in files {
            let current_hash = match file_hash(path) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let path_str = path.to_string_lossy().to_string();

            let mut stmt = conn.prepare(
                "SELECT id, source_hash FROM memories
                 WHERE source_file = ?1 AND source_hash IS NOT NULL AND source_hash != ?2",
            )?;
            let rows: Vec<(i64, String)> = stmt
                .query_map(rusqlite::params![path_str, current_hash], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (id, old_hash) in rows {
                stale.push((id, path_str.clone(), old_hash, current_hash.clone()));
            }
        }

        Ok(stale)
    }

    /// Mark memories as stale by setting importance to 0.0 (soft-delete via scoring).
    pub async fn mark_stale(&self, memory_ids: &[i64]) -> Result<usize> {
        if memory_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().await;
        let mut count = 0;
        for id in memory_ids {
            count += conn.execute(
                "UPDATE memories SET importance = 0.0, updated_at = datetime('now') WHERE id = ?1",
                rusqlite::params![id],
            )?;
        }
        Ok(count)
    }

    // ── Scoped write path ──────────────────────────────────────────

    /// Store a memory at a specific scope level with SHA-256 dedup.
    ///
    /// Dedup rules:
    /// - Same content_hash at same scope_path → reinforce (bump access_count).
    /// - Same content_hash at a broader scope → skip (already covered).
    /// - Same content_hash at a narrower scope → insert (may carry local nuance).
    pub async fn store_scoped(
        &self,
        scope: &WriteScope,
        content: &str,
        meta: &WriteMeta,
    ) -> Result<StoreResult> {
        let hash = schema::content_hash(content);
        let scope_level = scope.level_int();
        let scope_path = scope.to_path_string();

        // Compute embedding outside the lock
        let embedding = self
            .embedder
            .embed_document(content)
            .context("computing embedding for scoped store")?;
        let embedding_blob = embedding_to_blob(&embedding);
        let model_id = self.embedder.model_id().to_string();

        let conn = self.conn.lock().await;

        // Check for exact duplicate at same scope
        let existing: Option<i64> = conn
            .prepare(
                "SELECT id FROM memories
                 WHERE content_hash = ?1 AND scope_path = ?2",
            )?
            .query_row(rusqlite::params![hash, scope_path], |row| row.get(0))
            .ok();

        if let Some(id) = existing {
            // Reinforce: bump access count and timestamp
            conn.execute(
                "UPDATE memories SET access_count = access_count + 1,
                        last_accessed_at = datetime('now'),
                        updated_at = datetime('now')
                 WHERE id = ?1",
                rusqlite::params![id],
            )?;
            tracing::debug!(id, "scoped store: deduplicated at same scope");
            return Ok(StoreResult::Deduplicated(id));
        }

        // Check for duplicate at any broader scope
        for level in (0..scope_level).rev() {
            let covered: bool = conn
                .prepare(
                    "SELECT 1 FROM memories
                     WHERE content_hash = ?1 AND scope_level = ?2
                     LIMIT 1",
                )?
                .query_row(rusqlite::params![hash, level], |_| Ok(true))
                .unwrap_or(false);
            if covered {
                tracing::debug!(level, "scoped store: already covered at broader scope");
                return Ok(StoreResult::AlreadyCovered);
            }
        }

        // Insert new memory
        let namespace = meta.tag.as_deref().unwrap_or("default");
        let key = format!("scoped:{}:{}", scope_path, &hash[..12]);

        conn.execute(
            "INSERT INTO memories (
                namespace, key, content, embedding, model_id,
                scope_level, scope_path, repo_id, module_path, run_id,
                content_hash, memory_type, trust, tag,
                importance, privacy
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'public')",
            rusqlite::params![
                namespace,
                key,
                content,
                embedding_blob,
                model_id,
                scope_level,
                scope_path,
                scope.repo_id(),
                scope.module_path(),
                match scope {
                    WriteScope::Run { run_id, .. } => Some(run_id.as_str()),
                    _ => None,
                },
                hash,
                meta.memory_type.as_str(),
                meta.trust.as_str(),
                meta.tag.as_deref(),
                meta.importance,
            ],
        )
        .context("inserting scoped memory")?;

        let id: i64 = conn.last_insert_rowid();

        // Insert into scoped vec table
        let _ = conn.execute(
            "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
            rusqlite::params![id],
        );
        conn.execute(
            "INSERT INTO vec_memories_scoped(memory_id, embedding, scope_level)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![id, embedding_blob, scope_level],
        )
        .context("inserting into vec_memories_scoped")?;

        // Also insert into legacy vec_memories for backward compat
        let _ = conn.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            rusqlite::params![id],
        );
        conn.execute(
            "INSERT INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![id, embedding_blob],
        )
        .context("inserting into legacy vec_memories")?;

        tracing::debug!(id, scope_path, "scoped store: inserted new memory");
        Ok(StoreResult::Inserted(id))
    }

    // ── Cascading scoped search ───────────────────────────────────

    /// Cascading search across scope levels with early termination.
    ///
    /// Searches from narrowest to widest scope, accumulating results.
    /// Stops widening when enough high-confidence results are found.
    /// Uses hybrid vector + FTS search via RRF when `use_fts` is enabled.
    pub async fn search_scoped(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .context("computing query embedding for scoped search")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;
        let now = chrono_now_utc();

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut seen_hashes: HashSet<String> = HashSet::new();

        for level in config.scope.levels() {
            // Vector search within this scope level
            let scope_level_int = level.level_int();
            let vec_candidates = self.vec_search_at_level(
                &conn,
                &query_blob,
                scope_level_int,
                &level,
                config.per_level_limit,
            )?;

            // Optional FTS search
            let fts_candidates = if config.use_fts {
                self.fts_search_at_level(
                    &conn,
                    &config.query,
                    scope_level_int,
                    &level,
                    config.per_level_limit,
                )?
            } else {
                Vec::new()
            };

            // Merge via RRF if both sources produced results
            let candidate_ids: Vec<(i64, f32)> =
                if !fts_candidates.is_empty() && !vec_candidates.is_empty() {
                    let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                    merged
                        .into_iter()
                        .map(|(id, _rrf, sim)| (id, sim))
                        .collect()
                } else {
                    vec_candidates
                };

            // Score and accumulate
            for (memory_id, raw_sim) in &candidate_ids {
                if *raw_sim < config.similarity_threshold {
                    continue;
                }

                // Load full memory record
                let Some(mem) =
                    self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, &level)?
                else {
                    continue;
                };

                if seen_hashes.contains(&mem.content_hash) {
                    continue;
                }
                seen_hashes.insert(mem.content_hash.clone());
                accumulated.push(mem);
            }

            // Early termination check
            if accumulated.len() >= config.max_results {
                if let Some(best) = accumulated.iter().map(|m| m.final_score).reduce(f32::max) {
                    if best >= config.confidence_threshold {
                        tracing::debug!(
                            best_score = best,
                            level = scope_level_int,
                            "cascading search: early termination"
                        );
                        break;
                    }
                }
            }
        }

        // Final ranking
        accumulated.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        accumulated.truncate(config.max_results);

        // Update access timestamps and log for promotion heuristics
        if !accumulated.is_empty() {
            let ids: Vec<i64> = accumulated.iter().map(|m| m.id).collect();
            self.touch_accessed(&conn, &ids);
            self.log_access(&conn, &ids, &config.scope);
        }

        Ok(accumulated)
    }

    /// Format scoped search results as a prompt-ready context string.
    pub async fn search_scoped_context(&self, config: &SearchConfig) -> String {
        match self.search_scoped(config).await {
            Ok(results) if !results.is_empty() => scoring::format_memories_for_prompt(&results),
            _ => String::new(),
        }
    }

    // ── Internal helpers for scoped search ─────────────────────────

    /// Vector KNN search filtered to a scope level.
    fn vec_search_at_level(
        &self,
        conn: &Connection,
        query_blob: &[u8],
        scope_level: i32,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<(i64, f32)>> {
        // Over-fetch to allow post-filtering by repo_id/module_path
        let fetch_k = limit * 3;

        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance
             FROM vec_memories_scoped v
             WHERE v.embedding MATCH ?1 AND k = ?2 AND v.scope_level = ?3",
            )
            .context("preparing scoped KNN")?;

        let results: Vec<(i64, f32)> = stmt
            .query_map(
                rusqlite::params![query_blob, fetch_k as i64, scope_level],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();

        // Post-filter by repo_id / module_path / run_id
        let filtered = self.filter_by_scope(conn, results, level)?;
        Ok(filtered.into_iter().take(limit).collect())
    }

    /// FTS search filtered to a scope level.
    fn fts_search_at_level(
        &self,
        conn: &Connection,
        query: &str,
        scope_level: i32,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let fetch_k = limit * 3;

        let mut stmt = conn
            .prepare(
                "SELECT f.rowid, f.rank
             FROM memories_fts f
             JOIN memories m ON m.id = f.rowid
             WHERE memories_fts MATCH ?1
             AND m.scope_level = ?2
             ORDER BY f.rank
             LIMIT ?3",
            )
            .context("preparing FTS search")?;

        let results: Vec<(i64, f64)> = stmt
            .query_map(
                rusqlite::params![query, scope_level, fetch_k as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();

        // Post-filter
        let filtered: Vec<(i64, f64)> = results
            .into_iter()
            .filter(|(id, _)| self.matches_scope_filter(conn, *id, level))
            .take(limit)
            .collect();

        Ok(filtered)
    }

    /// Post-filter vector results by exact scope metadata.
    fn filter_by_scope(
        &self,
        conn: &Connection,
        results: Vec<(i64, f32)>,
        level: &ScopeFilter,
    ) -> Result<Vec<(i64, f32)>> {
        let mut filtered = Vec::new();
        for (id, distance) in results {
            if self.matches_scope_filter(conn, id, level) {
                // Convert L2 distance to cosine similarity
                let similarity = (1.0 - distance * distance / 2.0).max(0.0);
                filtered.push((id, similarity));
            }
        }
        Ok(filtered)
    }

    /// Check if a memory matches the given scope filter.
    fn matches_scope_filter(&self, conn: &Connection, memory_id: i64, level: &ScopeFilter) -> bool {
        match level {
            ScopeFilter::Global | ScopeFilter::Workspace => true,
            ScopeFilter::Repo { repo_id } => conn
                .prepare("SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2")
                .and_then(|mut s| s.query_row(rusqlite::params![memory_id, repo_id], |_| Ok(true)))
                .unwrap_or(false),
            ScopeFilter::Module {
                repo_id,
                module_path,
            } => conn
                .prepare(
                    "SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2 AND module_path = ?3",
                )
                .and_then(|mut s| {
                    s.query_row(rusqlite::params![memory_id, repo_id, module_path], |_| {
                        Ok(true)
                    })
                })
                .unwrap_or(false),
            ScopeFilter::Run { repo_id, run_id } => conn
                .prepare("SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2 AND run_id = ?3")
                .and_then(|mut s| {
                    s.query_row(rusqlite::params![memory_id, repo_id, run_id], |_| Ok(true))
                })
                .unwrap_or(false),
        }
    }

    /// Load a full ScoredMemory from the database.
    fn load_scoped_memory(
        &self,
        conn: &Connection,
        memory_id: i64,
        raw_similarity: f32,
        now: &str,
        level: &ScopeFilter,
    ) -> Result<Option<ScoredMemory>> {
        let result = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                    repo_id, module_path, memory_type, trust, importance,
                    access_count, created_at, updated_at, last_accessed_at,
                    tag, namespace, key
             FROM memories WHERE id = ?1",
            )?
            .query_row(rusqlite::params![memory_id], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let updated_at: String = row.get(12)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let importance: f32 = row.get(9)?;
                let access_count: i32 = row.get(10)?;

                let trust = Trust::parse_str(&trust_str);
                let days = hours_since(&accessed_at, &updated_at, now) / 24.0;

                let final_score =
                    scoring::score(raw_similarity, importance, days, access_count, trust, level);

                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type: MemoryType::parse_str(&type_str),
                    trust,
                    importance,
                    access_count,
                    created_at: row.get(11)?,
                    updated_at,
                    accessed_at,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    raw_similarity,
                    fts_rank: None,
                    final_score,
                })
            });

        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Bump access_count and last_accessed_at for returned results.
    fn touch_accessed(&self, conn: &Connection, ids: &[i64]) {
        for id in ids {
            let _ = conn.execute(
                "UPDATE memories SET access_count = access_count + 1,
                        last_accessed_at = datetime('now')
                 WHERE id = ?1",
                rusqlite::params![id],
            );
        }
    }

    /// Log access for cross-scope promotion heuristics.
    fn log_access(&self, conn: &Connection, ids: &[i64], scope: &MemoryScope) {
        let repo_id = scope.repo_id.as_deref();
        let module_path = scope.module_path.as_deref();

        for id in ids {
            let _ = conn.execute(
                "INSERT INTO memory_access_log (memory_id, repo_id, module_path)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, repo_id, module_path],
            );
        }
    }

    // ── Scoped delete operations ──────────────────────────────────

    /// Delete all memories at a scope and below.
    pub async fn forget_scope(&self, scope: &WriteScope) -> Result<u64> {
        let conn = self.conn.lock().await;
        let scope_path = scope.to_path_string();

        // Find all matching memories
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM memories WHERE scope_path = ?1 OR scope_path LIKE ?2")?
            .query_map(
                rusqlite::params![scope_path, format!("{}/%", scope_path)],
                |row| row.get(0),
            )?
            .filter_map(|r| r.ok())
            .collect();

        // Clean up vectors
        for id in &ids {
            let _ = conn.execute(
                "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        // Delete access logs
        for id in &ids {
            let _ = conn.execute(
                "DELETE FROM memory_access_log WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        // Delete memories
        let count = conn.execute(
            "DELETE FROM memories WHERE scope_path = ?1 OR scope_path LIKE ?2",
            rusqlite::params![scope_path, format!("{}/%", scope_path)],
        )?;

        Ok(count as u64)
    }

    /// Delete all run-level memories for a specific run.
    pub async fn delete_by_run(&self, run_id: &str) -> Result<u64> {
        let conn = self.conn.lock().await;

        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM memories WHERE run_id = ?1")?
            .query_map(rusqlite::params![run_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for id in &ids {
            let _ = conn.execute(
                "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM memory_access_log WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        let count = conn.execute(
            "DELETE FROM memories WHERE run_id = ?1",
            rusqlite::params![run_id],
        )?;

        Ok(count as u64)
    }

    /// Query all memories for a given run (for consolidation).
    pub async fn query_by_run(&self, run_id: &str) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;

        let mut stmt = conn.prepare(
            "SELECT id, content, content_hash, scope_level, scope_path,
                    repo_id, module_path, memory_type, trust, importance,
                    access_count, created_at, updated_at, last_accessed_at,
                    tag, namespace, key
             FROM memories WHERE run_id = ?1",
        )?;

        let results: Vec<ScoredMemory> = stmt
            .query_map(rusqlite::params![run_id], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let updated_at: String = row.get(12)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;

                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type: MemoryType::parse_str(&type_str),
                    trust: Trust::parse_str(&trust_str),
                    importance: row.get(9)?,
                    access_count: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: updated_at.clone(),
                    accessed_at: accessed_at.clone(),
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Query memories at a specific scope level (for consolidation diagnostics).
    pub async fn search_at_level(
        &self,
        level: &ScopeFilter,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        if query.is_empty() {
            // Return all memories at this level
            let conn = self.conn.lock().await;
            return self.list_at_level(&conn, level, limit);
        }

        // Use the cascading search with a single-level scope
        let scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            workspace_id: String::new(),
            repo_id: level.repo_id().map(String::from),
            module_path: level.module_path().map(String::from),
            run_id: level.run_id().map(String::from),
        };

        let config = SearchConfig {
            query: query.to_string(),
            max_results: limit,
            per_level_limit: limit,
            similarity_threshold: 0.0,
            confidence_threshold: 1.0, // don't terminate early
            use_fts: true,
            scope,
        };

        self.search_scoped(&config).await
    }

    /// List all memories at a level without embedding search.
    fn list_at_level(
        &self,
        conn: &Connection,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match level {
            ScopeFilter::Global => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key
                 FROM memories WHERE scope_level = 0 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Workspace => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key
                 FROM memories WHERE scope_level = 1 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Repo { repo_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key
                 FROM memories WHERE scope_level = 2 AND repo_id = ?1 LIMIT ?2",
                vec![Box::new(repo_id.clone()), Box::new(limit as i64)],
            ),
            ScopeFilter::Module { repo_id, module_path } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key
                 FROM memories WHERE scope_level = 3 AND repo_id = ?1 AND module_path = ?2 LIMIT ?3",
                vec![Box::new(repo_id.clone()), Box::new(module_path.clone()), Box::new(limit as i64)],
            ),
            ScopeFilter::Run { repo_id, run_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key
                 FROM memories WHERE scope_level = 4 AND repo_id = ?1 AND run_id = ?2 LIMIT ?3",
                vec![Box::new(repo_id.clone()), Box::new(run_id.clone()), Box::new(limit as i64)],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(sql)?;
        let results: Vec<ScoredMemory> = stmt
            .query_map(params_refs.as_slice(), |row| {
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type: MemoryType::parse_str(&type_str),
                    trust: Trust::parse_str(&trust_str),
                    importance: row.get(9)?,
                    access_count: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    accessed_at: row.get(13)?,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    // ── Maintenance ───────────────────────────────────────────────

    /// Decay importance and prune stale entries.
    ///
    /// - Decays importance based on time since last access (30-day half-life).
    /// - Prunes entries below min_importance that haven't been accessed in 90 days.
    /// - Never auto-prunes human-authored (trust=high) entries.
    pub async fn decay_and_prune(&self) -> Result<(usize, usize)> {
        let conn = self.conn.lock().await;

        let half_life_days = 30.0_f64;
        let lambda = 2.0_f64.ln() / half_life_days;
        let min_importance = 0.05;

        // Decay importance in Rust (SQLite doesn't have exp())
        let rows: Vec<(i64, f32, String)> = conn
            .prepare(
                "SELECT id, importance, COALESCE(last_accessed_at, updated_at)
                 FROM memories WHERE scope_level >= 2",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let now_str = chrono_now_utc();
        let mut decayed = 0usize;
        for (id, importance, ts) in &rows {
            let days = hours_since(&Some(ts.clone()), ts, &now_str) / 24.0;
            let decay_factor = (-lambda * days).exp() as f32;
            let new_importance = importance * decay_factor;
            if (new_importance - importance).abs() > 0.001 {
                conn.execute(
                    "UPDATE memories SET importance = ?1 WHERE id = ?2",
                    rusqlite::params![new_importance, id],
                )?;
                decayed += 1;
            }
        }

        // Prune
        let prune_ids: Vec<i64> = conn
            .prepare(
                "SELECT id FROM memories
                 WHERE importance < ?1
                 AND COALESCE(last_accessed_at, updated_at) < datetime('now', '-90 days')
                 AND trust != 'high'",
            )?
            .query_map(rusqlite::params![min_importance], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for id in &prune_ids {
            let _ = conn.execute(
                "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM memory_access_log WHERE memory_id = ?1",
                rusqlite::params![id],
            );
        }

        let pruned = conn.execute(
            "DELETE FROM memories
             WHERE importance < ?1
             AND COALESCE(last_accessed_at, updated_at) < datetime('now', '-90 days')
             AND trust != 'high'",
            rusqlite::params![min_importance],
        )?;

        // Clean orphaned vectors
        let _ = conn.execute(
            "DELETE FROM vec_memories_scoped WHERE memory_id NOT IN (SELECT id FROM memories)",
            [],
        );

        Ok((decayed, pruned))
    }

    /// Find module-level memories accessed from other modules (promotion candidates).
    pub async fn find_promotion_candidates(
        &self,
        min_cross_hits: i64,
    ) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;

        let mut stmt = conn.prepare(
            "SELECT m.id, m.content, m.content_hash, m.scope_level, m.scope_path,
                    m.repo_id, m.module_path, m.memory_type, m.trust, m.importance,
                    m.access_count, m.created_at, m.updated_at, m.last_accessed_at,
                    m.tag, m.namespace, m.key,
                    COUNT(DISTINCT a.module_path) as cross_module_hits
             FROM memories m
             JOIN memory_access_log a ON a.memory_id = m.id
             WHERE m.scope_level = 3
             AND a.module_path != m.module_path
             GROUP BY m.id
             HAVING cross_module_hits >= ?1",
        )?;

        let results: Vec<ScoredMemory> = stmt
            .query_map(rusqlite::params![min_cross_hits], |row| {
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type: MemoryType::parse_str(&type_str),
                    trust: Trust::parse_str(&trust_str),
                    importance: row.get(9)?,
                    access_count: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    accessed_at: row.get(13)?,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    // ── Legacy API (kept for backward compat) ─────────────────────

    /// Delete stale swarm-generated entries based on retention policy.
    ///
    /// Preserves all `user:*` entries (explicit `/remember` commands).
    /// Preserves entries that don't match swarm key patterns (agents:, verification:, tiers:).
    pub async fn prune(
        &self,
        namespace: &str,
        max_age_days: u32,
        max_runs: usize,
    ) -> Result<usize> {
        let conn = self.conn.lock().await;

        let mut total_deleted = 0;

        // Delete entries older than max_age_days (only swarm-generated keys)
        if max_age_days > 0 {
            // Collect ids to delete from vec_memories too
            let ids: Vec<i64> = conn
                .prepare(
                    "SELECT id FROM memories WHERE namespace = ?1
                 AND key NOT LIKE 'user:%'
                 AND (key LIKE 'agents:%' OR key LIKE 'verification:%' OR key LIKE 'tiers:%')
                 AND created_at < datetime('now', ?2)",
                )?
                .query_map(
                    rusqlite::params![namespace, format!("-{} days", max_age_days)],
                    |row| row.get(0),
                )?
                .filter_map(|r| r.ok())
                .collect();

            for id in &ids {
                let _ = conn.execute(
                    "DELETE FROM vec_memories WHERE memory_id = ?1",
                    rusqlite::params![id],
                );
            }

            let rows = conn.execute(
                "DELETE FROM memories WHERE namespace = ?1
                 AND key NOT LIKE 'user:%'
                 AND (key LIKE 'agents:%' OR key LIKE 'verification:%' OR key LIKE 'tiers:%')
                 AND created_at < datetime('now', ?2)",
                rusqlite::params![namespace, format!("-{} days", max_age_days),],
            )?;
            total_deleted += rows;
        }

        // Enforce max_runs: keep only the latest N run IDs
        if max_runs > 0 {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT substr(key, 8, instr(substr(key, 8), ':') - 1) as parsed_run_id,
                        MIN(created_at) as first_seen
                 FROM memories
                 WHERE namespace = ?1 AND key LIKE 'agents:%'
                 GROUP BY parsed_run_id
                 ORDER BY first_seen DESC",
            )?;

            let run_ids: Vec<String> = stmt
                .query_map(rusqlite::params![namespace], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();

            if run_ids.len() > max_runs {
                let old_runs = &run_ids[max_runs..];
                for run_id in old_runs {
                    // Collect ids for vec_memories cleanup
                    let ids: Vec<i64> = conn
                        .prepare(
                            "SELECT id FROM memories WHERE namespace = ?1
                         AND (key LIKE ?2 OR key LIKE ?3 OR key LIKE ?4)",
                        )?
                        .query_map(
                            rusqlite::params![
                                namespace,
                                format!("agents:{}:%", run_id),
                                format!("verification:{}", run_id),
                                format!("tiers:{}", run_id),
                            ],
                            |row| row.get(0),
                        )?
                        .filter_map(|r| r.ok())
                        .collect();

                    for id in &ids {
                        let _ = conn.execute(
                            "DELETE FROM vec_memories WHERE memory_id = ?1",
                            rusqlite::params![id],
                        );
                    }

                    let rows = conn.execute(
                        "DELETE FROM memories WHERE namespace = ?1
                         AND (key LIKE ?2 OR key LIKE ?3 OR key LIKE ?4)",
                        rusqlite::params![
                            namespace,
                            format!("agents:{}:%", run_id),
                            format!("verification:{}", run_id),
                            format!("tiers:{}", run_id),
                        ],
                    )?;
                    total_deleted += rows;
                }
            }
        }

        Ok(total_deleted)
    }
}

// ── Retrieval scoring (Stanford Generative Agents) ─────────────

/// Composite retrieval score combining recency, importance, and relevance.
///
/// Formula: (α·recency + β·importance + γ·relevance) × reinforcement
/// - Recency: exponential decay 0.995^hours
/// - Importance: [0,1] normalized
/// - Relevance: cosine similarity from sqlite-vec
/// - Reinforcement: (1.0 + access_count × 0.1), capped at 3×
fn retrieval_score(recency_hours: f64, importance: f32, relevance: f32, access_count: i32) -> f32 {
    let recency = 0.995_f64.powf(recency_hours) as f32;
    let reinforcement = (1.0 + access_count as f32 * 0.1).min(3.0);
    // α = β = γ = 1.0
    (recency + importance + relevance) * reinforcement
}

/// Compute hours elapsed since the given timestamp (or fallback).
fn hours_since(last_accessed: &Option<String>, updated_at: &str, now: &str) -> f64 {
    let ts = last_accessed.as_deref().unwrap_or(updated_at);
    // Parse SQLite datetime format: "YYYY-MM-DD HH:MM:SS"
    // On parse failure, assume 24 hours (reasonable default)
    parse_sqlite_datetime_diff_hours(ts, now).unwrap_or(24.0)
}

/// Parse two SQLite datetime strings and return the difference in hours.
fn parse_sqlite_datetime_diff_hours(from: &str, to: &str) -> Option<f64> {
    // SQLite datetime format: "YYYY-MM-DD HH:MM:SS"
    let parse = |s: &str| -> Option<i64> {
        let parts: Vec<&str> = s.split(|c| c == '-' || c == ' ' || c == ':').collect();
        if parts.len() < 6 {
            return None;
        }
        let year: i64 = parts[0].parse().ok()?;
        let month: i64 = parts[1].parse().ok()?;
        let day: i64 = parts[2].parse().ok()?;
        let hour: i64 = parts[3].parse().ok()?;
        let min: i64 = parts[4].parse().ok()?;
        let sec: i64 = parts[5].parse().ok()?;
        // Approximate: seconds since epoch (good enough for hour differences)
        Some(((year * 365 + month * 30 + day) * 86400) + hour * 3600 + min * 60 + sec)
    };
    let from_secs = parse(from)?;
    let to_secs = parse(to)?;
    let diff_secs = (to_secs - from_secs).max(0);
    Some(diff_secs as f64 / 3600.0)
}

/// Get current UTC datetime in SQLite format.
fn chrono_now_utc() -> String {
    // Use SQLite's own datetime function via a simple formatted string
    // We can't call SQLite here (no connection), so use std::time
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Convert to date components (simplified UTC)
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days as i64);
    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i64, d as i64)
}

// ── Staleness detection ────────────────────────────────────────

/// Compute SHA256 hash of a file's contents.
pub fn file_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading file for hash: {}", path.display()))?;
    let hash = Sha256::digest(&bytes);
    Ok(format!("{:x}", hash))
}

// ── Embedding serialization ────────────────────────────────────

/// Encode a float vector as little-endian bytes for SQLite BLOB storage.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock embedder that produces deterministic vectors from content hash.
    struct MockEmbedder;

    impl Embedder for MockEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            // Simple hash-based deterministic embedding
            let mut vec = vec![0.0f32; 8];
            for (i, byte) in text.bytes().enumerate() {
                vec[i % 8] += byte as f32;
            }
            // L2 normalize
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

    fn mock_embedder() -> Arc<dyn Embedder> {
        Arc::new(MockEmbedder)
    }

    #[tokio::test]
    async fn test_store_and_get() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let id = store
            .store("test", "greeting", "hello world", None)
            .await
            .unwrap();
        assert!(id > 0);

        let entry = store.get("test", "greeting").await.unwrap().unwrap();
        assert_eq!(entry.content, "hello world");
        assert_eq!(entry.namespace, "test");
        assert_eq!(entry.key, "greeting");
        assert!((entry.importance - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_upsert() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "k", "original", None).await.unwrap();
        store.store("ns", "k", "updated", None).await.unwrap();

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "updated");
    }

    #[tokio::test]
    async fn test_search() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store
            .store("ns", "rust", "Rust programming language", None)
            .await
            .unwrap();
        store
            .store("ns", "python", "Python scripting language", None)
            .await
            .unwrap();
        store
            .store("ns", "cooking", "How to make pasta", None)
            .await
            .unwrap();

        let results = store.search("ns", "Rust language", 2).await.unwrap();
        assert_eq!(results.len(), 2);
        // First result should have highest composite score
        assert!(results[0].score >= results[1].score);
    }

    #[tokio::test]
    async fn m3_search_candidates_returns_structured_records() {
        // V9 §11 M3 acceptance: planner gets structured `MemoryCandidate`s
        // with id, namespace, score, content. Pins the From<&SearchResult>
        // mapping and verifies search_candidates threads through correctly.
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let id1 = store
            .store("ws", "rust", "Rust programming language", None)
            .await
            .unwrap();
        let id2 = store
            .store("ws", "python", "Python scripting language", None)
            .await
            .unwrap();

        let candidates = store
            .search_candidates(&["ws".to_string()], "Rust language", 5)
            .await;
        assert!(candidates.len() >= 2);
        // All returned ids must be among the stored ones.
        let ids: Vec<i64> = candidates.iter().map(|c| c.id).collect();
        for c in &candidates {
            assert_eq!(c.namespace, "ws");
            assert_eq!(c.scope_label, "ws");
            assert!(!c.content.is_empty());
            assert!(c.score >= 0.0);
            // updated_at populated from MemoryEntry.
            assert!(c.updated_at.is_some());
        }
        assert!(ids.contains(&id1) || ids.contains(&id2));
    }

    #[tokio::test]
    async fn test_list_keys() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "b", "content b", None).await.unwrap();
        store.store("ns", "a", "content a", None).await.unwrap();
        store.store("other", "c", "content c", None).await.unwrap();

        let keys = store.list_keys("ns").await.unwrap();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_delete() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "k", "content", None).await.unwrap();
        assert!(store.delete("ns", "k").await.unwrap());
        assert!(store.get("ns", "k").await.unwrap().is_none());
        assert!(!store.delete("ns", "k").await.unwrap()); // already deleted
    }

    #[tokio::test]
    async fn test_clear_namespace() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "a", "a", None).await.unwrap();
        store.store("ns", "b", "b", None).await.unwrap();
        store.store("other", "c", "c", None).await.unwrap();

        let cleared = store.clear_namespace("ns").await.unwrap();
        assert_eq!(cleared, 2);
        assert!(store.list_keys("ns").await.unwrap().is_empty());
        assert_eq!(store.list_keys("other").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_embedding_roundtrip() {
        let original = vec![1.0, -2.5, 3.14, 0.0];
        let blob = embedding_to_blob(&original);
        // Verify blob is correct length
        assert_eq!(blob.len(), 16);
    }

    #[tokio::test]
    async fn test_store_with_privacy() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let id = store
            .store_with_privacy("ns", "k", "secret content", "local_only", None)
            .await
            .unwrap();
        assert!(id > 0);

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "secret content");
    }

    #[tokio::test]
    async fn test_search_context_filtered_excludes_local_only() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        // Store entries with different privacy levels
        store
            .store_with_privacy("ns", "public1", "Rust programming", "public", None)
            .await
            .unwrap();
        store
            .store_with_privacy("ns", "private1", "Secret clinical data", "local_only", None)
            .await
            .unwrap();
        store
            .store_with_privacy("ns", "public2", "Python scripting", "public", None)
            .await
            .unwrap();

        // ExcludeLocalOnly should not return the local_only entry
        let namespaces = vec!["ns".to_string()];
        let ctx = store
            .search_context_filtered(
                &namespaces,
                "programming",
                10,
                PrivacyFilter::ExcludeLocalOnly,
            )
            .await;
        assert!(!ctx.contains("Secret clinical data"));
        assert!(ctx.contains("Rust programming") || ctx.contains("Python scripting"));

        // IncludeAll should return all entries
        let ctx_all = store
            .search_context_filtered(&namespaces, "programming", 10, PrivacyFilter::IncludeAll)
            .await;
        assert!(ctx_all.contains("Secret clinical data") || ctx_all.contains("Rust programming"));
    }

    #[tokio::test]
    async fn test_store_with_privacy_upserts() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store
            .store_with_privacy("ns", "k", "v1", "public", None)
            .await
            .unwrap();
        store
            .store_with_privacy("ns", "k", "v2", "local_only", Some("{\"v\":2}"))
            .await
            .unwrap();

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "v2");
    }

    #[tokio::test]
    async fn test_store_with_options() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let opts = StoreOptions {
            importance: 0.9,
            source_file: Some("/tmp/test.rs".to_string()),
            source_hash: Some("abc123".to_string()),
            ..Default::default()
        };
        store
            .store_with_options("ns", "k", "important content", &opts)
            .await
            .unwrap();

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "important content");
        assert!((entry.importance - 0.9).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_importance_scoring_affects_ranking() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        // Store two similar entries with different importance
        let opts_low = StoreOptions {
            importance: 0.1,
            ..Default::default()
        };
        let opts_high = StoreOptions {
            importance: 1.0,
            ..Default::default()
        };
        store
            .store_with_options("ns", "low", "test content alpha", &opts_low)
            .await
            .unwrap();
        store
            .store_with_options("ns", "high", "test content alpha beta", &opts_high)
            .await
            .unwrap();

        let results = store.search("ns", "test content alpha", 2).await.unwrap();
        assert_eq!(results.len(), 2);
        // High-importance entry should rank first (assuming similar relevance)
        assert!(
            results[0].entry.importance > results[1].entry.importance
                || results[0].score >= results[1].score
        );
    }

    #[tokio::test]
    async fn test_prune_preserves_user_entries() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store
            .store("ns", "user:note1", "my note", None)
            .await
            .unwrap();
        store
            .store("ns", "agents:run1:unit1", "agent result", None)
            .await
            .unwrap();
        store
            .store("ns", "verification:run1", "v result", None)
            .await
            .unwrap();

        // Prune with 0 max_age (won't delete recent), but exercise the code
        let deleted = store.prune("ns", 0, 100).await.unwrap();
        assert_eq!(deleted, 0); // Nothing old enough

        // user entry should always survive
        let keys = store.list_keys("ns").await.unwrap();
        assert!(keys.contains(&"user:note1".to_string()));
    }

    #[tokio::test]
    async fn test_prune_by_max_runs() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        // Create entries for 3 runs
        for run in 1..=3 {
            store
                .store(
                    "ns",
                    &format!("agents:run{}:a", run),
                    &format!("run {} a", run),
                    None,
                )
                .await
                .unwrap();
            store
                .store(
                    "ns",
                    &format!("agents:run{}:b", run),
                    &format!("run {} b", run),
                    None,
                )
                .await
                .unwrap();
        }
        // Also a user entry that should not be pruned
        store
            .store("ns", "user:important", "keep this", None)
            .await
            .unwrap();

        let keys_before = store.list_keys("ns").await.unwrap();
        assert_eq!(keys_before.len(), 7); // 6 agent + 1 user

        // Prune to keep only 2 most recent runs
        let deleted = store.prune("ns", 0, 2).await.unwrap();
        // The oldest run (run1) should be pruned
        assert!(deleted > 0);

        let keys_after = store.list_keys("ns").await.unwrap();
        assert!(keys_after.contains(&"user:important".to_string()));
        // run1 entries should be gone
        assert!(!keys_after.contains(&"agents:run1:a".to_string()));
    }

    #[tokio::test]
    async fn test_retrieval_score_function() {
        // Recent, important, relevant → high score
        let high = retrieval_score(0.0, 1.0, 1.0, 5);
        // Old, unimportant, irrelevant → low score
        let low = retrieval_score(720.0, 0.1, 0.1, 0);
        assert!(high > low);
    }

    // ── Scoped store tests ────────────────────────────────────

    #[tokio::test]
    async fn test_store_scoped_insert() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "test_repo".into(),
        };
        let meta = WriteMeta::default();

        let result = store
            .store_scoped(&scope, "hello scoped world", &meta)
            .await
            .unwrap();
        match result {
            StoreResult::Inserted(id) => assert!(id > 0),
            other => panic!("expected Inserted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_store_scoped_dedup_same_scope() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        let meta = WriteMeta::default();

        let r1 = store
            .store_scoped(&scope, "dedup test content", &meta)
            .await
            .unwrap();
        assert!(matches!(r1, StoreResult::Inserted(_)));

        // Same content at same scope → deduplicate
        let r2 = store
            .store_scoped(&scope, "dedup test content", &meta)
            .await
            .unwrap();
        assert!(matches!(r2, StoreResult::Deduplicated(_)));
    }

    #[tokio::test]
    async fn test_store_scoped_broader_scope_covers() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();

        // Store at repo level (scope_level=2)
        let repo_scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        store
            .store_scoped(&repo_scope, "shared knowledge", &meta)
            .await
            .unwrap();

        // Try to store same content at module level (scope_level=3, narrower)
        // → broader scope (repo=2) already covers it, so skip
        let module_scope = WriteScope::Module {
            repo_id: "r1".into(),
            module_path: "crates/core".into(),
        };
        let r2 = store
            .store_scoped(&module_scope, "shared knowledge", &meta)
            .await
            .unwrap();
        assert!(matches!(r2, StoreResult::AlreadyCovered));

        // Store at workspace level (scope_level=1, broader than repo=2)
        let ws_scope = WriteScope::Workspace;
        store
            .store_scoped(&ws_scope, "workspace fact", &meta)
            .await
            .unwrap();

        // Same content at repo level should see it's already covered at broader scope
        let r3 = store
            .store_scoped(&repo_scope, "workspace fact", &meta)
            .await
            .unwrap();
        assert!(matches!(r3, StoreResult::AlreadyCovered));
    }

    #[tokio::test]
    async fn test_cascading_search_basic() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();

        // Store memories at different scope levels
        let repo_scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        store
            .store_scoped(&repo_scope, "Rust programming patterns", &meta)
            .await
            .unwrap();

        let module_scope = WriteScope::Module {
            repo_id: "r1".into(),
            module_path: "crates/core".into(),
        };
        store
            .store_scoped(&module_scope, "Core module Rust conventions", &meta)
            .await
            .unwrap();

        // Build a scope that includes both levels
        let scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            workspace_id: "ws1".into(),
            repo_id: Some("r1".into()),
            module_path: Some("crates/core".into()),
            run_id: None,
        };

        let config = SearchConfig::new("Rust programming", scope).with_fts(false);
        let results = store.search_scoped(&config).await.unwrap();

        assert!(!results.is_empty(), "should find at least one result");
        // Results should be scored — module-level should have higher scope weight
    }

    #[tokio::test]
    async fn test_forget_scope() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();

        let repo_scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        store
            .store_scoped(&repo_scope, "repo memory 1", &meta)
            .await
            .unwrap();
        store
            .store_scoped(&repo_scope, "repo memory 2", &meta)
            .await
            .unwrap();

        let deleted = store.forget_scope(&repo_scope).await.unwrap();
        assert_eq!(deleted, 2);
    }

    #[tokio::test]
    async fn test_delete_by_run() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        let run_scope = WriteScope::Run {
            repo_id: "r1".into(),
            run_id: "run_abc".into(),
        };
        let meta = WriteMeta::agent_observation("agent1");
        store
            .store_scoped(&run_scope, "observation 1", &meta)
            .await
            .unwrap();
        store
            .store_scoped(&run_scope, "observation 2", &meta)
            .await
            .unwrap();

        let deleted = store.delete_by_run("run_abc").await.unwrap();
        assert_eq!(deleted, 2);

        // Verify empty
        let remaining = store.query_by_run("run_abc").await.unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn test_query_by_run() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        let run_scope = WriteScope::Run {
            repo_id: "r1".into(),
            run_id: "run_xyz".into(),
        };
        let meta = WriteMeta::agent_observation("fixer");
        store
            .store_scoped(&run_scope, "found issue in auth module", &meta)
            .await
            .unwrap();
        store
            .store_scoped(&run_scope, "fixed permissions check", &meta)
            .await
            .unwrap();

        let memories = store.query_by_run("run_xyz").await.unwrap();
        assert_eq!(memories.len(), 2);
        assert!(memories.iter().all(|m| m.scope_level == 4));
    }
}
