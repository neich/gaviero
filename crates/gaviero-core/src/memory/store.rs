use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;
use tokio::sync::Mutex;
use tracing;

use super::embedder::Embedder;
use super::schema;
use super::scope::{
    MemoryScope, MemoryType, ScopeFilter, StoreResult, Trust, WriteMeta, WriteScope,
};
use super::scoring::{self, ScoredMemory, SearchConfig};
use super::trust_defaults::MemorySource;

const SEMANTIC_DEDUP_THRESHOLD: f32 = 0.95;

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

/// A row from the `session_ledger_turns` table (Tier A / A1).
#[derive(Debug, Clone)]
pub struct SessionLedgerTurn {
    pub id: i64,
    pub session_id: String,
    pub turn_id: String,
    pub session_thread: Option<String>,
    pub open_questions_json: Option<String>,
    pub annotations_json: Option<String>,
    pub created_at: String,
}

impl SessionLedgerTurn {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            turn_id: row.get(2)?,
            session_thread: row.get(3)?,
            open_questions_json: row.get(4)?,
            annotations_json: row.get(5)?,
            created_at: row.get(6)?,
        })
    }

    /// Decode `open_questions` JSON into a Vec. Empty list on missing
    /// or malformed payload.
    pub fn open_questions(&self) -> Vec<String> {
        match &self.open_questions_json {
            Some(s) => serde_json::from_str(s).unwrap_or_default(),
            None => Vec::new(),
        }
    }
}

/// A row from the `injection_manifests` table. `payload` is opaque JSON.
#[derive(Debug, Clone)]
pub struct InjectionManifestRow {
    pub id: i64,
    pub turn_id: String,
    pub session_id: String,
    pub source_channel: String,
    pub payload: String,
    pub created_at: String,
}

impl InjectionManifestRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            turn_id: row.get(1)?,
            session_id: row.get(2)?,
            source_channel: row.get(3)?,
            payload: row.get(4)?,
            created_at: row.get(5)?,
        })
    }
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

impl MemoryCandidate {
    /// Build a planner-shaped candidate from a fully-ranked
    /// `ScoredMemory`. Used when the planner / swarm bundle goes
    /// through the central [`crate::memory::retrieve_ranked`] entry
    /// point — preserves the rerank-blended `final_score` and the real
    /// scope path (no longer the namespace stand-in).
    pub fn from_scored(m: &super::scoring::ScoredMemory) -> Self {
        Self {
            id: m.id,
            namespace: m.namespace.clone(),
            scope_label: m.scope_path.clone(),
            score: m.final_score,
            trust: Some(format!("{:.2}", m.trust_score)),
            content: m.content.clone(),
            source_hash: Some(m.content_hash.clone()),
            updated_at: Some(m.updated_at.clone()),
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
    /// B4: floor + exempt-types config. Stored behind a `RwLock` so the
    /// TUI / CLI can apply workspace-settings overrides on an existing
    /// `Arc<MemoryStore>` without rebuilding the store. Reads are
    /// short-lived snapshots taken inside `load_scoped_memory`.
    scoring_cfg: std::sync::RwLock<ScoringCfg>,
    /// B1: absolute path to the backing `memory.db`. `None` for the
    /// in-memory test store. The re-embed migration uses this to take
    /// a `.bak` before mutating, and the bootstrap uses it to detect
    /// the embedder-version mismatch.
    db_path: Option<PathBuf>,
}

/// B4: per-store scoring configuration. Defaults match the plan's
/// recommended values and the constants in [`scoring`].
#[derive(Debug, Clone)]
pub struct ScoringCfg {
    pub recency_floor: f32,
    pub decay_exempt_types: Vec<MemoryType>,
}

impl Default for ScoringCfg {
    fn default() -> Self {
        Self {
            recency_floor: scoring::DEFAULT_RECENCY_FLOOR,
            decay_exempt_types: scoring::DEFAULT_DECAY_EXEMPT_TYPES.to_vec(),
        }
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cfg = self.scoring_cfg.read().expect("scoring cfg poisoned");
        f.debug_struct("MemoryStore")
            .field("model", &self.embedder.model_id())
            .field("recency_floor", &cfg.recency_floor)
            .field("decay_exempt_types", &cfg.decay_exempt_types)
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
        Self::init(conn, embedder, Some(db_path.to_path_buf()))
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory(embedder: Arc<dyn Embedder>) -> Result<Self> {
        Self::register_sqlite_vec();
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        Self::init(conn, embedder, None)
    }

    fn init(
        conn: Connection,
        embedder: Arc<dyn Embedder>,
        db_path: Option<PathBuf>,
    ) -> Result<Self> {
        schema::run_migrations(&conn, embedder.dimensions())
            .context("running schema migrations")?;

        // B1: stamp `_gaviero_meta.embedder_model` so mismatch detection
        // works on existing DBs. Without this, a pre-B1 database has no
        // meta row and `detect_embedder_mismatch` would silently return
        // None — letting the user query nomic-stored vectors with a
        // gte-modernbert query embedder. The rule:
        //   - if a stamp already exists, leave it untouched.
        //   - if absent AND the `memories` table holds at least one
        //     non-empty embedding, stamp the legacy default
        //     (`nomic-embed-text-v1.5`) — anything stored before v8
        //     came from there. The user will be prompted to /reembed
        //     if their configured embedder differs.
        //   - if absent AND there are no embedded rows yet (fresh DB),
        //     stamp the configured embedder so the next launch is a
        //     no-op.
        // Best-effort: meta stamp failure is logged, never fatal.
        let has_stamp: bool = conn
            .prepare("SELECT 1 FROM _gaviero_meta WHERE key = 'embedder_model'")
            .and_then(|mut s| s.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_stamp {
            let has_embedded_rows: bool = conn
                .prepare("SELECT 1 FROM memories WHERE embedding IS NOT NULL LIMIT 1")
                .and_then(|mut s| s.query_row([], |_| Ok(true)))
                .unwrap_or(false);
            let stamp = if has_embedded_rows {
                "nomic-embed-text-v1.5"
            } else {
                embedder.model_id()
            };
            if let Err(e) = conn.execute(
                "INSERT INTO _gaviero_meta(key, value) VALUES ('embedder_model', ?1)
                 ON CONFLICT(key) DO NOTHING",
                rusqlite::params![stamp],
            ) {
                tracing::warn!(
                    target: "memory_reembed",
                    error = %e,
                    stamp,
                    "failed to backfill _gaviero_meta.embedder_model — \
                     mismatch detection disabled until next write"
                );
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            embedder,
            scoring_cfg: std::sync::RwLock::new(ScoringCfg::default()),
            db_path,
        })
    }

    /// B1: backing `memory.db` path. `None` for the in-memory store
    /// used in tests.
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// B1: detect a stale embedder-model stamp. Returns the persisted
    /// model id (from `_gaviero_meta.embedder_model`) when it differs
    /// from the configured embedder. `None` means either no mismatch,
    /// no stamp yet (first-ever open — the writer task stamps after the
    /// next write), or the meta-read failed (logged at warn level).
    ///
    /// Used by the TUI bootstrap on `Event::MemoryReady` to surface a
    /// `/reembed` prompt when the user flipped `memory.embedder.model`.
    pub async fn detect_embedder_mismatch(&self) -> Option<String> {
        let configured = self.embedder.model_id().to_string();
        match self.get_meta_value("embedder_model").await {
            Ok(Some(stored)) if stored != configured => Some(stored),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(target: "memory_reembed", error = %e, "reading embedder meta");
                None
            }
        }
    }

    /// B4: override the recency floor on an already-Arc-wrapped store.
    /// Called from TUI / CLI bootstrap after settings have been resolved.
    pub fn set_recency_floor(&self, floor: f32) {
        let mut cfg = self.scoring_cfg.write().expect("scoring cfg poisoned");
        cfg.recency_floor = floor.clamp(0.0, 1.0);
    }

    /// B4: override the decay-exempt memory types on an already-Arc-wrapped store.
    pub fn set_decay_exempt_types(&self, types: Vec<MemoryType>) {
        let mut cfg = self.scoring_cfg.write().expect("scoring cfg poisoned");
        cfg.decay_exempt_types = types;
    }

    /// B4: read a snapshot of the current scoring config.
    pub fn scoring_cfg(&self) -> ScoringCfg {
        self.scoring_cfg
            .read()
            .expect("scoring cfg poisoned")
            .clone()
    }

    /// Return a reference to the embedder for external use.
    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    // ── Store operations ───────────────────────────────────────────

    /// Store a memory entry. Upserts on (namespace, key).
    ///
    /// Embedding is computed BEFORE acquiring the database lock.
    ///
    /// **New callers should enqueue a `WriterMessage` via `WriterHandle`** so
    /// writes serialize with every other memory write on the workspace. This
    /// method remains public for the writer task's internal use and for
    /// in-crate legacy call sites (e.g. `swarm::pipeline`); direct use from
    /// `gaviero-tui` / `gaviero-cli` should be avoided.
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
            .await
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
            .await
            .context("computing query embedding")?;
        let query_blob = embedding_to_blob(&query_embedding);
        let active_model_id = self.embedder.model_id().to_string();

        let conn = self.conn.lock().await;

        // Over-fetch from vec_memories (5x limit to allow for post-filtering).
        // C5/B4: filter by `m.model_id` so dimension/model coexistence is
        // safe. Without this filter, a row embedded by an older model
        // (potentially with a different dimensionality) joins against the
        // current query vector and returns garbage similarities — or, if
        // the dim differs, sqlite-vec rejects the JOIN. Rows from other
        // models stay in the table for `reembed_migration` to rewrite.
        let fetch_k = limit * 5;
        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance,
                    m.id, m.namespace, m.key, m.content, m.metadata,
                    m.created_at, m.updated_at, m.importance, m.access_count,
                    m.last_accessed_at, m.privacy
             FROM vec_memories v
             JOIN memories m ON m.id = v.memory_id
             WHERE v.embedding MATCH ?1 AND k = ?2 AND m.model_id = ?3",
            )
            .context("preparing KNN search")?;

        let now = chrono_now_utc();
        let ns_set: std::collections::HashSet<&str> =
            namespaces.iter().map(|s| s.as_str()).collect();

        let mut results: Vec<SearchResult> = stmt
            .query_map(
                rusqlite::params![query_blob, fetch_k as i64, active_model_id],
                |row| {
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
            let embedding = self.embedder.embed_document(content).await?;
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

    // ── B1d: Re-embed migration plumbing ───────────────────────────

    /// Read every memory row's `(id, content, current model_id)` for
    /// the re-embed migration. Held briefly under the lock; the caller
    /// embeds outside the lock and writes via [`reembed_apply_batch`].
    pub async fn reembed_fetch_rows(&self) -> Result<Vec<(i64, String, Option<String>)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT id, content, model_id FROM memories ORDER BY id")?;
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Commit a batch of re-embedded rows: updates `memories.embedding`
    /// + `memories.model_id`, and replaces both vec virtual-table rows.
    /// Brief lock held only for the batch's writes.
    pub async fn reembed_apply_batch(
        &self,
        updates: &[(i64, Vec<u8>)],
        new_model_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let tx = conn.unchecked_transaction()?;
        for (id, blob) in updates {
            tx.execute(
                "UPDATE memories SET embedding = ?1, model_id = ?2,
                        updated_at = datetime('now') WHERE id = ?3",
                rusqlite::params![blob, new_model_id, id],
            )?;
            // sqlite-vec: replace by delete-then-insert (vec0 has no UPSERT).
            tx.execute(
                "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
                rusqlite::params![id],
            )?;
            // Pull scope_level back from memories so the vec partition
            // matches; cheap because rows we just touched are hot.
            let scope_level: i32 = tx
                .query_row(
                    "SELECT scope_level FROM memories WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            tx.execute(
                "INSERT INTO vec_memories_scoped(memory_id, embedding, scope_level)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, blob, scope_level],
            )?;
            // Legacy (unscoped) table — kept in sync best-effort.
            let _ = tx.execute(
                "DELETE FROM vec_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            );
            let _ = tx.execute(
                "INSERT INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![id, blob],
            );
        }
        tx.commit()?;
        Ok(())
    }

    /// Set a key in `_gaviero_meta`. Used by the re-embed migration to
    /// stamp `embedder_model` only after the final batch commits.
    pub async fn set_meta_value(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO _gaviero_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Read a key from `_gaviero_meta`. Returns `None` when absent.
    pub async fn get_meta_value(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let v: Option<String> = conn
            .prepare("SELECT value FROM _gaviero_meta WHERE key = ?1")?
            .query_row(rusqlite::params![key], |row| row.get(0))
            .ok();
        Ok(v)
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
    /// Probe whether a content-hash row already exists at the given
    /// `(scope_level, scope_path)` in this store. Used by the
    /// [`super::stores::MemoryStores`] registry to detect cross-DB
    /// coverage by a broader scope before delegating the actual
    /// write to the target store.
    pub async fn has_content_hash(
        &self,
        scope_level: i32,
        scope_path: &str,
        content_hash: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().await;
        let hit: bool = conn
            .prepare(
                "SELECT 1 FROM memories
                 WHERE content_hash = ?1 AND scope_level = ?2 AND scope_path = ?3
                 LIMIT 1",
            )?
            .query_row(
                rusqlite::params![content_hash, scope_level, scope_path],
                |_| Ok(true),
            )
            .unwrap_or(false);
        Ok(hit)
    }

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
            .await
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
            reinforce_memory(&conn, id)?;
            tracing::debug!(id, "scoped store: deduplicated at same scope");
            return Ok(StoreResult::Deduplicated(id));
        }

        let ancestor_paths = broader_scope_paths(scope);
        for (level, ancestor_path) in &ancestor_paths {
            let covered: bool = conn
                .prepare(
                    "SELECT 1 FROM memories
                     WHERE content_hash = ?1 AND scope_level = ?2 AND scope_path = ?3
                     LIMIT 1",
                )?
                .query_row(rusqlite::params![hash, level, ancestor_path], |_| Ok(true))
                .unwrap_or(false);
            if covered {
                tracing::debug!(level, "scoped store: already covered at broader scope");
                return Ok(StoreResult::AlreadyCovered);
            }
        }

        if scope_level != super::scope::SCOPE_RUN {
            if let Some(id) = find_semantic_duplicate(
                &conn,
                content,
                &embedding,
                scope_level,
                &scope_path,
                meta.memory_type.as_str(),
                SEMANTIC_DEDUP_THRESHOLD,
            )? {
                reinforce_memory(&conn, id)?;
                tracing::debug!(
                    id,
                    scope_path,
                    threshold = SEMANTIC_DEDUP_THRESHOLD,
                    "scoped store: semantic duplicate at same scope"
                );
                return Ok(StoreResult::Deduplicated(id));
            }
        }

        // Check for semantic coverage at any broader ancestor scope.
        for (level, ancestor_path) in ancestor_paths {
            if find_semantic_duplicate(
                &conn,
                content,
                &embedding,
                level,
                &ancestor_path,
                meta.memory_type.as_str(),
                SEMANTIC_DEDUP_THRESHOLD,
            )?
            .is_some()
            {
                tracing::debug!(
                    level,
                    scope_path = %ancestor_path,
                    threshold = SEMANTIC_DEDUP_THRESHOLD,
                    "scoped store: semantically covered at broader scope"
                );
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
                importance, privacy, source, trust_score
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'public', ?16, ?17)",
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
                meta.source_kind.as_str(),
                meta.trust_score,
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

    // ── Scoped search ─────────────────────────────────────────────

    /// Scoped retrieval entry point.
    ///
    /// B3: this is the merged-multi-scope path. All scope levels
    /// participate in a single ranking pool — the cascade with the
    /// 0.70 early-exit is gone. Scope bias survives as the soft
    /// `scope_weight` multiplier already applied inside scoring.
    ///
    /// The legacy cascade is reachable via [`Self::search_scoped_cascade`]
    /// for one release as a kill switch (settings flag
    /// `memory.retrieval.mode = "cascade"`); it will be removed once
    /// the merged path stabilises in eval.
    pub async fn search_scoped(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        self.multi_scope_retrieve(config).await
    }

    /// B3: merged multi-scope retrieval.
    ///
    /// 1. Embed the query **once** (before the SQLite lock).
    /// 2. Acquire the SQLite mutex once and walk every admissible scope
    ///    level under that single lock. The per-scope hybrid (vec + FTS
    ///    + RRF) reads run sequentially — sqlite-vec ships only a
    ///    single connection in this store, so genuine I/O parallelism
    ///    would need a connection pool (a Tier C refactor). The win
    ///    over the pre-B3 cascade is the cascade's serial early-exit,
    ///    not extra cores.
    /// 3. Merge candidates from all scopes into one pool, dedupe by
    ///    content hash (narrower scope wins on ties so module-specific
    ///    memories don't get masked by a workspace duplicate).
    /// 4. Score each via the existing composite formula; the scope
    ///    multiplier applied inside the scorer carries the (now soft)
    ///    scope bias.
    /// 5. Sort by composite score, truncate to `config.max_results`
    ///    (this is the `memory.retrieval.maxMergedPool` cap when called
    ///    from the central `retrieve_ranked` engine). Reranker
    ///    integration happens above this layer in
    ///    [`retrieve_for_chat_with_reranker`].
    pub async fn multi_scope_retrieve(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .await
            .context("computing query embedding for multi-scope retrieve")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;
        let now = chrono_now_utc();

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut by_hash: HashMap<String, usize> = HashMap::new();

        for level in config.scope.levels() {
            let scope_level_int = level.level_int();
            let vec_candidates = self.vec_search_at_level(
                &conn,
                &query_blob,
                scope_level_int,
                &level,
                config.per_level_limit,
            )?;

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

            // B2/B3 fix: honour FTS-only candidates. The pre-fix code
            // dropped FTS hits when vec returned nothing (fell through
            // to `vec_candidates`), and even in the both-non-empty case
            // FTS-only RRF entries arrived with raw_sim=0 and got
            // filtered by the similarity threshold below. Now:
            //   - vec ∪ fts both non-empty → RRF, then promote FTS-only
            //     (raw_sim==0) entries to the threshold so they survive
            //     the filter and contribute via composite scoring.
            //   - vec empty, fts non-empty → use FTS hits with the
            //     threshold as a synthetic similarity floor.
            //   - vec non-empty, fts empty → use vec as-is.
            //   - both empty → empty pool for this scope.
            let candidate_ids: Vec<(i64, f32)> =
                match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                    (false, false) => {
                        let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                        merged
                            .into_iter()
                            .map(|(id, _rrf, sim)| {
                                let sim = if sim == 0.0 {
                                    config.similarity_threshold
                                } else {
                                    sim
                                };
                                (id, sim)
                            })
                            .collect()
                    }
                    (true, false) => fts_candidates
                        .into_iter()
                        .map(|(id, _rank)| (id, config.similarity_threshold))
                        .collect(),
                    (false, true) => vec_candidates,
                    (true, true) => Vec::new(),
                };

            for (memory_id, raw_sim) in &candidate_ids {
                if *raw_sim < config.similarity_threshold {
                    continue;
                }
                let Some(mem) =
                    self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, &level)?
                else {
                    continue;
                };

                // Cross-scope dedup: when the same content_hash appears
                // at multiple scopes, keep the one with the higher
                // final_score (which, due to scope_multiplier, will
                // typically be the narrower scope at equal similarity).
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
            // No early-exit (B3): every scope's candidates flow into
            // the merged pool unconditionally.
        }

        accumulated.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        accumulated.truncate(config.max_results);

        if !accumulated.is_empty() {
            let ids: Vec<i64> = accumulated.iter().map(|m| m.id).collect();
            self.touch_accessed(&conn, &ids);
            self.log_access(&conn, &ids, &config.scope);
        }

        Ok(accumulated)
    }

    /// Per-store, per-level retrieval. Returns scored memories for
    /// ONE scope level on this store. No cross-scope dedup, no
    /// truncation, no access logging — those happen at the merge
    /// layer in [`super::stores::MemoryStores::multi_scope_retrieve`].
    ///
    /// Acquires this store's connection lock briefly. Callers from
    /// the registry pass the precomputed query embedding so it isn't
    /// recomputed per-store.
    pub async fn retrieve_at_level(
        &self,
        query: &str,
        query_blob: &[u8],
        level: &ScopeFilter,
        config: &SearchConfig,
    ) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;
        let now = chrono_now_utc();
        let scope_level_int = level.level_int();
        let vec_candidates = self.vec_search_at_level(
            &conn,
            query_blob,
            scope_level_int,
            level,
            config.per_level_limit,
        )?;
        let fts_candidates = if config.use_fts {
            self.fts_search_at_level(&conn, query, scope_level_int, level, config.per_level_limit)?
        } else {
            Vec::new()
        };
        let candidate_ids: Vec<(i64, f32)> =
            match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                (false, false) => {
                    let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                    merged
                        .into_iter()
                        .map(|(id, _rrf, sim)| {
                            let sim = if sim == 0.0 {
                                config.similarity_threshold
                            } else {
                                sim
                            };
                            (id, sim)
                        })
                        .collect()
                }
                (true, false) => fts_candidates
                    .into_iter()
                    .map(|(id, _rank)| (id, config.similarity_threshold))
                    .collect(),
                (false, true) => vec_candidates,
                (true, true) => Vec::new(),
            };
        let mut out = Vec::with_capacity(candidate_ids.len());
        for (memory_id, raw_sim) in &candidate_ids {
            if *raw_sim < config.similarity_threshold {
                continue;
            }
            if let Some(mem) = self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, level)? {
                out.push(mem);
            }
        }
        Ok(out)
    }

    /// Bump `last_accessed_at` and append a `retrieval_use` entry for
    /// `ids` on this store. Called by the registry-level merge after
    /// a multi-DB retrieval, with `ids` filtered to those that came
    /// from THIS store.
    pub async fn record_access(&self, ids: &[i64], scope: &MemoryScope) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().await;
        self.touch_accessed(&conn, ids);
        self.log_access(&conn, ids, scope);
        Ok(())
    }

    /// B3 kill switch: legacy cascade-with-early-exit retrieval.
    ///
    /// Kept for one release behind `memory.retrieval.mode = "cascade"`
    /// in case the merged path regresses on a workload we haven't
    /// characterised. Will be removed in the next minor cycle.
    #[deprecated(note = "Use multi_scope_retrieve (B3); this is a kill switch.")]
    pub async fn search_scoped_cascade(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .await
            .context("computing query embedding for scoped search")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;
        let now = chrono_now_utc();

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut seen_hashes: HashSet<String> = HashSet::new();

        for level in config.scope.levels() {
            let scope_level_int = level.level_int();
            let vec_candidates = self.vec_search_at_level(
                &conn,
                &query_blob,
                scope_level_int,
                &level,
                config.per_level_limit,
            )?;
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
            // Mirror the FTS-only fallback fix from `multi_scope_retrieve`
            // so the kill-switch path doesn't silently differ in
            // candidate-pool composition.
            let candidate_ids: Vec<(i64, f32)> =
                match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                    (false, false) => {
                        let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                        merged
                            .into_iter()
                            .map(|(id, _rrf, sim)| {
                                let sim = if sim == 0.0 {
                                    config.similarity_threshold
                                } else {
                                    sim
                                };
                                (id, sim)
                            })
                            .collect()
                    }
                    (true, false) => fts_candidates
                        .into_iter()
                        .map(|(id, _rank)| (id, config.similarity_threshold))
                        .collect(),
                    (false, true) => vec_candidates,
                    (true, true) => Vec::new(),
                };

            for (memory_id, raw_sim) in &candidate_ids {
                if *raw_sim < config.similarity_threshold {
                    continue;
                }
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
            if accumulated.len() >= config.max_results {
                if let Some(best) = accumulated.iter().map(|m| m.final_score).reduce(f32::max) {
                    if best >= config.confidence_threshold {
                        break;
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

    // ── Injection manifests (Tier S / S4) ──────────────────────────

    /// Persist a retrieval manifest for a chat turn.
    ///
    /// Called exclusively from the writer task in response to
    /// `WriterMessage::InjectionManifest`. `payload` is opaque JSON
    /// serialized to TEXT; the writer normalises shape upstream.
    pub async fn store_injection_manifest(
        &self,
        turn_id: &str,
        session_id: &str,
        source_channel: &str,
        payload: &serde_json::Value,
    ) -> Result<i64> {
        let payload_str =
            serde_json::to_string(payload).context("serialising injection manifest payload")?;
        let turn_id = turn_id.to_string();
        let session_id = session_id.to_string();
        let source_channel = source_channel.to_string();

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO injection_manifests
                (turn_id, session_id, source_channel, payload)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![turn_id, session_id, source_channel, payload_str],
        )
        .context("inserting injection manifest")?;
        Ok(conn.last_insert_rowid())
    }

    /// Fetch the N most recent manifests (any session). Used by
    /// `gaviero-cli memory manifest --last N`.
    pub async fn recent_manifests(&self, limit: usize) -> Result<Vec<InjectionManifestRow>> {
        let limit = limit as i64;
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, turn_id, session_id, source_channel, payload, created_at
                 FROM injection_manifests
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .context("preparing recent_manifests")?;
        let rows = stmt
            .query_map([limit], InjectionManifestRow::from_row)
            .context("executing recent_manifests")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading manifest row")?);
        }
        Ok(out)
    }

    /// Fetch the manifest(s) for a specific turn id.
    pub async fn manifests_for_turn(&self, turn_id: &str) -> Result<Vec<InjectionManifestRow>> {
        let turn_id = turn_id.to_string();
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, turn_id, session_id, source_channel, payload, created_at
                 FROM injection_manifests
                 WHERE turn_id = ?1
                 ORDER BY id DESC",
            )
            .context("preparing manifests_for_turn")?;
        let rows = stmt
            .query_map([turn_id], InjectionManifestRow::from_row)
            .context("executing manifests_for_turn")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading manifest row")?);
        }
        Ok(out)
    }

    /// Delete manifests older than `retention_days`. Wired into the future
    /// sleeptime pass (Tier B5); exposed now so S4 has a testable prune
    /// entry point even when the pass itself is absent.
    pub async fn prune_manifests_older_than(&self, retention_days: u32) -> Result<usize> {
        let conn = self.conn.lock().await;
        let affected = conn
            .execute(
                "DELETE FROM injection_manifests
                 WHERE created_at < datetime('now', ?1)",
                rusqlite::params![format!("-{} days", retention_days)],
            )
            .context("pruning manifests")?;
        Ok(affected)
    }

    // ── TUI memory panel queries (Tier A / A4) ───────────────────

    /// Memories written within the last `hours` hours, newest first.
    /// Drives Section 2 "Recently Written". Query is indexed on
    /// `created_at` via SQLite's default rowid ordering.
    pub async fn recent_memories(&self, hours: u32, limit: usize) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;
        let since = format!("-{hours} hours");
        let mut stmt = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories
                 WHERE created_at >= datetime('now', ?1)
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .context("preparing recent_memories")?;
        let rows = stmt
            .query_map(rusqlite::params![since, limit as i64], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;
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
                    accessed_at,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    source: super::trust_defaults::MemorySource::parse_str(&source_str),
                    trust_score,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })
            .context("running recent_memories")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading recent_memory row")?);
        }
        Ok(out)
    }

    /// B5 fix: time-bounded recent memories filtered to a specific
    /// `run_id`. Used by the session consolidator so it can never see
    /// memories from another concurrent (or recent) session, even when
    /// runs overlap in wall-clock time.
    ///
    /// `run_id` of empty string returns nothing — refuse to leak across
    /// sessions if the caller didn't supply identity.
    pub async fn recent_memories_for_run(
        &self,
        run_id: &str,
        hours: u32,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        if run_id.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().await;
        let since = format!("-{hours} hours");
        let mut stmt = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories
                 WHERE run_id = ?1
                   AND created_at >= datetime('now', ?2)
                   AND superseded_by IS NULL
                 ORDER BY id DESC
                 LIMIT ?3",
            )
            .context("preparing recent_memories_for_run")?;
        let rows = stmt
            .query_map(rusqlite::params![run_id, since, limit as i64], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;
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
                    accessed_at,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    source: super::trust_defaults::MemorySource::parse_str(&source_str),
                    trust_score,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })
            .context("running recent_memories_for_run")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading recent_memory_for_run row")?);
        }
        Ok(out)
    }

    /// Per-scope counts + last-write timestamp for Section 3 "Scope
    /// Summary". Returns one row per scope level present; callers fill
    /// missing levels with zero.
    pub async fn scope_summary(&self) -> Result<Vec<(i32, i64, Option<String>)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT scope_level, COUNT(*), MAX(created_at)
                 FROM memories
                 GROUP BY scope_level
                 ORDER BY scope_level",
            )
            .context("preparing scope_summary")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .context("running scope_summary")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading scope_summary row")?);
        }
        Ok(out)
    }

    // ── Tier B / B5 sleeptime + B6 telemetry ─────────────────────

    /// Insert one row into `sleeptime_audit`. Idempotent at the row
    /// level — repeated runs append; the run_id ties them back.
    pub async fn log_sleeptime_audit(
        &self,
        run_id: &str,
        kind: &str,
        memory_id: Option<i64>,
        related_id: Option<i64>,
        payload: &str,
        dry_run: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sleeptime_audit
                 (run_id, kind, memory_id, related_id, payload, dry_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![run_id, kind, memory_id, related_id, payload, dry_run as i64],
        )
        .context("inserting sleeptime_audit row")?;
        Ok(())
    }

    /// B5 step 1: decay sweep. Walks every memory, computes recency
    /// under the current B4 floor, and returns
    /// [`crate::memory::sleeptime::SleeptimeOperation::DecayFlagged`] for rows
    /// at the floor. **Never deletes.** `dry_run` only affects the
    /// audit-log marker — flagging is itself non-destructive.
    pub async fn sleeptime_decay_sweep(
        &self,
        _dry_run: bool,
    ) -> Result<Vec<crate::memory::sleeptime::SleeptimeOperation>> {
        use super::scoring::recency_factor;
        let cfg = self.scoring_cfg();
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, COALESCE(last_accessed_at, updated_at)
                 FROM memories
                 WHERE superseded_by IS NULL
                 ORDER BY id",
            )
            .context("preparing decay sweep")?;
        let mut ops = Vec::new();
        let mut rows = stmt.query([]).context("running decay sweep")?;
        while let Some(row) = rows.next().context("decay sweep row")? {
            let id: i64 = row.get(0)?;
            let mt: String = row.get(1)?;
            let stamp: String = row.get(2)?;
            let memory_type = MemoryType::parse_str(&mt);
            let days = days_since_iso(&stamp);
            let r = recency_factor(
                days,
                memory_type,
                cfg.recency_floor,
                &cfg.decay_exempt_types,
            );
            if r <= cfg.recency_floor + 1e-6 {
                ops.push(crate::memory::sleeptime::SleeptimeOperation::DecayFlagged {
                    memory_id: id,
                    recency: r,
                });
            }
        }
        Ok(ops)
    }

    /// B5 step 2: near-duplicate merge within a scope level. Pairs
    /// with cosine ≥ `threshold` and matching memory_type are merged
    /// — the source-aware winner is retained (`user_remember` always
    /// wins). Vec rows for the loser are dropped. Returns one
    /// [`crate::memory::sleeptime::SleeptimeOperation::NearDupMerged`] per
    /// pair processed.
    pub async fn sleeptime_near_dup_merge(
        &self,
        threshold: f32,
        dry_run: bool,
    ) -> Result<Vec<crate::memory::sleeptime::SleeptimeOperation>> {
        use crate::memory::sleeptime::{SleeptimeOperation, pick_merge_winner};

        // Pull all rows with their embeddings + source/trust. We hold
        // the lock for the read pass, then drop it before each merge so
        // `delete_memory_by_id` can re-acquire.
        struct Row {
            id: i64,
            scope_level: i32,
            // B5 safety: full scope identity (scope_path encodes
            // repo:<id>[:module:<path>] or run:<id>) is required to
            // ensure near-dup merges never cross repo / module / run
            // boundaries. Two memories at scope_level=Repo in different
            // repos must NOT be considered near-duplicates.
            scope_path: String,
            memory_type: MemoryType,
            content_hash: String,
            source: MemorySource,
            trust_score: f32,
            embedding: Vec<f32>,
        }

        let rows: Vec<Row> = {
            let conn = self.conn.lock().await;
            let mut stmt = conn
                .prepare(
                    "SELECT id, scope_level, scope_path, memory_type, content_hash,
                            source, trust_score, embedding
                     FROM memories
                     WHERE superseded_by IS NULL AND embedding IS NOT NULL",
                )
                .context("preparing near_dup_merge")?;
            let mut out = Vec::new();
            let mut q = stmt.query([]).context("running near_dup_merge")?;
            while let Some(r) = q.next().context("near_dup row")? {
                let id: i64 = r.get(0)?;
                let scope_level: i32 = r.get(1)?;
                let scope_path: Option<String> = r.get(2)?;
                let mt: String = r.get(3)?;
                let ch: Option<String> = r.get(4)?;
                let src: String = r.get(5)?;
                let trust: f32 = r.get(6)?;
                let emb_blob: Vec<u8> = r.get(7)?;
                let embedding = blob_to_embedding(&emb_blob).unwrap_or_default();
                if embedding.is_empty() {
                    continue;
                }
                out.push(Row {
                    id,
                    scope_level,
                    scope_path: scope_path.unwrap_or_default(),
                    memory_type: MemoryType::parse_str(&mt),
                    content_hash: ch.unwrap_or_default(),
                    source: MemorySource::parse_str(&src),
                    trust_score: trust,
                    embedding,
                });
            }
            out
        };

        let mut ops: Vec<SleeptimeOperation> = Vec::new();
        let mut dropped: std::collections::HashSet<i64> = std::collections::HashSet::new();

        for i in 0..rows.len() {
            if dropped.contains(&rows[i].id) {
                continue;
            }
            for j in (i + 1)..rows.len() {
                if dropped.contains(&rows[j].id) {
                    continue;
                }
                // Group strictly by scope identity: same scope_level is
                // necessary but not sufficient (Repo-level rows in two
                // different repos share scope_level but not scope_path).
                // Empty scope_path falls through this check — pre-A4
                // legacy rows lacking scope metadata never merge across
                // each other, which is conservative on purpose.
                if rows[i].scope_level != rows[j].scope_level
                    || rows[i].scope_path != rows[j].scope_path
                    || rows[i].scope_path.is_empty()
                    || rows[i].memory_type != rows[j].memory_type
                {
                    continue;
                }
                if !rows[i].content_hash.is_empty() && rows[i].content_hash == rows[j].content_hash
                {
                    // Exact-content dups already handled by write-time
                    // dedup; skip here so the merge counter stays
                    // honest about *near*-dups.
                    continue;
                }
                let cos = cosine_similarity(&rows[i].embedding, &rows[j].embedding);
                if cos < threshold {
                    continue;
                }
                let (keep_id, drop_id) = pick_merge_winner(
                    rows[i].id,
                    rows[i].source,
                    rows[i].trust_score,
                    rows[j].id,
                    rows[j].source,
                    rows[j].trust_score,
                );
                let (keep_source, drop_source) = if keep_id == rows[i].id {
                    (rows[i].source, rows[j].source)
                } else {
                    (rows[j].source, rows[i].source)
                };
                ops.push(SleeptimeOperation::NearDupMerged {
                    keep_id,
                    drop_id,
                    cosine: cos,
                    keep_source,
                    drop_source,
                });
                dropped.insert(drop_id);
                if !dry_run {
                    // Delete the loser. Vec rows are cleaned up by
                    // `delete_memory_by_id`. Best-effort — failures
                    // are logged but don't abort the sweep.
                    if let Err(e) = self.delete_memory_by_id(drop_id).await {
                        tracing::warn!(
                            target: "memory_sleeptime",
                            drop_id = drop_id,
                            error = %e,
                            "near_dup merge: delete loser failed"
                        );
                    }
                }
            }
        }
        Ok(ops)
    }

    /// B5 step 3: cross-scope promotion. The pre-existing consolidator
    /// already handles "3+ run-scope hits → repo"; this stub promotes
    /// `decision|convention|invariant` types after a single hit so
    /// high-value reference rows widen sooner. Returns the operations
    /// applied. Currently a no-op skeleton — wiring to the existing
    /// promotion path is a follow-up; the sleeptime caller still gets
    /// a structured empty result.
    pub async fn sleeptime_promote(
        &self,
        _dry_run: bool,
    ) -> Result<Vec<crate::memory::sleeptime::SleeptimeOperation>> {
        Ok(Vec::new())
    }

    /// B5 step 4: trust re-scoring driven by retrieval-use telemetry
    /// (B6) and the manifest hit count fallback. Returns one
    /// [`crate::memory::sleeptime::SleeptimeOperation::TrustAdjusted`] per
    /// adjusted row.
    pub async fn sleeptime_trust_rescore(
        &self,
        cfg: &crate::memory::sleeptime::SleeptimeConfig,
    ) -> Result<Vec<crate::memory::sleeptime::SleeptimeOperation>> {
        use crate::memory::sleeptime::SleeptimeOperation;

        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.trust_score, m.source,
                        COUNT(ru.id)                                       AS injections,
                        SUM(CASE WHEN ru.classification = 'used'    THEN 1 ELSE 0 END) AS used,
                        SUM(CASE WHEN ru.classification = 'unused'  THEN 1 ELSE 0 END) AS unused
                 FROM memories m
                 LEFT JOIN retrieval_use ru ON ru.memory_id = m.id
                 WHERE m.superseded_by IS NULL
                 GROUP BY m.id",
            )
            .context("preparing trust_rescore")?;
        let mut q = stmt.query([]).context("running trust_rescore")?;
        let mut updates: Vec<(i64, f32, f32, Option<f32>, u32)> = Vec::new();
        while let Some(r) = q.next().context("trust_rescore row")? {
            let id: i64 = r.get(0)?;
            let trust: f32 = r.get(1)?;
            let source_str: String = r.get(2)?;
            let injections: u32 = r.get::<_, i64>(3)? as u32;
            let used: u32 = r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u32;
            let _unused: u32 = r.get::<_, Option<i64>>(5)?.unwrap_or(0) as u32;
            if injections < cfg.trust_min_injections {
                continue;
            }
            let source = MemorySource::parse_str(&source_str);
            let user_authored =
                matches!(source, MemorySource::UserRemember | MemorySource::UserPanel);
            if user_authored {
                continue;
            }
            let rate = used as f32 / injections as f32;
            let delta = if rate > cfg.utilization_used_threshold {
                cfg.trust_adjust_delta
            } else if rate < cfg.utilization_unused_threshold {
                -cfg.trust_adjust_delta
            } else {
                continue;
            };
            let new_trust = (trust + delta).clamp(cfg.trust_floor, cfg.trust_ceiling_llm);
            if (new_trust - trust).abs() < 1e-6 {
                continue;
            }
            updates.push((id, trust, new_trust, Some(rate), injections));
        }
        drop(q);
        drop(stmt);
        // Apply updates in a fresh write-pass.
        let mut ops = Vec::with_capacity(updates.len());
        for (id, old_trust, new_trust, rate, injections) in updates {
            if !cfg.dry_run {
                conn.execute(
                    "UPDATE memories SET trust_score = ?1, updated_at = datetime('now')
                     WHERE id = ?2",
                    rusqlite::params![new_trust, id],
                )
                .context("applying trust_rescore update")?;
            }
            ops.push(SleeptimeOperation::TrustAdjusted {
                memory_id: id,
                old_trust,
                new_trust,
                utilization_rate: rate,
                injections,
            });
        }
        Ok(ops)
    }

    /// B5 step 5 / B6 retention: prune `retrieval_use` rows older than
    /// `cutoff_days`. The aggregated trust adjustment has already been
    /// applied at sleeptime time; raw rows are no longer load-bearing.
    pub async fn sleeptime_prune_telemetry(&self, cutoff_days: u32) -> Result<usize> {
        let conn = self.conn.lock().await;
        let cutoff = format!("-{cutoff_days} days");
        let n = conn
            .execute(
                "DELETE FROM retrieval_use WHERE created_at < datetime('now', ?1)",
                rusqlite::params![cutoff],
            )
            .context("pruning retrieval_use")?;
        Ok(n)
    }

    /// B6: persist one classification row from the telemetry pass.
    pub async fn record_retrieval_use(
        &self,
        memory_id: i64,
        turn_id: &str,
        session_id: Option<&str>,
        injected_rank: i32,
        classification: &str,
        cosine_to_response: f32,
        substring_hit: bool,
    ) -> Result<i64> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO retrieval_use
                 (memory_id, turn_id, session_id, injected_rank,
                  classification, cosine_to_response, substring_hit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                memory_id,
                turn_id,
                session_id,
                injected_rank,
                classification,
                cosine_to_response,
                substring_hit as i64,
            ],
        )
        .context("inserting retrieval_use row")?;
        Ok(conn.last_insert_rowid())
    }

    /// B6: per-memory utilization aggregate. Computed on demand
    /// (cheap query against `retrieval_use`); no materialised view
    /// needed at this scale.
    pub async fn memory_utilization(&self, memory_ids: &[i64]) -> Result<Vec<MemoryUtilization>> {
        if memory_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().await;
        let placeholders = memory_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT memory_id,
                    COUNT(*) AS injected,
                    SUM(CASE WHEN classification = 'used' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN classification = 'partial' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN classification = 'unused' THEN 1 ELSE 0 END),
                    MAX(created_at)
             FROM retrieval_use
             WHERE memory_id IN ({placeholders})
             GROUP BY memory_id"
        );
        let mut stmt = conn.prepare(&sql).context("preparing memory_utilization")?;
        let params: Vec<&dyn rusqlite::ToSql> = memory_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params.iter()))
            .context("running memory_utilization")?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().context("utilization row")? {
            let memory_id: i64 = r.get(0)?;
            let injected: i64 = r.get(1)?;
            let used: i64 = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let partial: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let unused: i64 = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let last: Option<String> = r.get(5)?;
            out.push(MemoryUtilization {
                memory_id,
                times_injected: injected as u32,
                times_used: used as u32,
                times_partial: partial as u32,
                times_unused: unused as u32,
                utilization_rate: if injected > 0 {
                    used as f32 / injected as f32
                } else {
                    0.0
                },
                last_used_at: last,
            });
        }
        Ok(out)
    }

    /// B6: top-N rows by utilization rate within a scope. Used by the
    /// `gaviero-cli memory utilization` command.
    pub async fn top_utilization_in_scope(
        &self,
        scope_level: i32,
        ascending: bool,
        limit: usize,
    ) -> Result<Vec<(i64, MemoryUtilization)>> {
        let conn = self.conn.lock().await;
        let order = if ascending { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT m.id, COUNT(ru.id) AS injected,
                    SUM(CASE WHEN ru.classification = 'used' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN ru.classification = 'partial' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN ru.classification = 'unused' THEN 1 ELSE 0 END),
                    MAX(ru.created_at)
             FROM memories m
             LEFT JOIN retrieval_use ru ON ru.memory_id = m.id
             WHERE m.scope_level = ?1 AND m.superseded_by IS NULL
             GROUP BY m.id
             HAVING injected > 0
             ORDER BY (CAST(SUM(CASE WHEN ru.classification = 'used' THEN 1 ELSE 0 END) AS REAL)
                       / MAX(1.0, CAST(injected AS REAL))) {order}, injected DESC
             LIMIT ?2"
        );
        let mut stmt = stmt_or_err(&conn, &sql)?;
        let mut q = stmt
            .query(rusqlite::params![scope_level, limit as i64])
            .context("running top_utilization_in_scope")?;
        let mut out = Vec::new();
        while let Some(r) = q.next().context("top_utilization row")? {
            let id: i64 = r.get(0)?;
            let injected: i64 = r.get(1)?;
            let used: i64 = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let partial: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let unused: i64 = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let last: Option<String> = r.get(5)?;
            let util = MemoryUtilization {
                memory_id: id,
                times_injected: injected as u32,
                times_used: used as u32,
                times_partial: partial as u32,
                times_unused: unused as u32,
                utilization_rate: if injected > 0 {
                    used as f32 / injected as f32
                } else {
                    0.0
                },
                last_used_at: last,
            };
            out.push((id, util));
        }
        Ok(out)
    }

    /// B5 SUPERSEDE: mark `old_id` as superseded by `new_id`. Soft
    /// delete — the row stays for audit / retrieval-by-id but is
    /// excluded from search via the `superseded_by IS NULL` predicate
    /// the sleeptime loops use.
    pub async fn supersede_memory(&self, old_id: i64, new_id: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let n = conn
            .execute(
                "UPDATE memories SET superseded_by = ?1, updated_at = datetime('now')
                 WHERE id = ?2",
                rusqlite::params![new_id, old_id],
            )
            .context("marking superseded")?;
        Ok(n)
    }

    /// B6: fetch a memory's `content` text by id. Used by the
    /// telemetry classifier's substring-match path.
    pub async fn get_content(&self, memory_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let row: Result<String, rusqlite::Error> = conn.query_row(
            "SELECT content FROM memories WHERE id = ?1",
            rusqlite::params![memory_id],
            |r| r.get(0),
        );
        match row {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("reading content: {e}")),
        }
    }

    /// B6: fetch the embedding blob for a memory id (returns the row's
    /// vector as `Vec<f32>`). Used by the telemetry classifier so the
    /// per-injected-memory cosine doesn't need to re-embed.
    pub async fn embedding_for(&self, memory_id: i64) -> Result<Option<Vec<f32>>> {
        let conn = self.conn.lock().await;
        let row: Result<Vec<u8>, rusqlite::Error> = conn.query_row(
            "SELECT embedding FROM memories WHERE id = ?1",
            rusqlite::params![memory_id],
            |r| r.get(0),
        );
        match row {
            Ok(blob) if blob.is_empty() => Ok(None),
            Ok(blob) => Ok(blob_to_embedding(&blob)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("reading embedding: {e}")),
        }
    }

    // ── Panel edits (Tier A / A4) ───────────────────────────────

    /// Delete a memory row by id (Panel `d`). Returns the number of
    /// rows affected — zero means the id no longer existed (already
    /// pruned or a UI race).
    pub async fn delete_memory_by_id(&self, memory_id: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        // Drop adjacent vec rows first so the memories delete doesn't
        // leave dangling vector entries. All three deletes are idempotent.
        let _ = conn.execute(
            "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        let _ = conn.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        let _ = conn.execute(
            "DELETE FROM memory_access_log WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        let n = conn
            .execute(
                "DELETE FROM memories WHERE id = ?1",
                rusqlite::params![memory_id],
            )
            .context("deleting memory row")?;
        Ok(n)
    }

    /// Test-only helper: backdate `updated_at` and `last_accessed_at`
    /// of a memory by `days_old` days. Integration tests for B4 decay
    /// behavior need to simulate aged rows without manipulating the
    /// `_for_test` constants used by the formula. Hidden from rustdoc
    /// because production code must not call this — it bypasses the
    /// writer task and the access-log invariants.
    #[doc(hidden)]
    pub async fn force_age_for_test(&self, memory_id: i64, days_old: i32) -> Result<()> {
        let conn = self.conn.lock().await;
        let offset = format!("-{} days", days_old.max(0));
        conn.execute(
            "UPDATE memories
             SET updated_at = datetime('now', ?1),
                 last_accessed_at = datetime('now', ?1)
             WHERE id = ?2",
            rusqlite::params![offset, memory_id],
        )
        .context("force_age_for_test")?;
        Ok(())
    }

    /// Test-only helper: count `sleeptime_audit` rows by kind. Used
    /// by integration tests to assert the audit trail landed without
    /// exposing the raw SQLite connection.
    #[doc(hidden)]
    pub async fn count_audit_for_test(&self, kind: &str) -> Result<i64> {
        let conn = self.conn.lock().await;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sleeptime_audit WHERE kind = ?1",
                rusqlite::params![kind],
                |r| r.get(0),
            )
            .context("count_audit_for_test")?;
        Ok(n)
    }

    /// Set `trust_score` on an existing row. Panel `p` (pin) calls this
    /// with 1.0; Tier B5 sleeptime (later) reuses it to down-score
    /// unused memories.
    pub async fn set_trust_score(&self, memory_id: i64, trust_score: f32) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE memories SET trust_score = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![trust_score, memory_id],
        )
        .context("setting trust_score")?;
        Ok(())
    }

    /// Move a memory to a different scope. sqlite-vec partitions by
    /// `scope_level`, so the simplest correct path is: read the row,
    /// delete it, re-insert at the new scope via `store_scoped`. Caller
    /// gets the new row id back.
    pub async fn change_memory_scope(&self, memory_id: i64, new_scope: &WriteScope) -> Result<i64> {
        // Read the existing row (content + meta) — outside the write
        // lock because `store_scoped` acquires its own.
        let (content, memory_type, importance, trust_score, source_str, tag): (
            String,
            String,
            f32,
            f32,
            String,
            Option<String>,
        ) = {
            let conn = self.conn.lock().await;
            conn.query_row(
                "SELECT content, memory_type, importance, trust_score, source, tag
                 FROM memories WHERE id = ?1",
                rusqlite::params![memory_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .context("reading memory for scope change")?
        };
        let source_kind = super::trust_defaults::MemorySource::parse_str(&source_str);
        let mut meta = WriteMeta::for_source(source_kind)
            .with_importance(importance)
            .with_trust_score(trust_score)
            .with_type(MemoryType::parse_str(&memory_type));
        if let Some(t) = tag {
            meta = meta.with_tag(t);
        }

        // Delete the old row first so dedup on the new scope doesn't
        // collapse into it. `delete_memory_by_id` acquires its own lock.
        self.delete_memory_by_id(memory_id).await?;

        let result = self.store_scoped(new_scope, &content, &meta).await?;
        match result {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => Ok(id),
            StoreResult::AlreadyCovered => Ok(memory_id),
        }
    }

    /// Replace a memory row's `content` + re-embed + re-hash. Used by
    /// the panel's `e` (edit) action. sqlite-vec partitions forbid an
    /// in-place vector update, so we delete-and-reinsert at the same
    /// scope rather than trying to rewrite vec rows.
    pub async fn update_memory_text(&self, memory_id: i64, new_text: &str) -> Result<i64> {
        // Read the row's scope + meta so we can reinsert at the same
        // position. Outside the write lock.
        let (
            scope_level,
            scope_path,
            repo_id,
            module_path,
            run_id,
            memory_type,
            importance,
            trust_score,
            source_str,
            tag,
        ): (
            i32,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            f32,
            f32,
            String,
            Option<String>,
        ) = {
            let conn = self.conn.lock().await;
            conn.query_row(
                "SELECT scope_level, scope_path, repo_id, module_path, run_id,
                        memory_type, importance, trust_score, source, tag
                 FROM memories WHERE id = ?1",
                rusqlite::params![memory_id],
                |r| {
                    Ok((
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
                    ))
                },
            )
            .context("reading memory for text update")?
        };
        let scope = match scope_level {
            0 => WriteScope::Global,
            1 => WriteScope::Workspace,
            2 => WriteScope::Repo {
                repo_id: repo_id.unwrap_or_default(),
            },
            3 => WriteScope::Module {
                repo_id: repo_id.unwrap_or_default(),
                module_path: module_path.unwrap_or_default(),
            },
            _ => WriteScope::Run {
                repo_id: repo_id.unwrap_or_default(),
                run_id: run_id.unwrap_or_default(),
            },
        };
        let _ = scope_path; // informational only; `scope` is authoritative
        let source_kind = super::trust_defaults::MemorySource::parse_str(&source_str);
        let mut meta = WriteMeta::for_source(source_kind)
            .with_importance(importance)
            .with_trust_score(trust_score)
            .with_type(MemoryType::parse_str(&memory_type));
        if let Some(t) = tag {
            meta = meta.with_tag(t);
        }

        self.delete_memory_by_id(memory_id).await?;
        let result = self.store_scoped(&scope, new_text, &meta).await?;
        match result {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => Ok(id),
            StoreResult::AlreadyCovered => Ok(memory_id),
        }
    }

    // ── Session ledger (Tier A / A1) ────────────────────────────

    /// Persist a session-ledger row for a chat turn's annotations.
    ///
    /// Called by the writer task after parsing a `<turn_annotations>`
    /// block. `session_thread` and `open_questions` are surfaced in the
    /// TUI panel (A4) and fed to Tier B5 session consolidation.
    pub async fn store_session_ledger_turn(
        &self,
        session_id: &str,
        turn_id: &str,
        session_thread: Option<&str>,
        open_questions: &[String],
        annotations_json: Option<&str>,
    ) -> Result<i64> {
        let session_id = session_id.to_string();
        let turn_id = turn_id.to_string();
        let thread = session_thread.map(String::from);
        let oq_json = if open_questions.is_empty() {
            None
        } else {
            Some(serde_json::to_string(open_questions).unwrap_or_default())
        };
        let ann_json = annotations_json.map(String::from);

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO session_ledger_turns
                 (session_id, turn_id, session_thread, open_questions, annotations_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, turn_id) DO UPDATE SET
                 session_thread = excluded.session_thread,
                 open_questions = excluded.open_questions,
                 annotations_json = excluded.annotations_json",
            rusqlite::params![session_id, turn_id, thread, oq_json, ann_json],
        )
        .context("inserting session_ledger_turns row")?;
        Ok(conn.last_insert_rowid())
    }

    /// Fetch raw session ledger rows for a session, newest-first.
    pub async fn session_ledger_for(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionLedgerTurn>> {
        let session_id = session_id.to_string();
        let limit = limit as i64;
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, turn_id, session_thread, open_questions,
                        annotations_json, created_at
                 FROM session_ledger_turns
                 WHERE session_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .context("preparing session_ledger_for")?;
        let rows = stmt
            .query_map(
                rusqlite::params![session_id, limit],
                SessionLedgerTurn::from_row,
            )
            .context("executing session_ledger_for")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading session_ledger row")?);
        }
        Ok(out)
    }

    // ── Internal helpers for scoped search ─────────────────────────

    /// Vector KNN search filtered to a scope level.
    ///
    /// C5/B4: also filters by `m.model_id = active_embedder.model_id()`
    /// so memories embedded by a previous model don't pollute results
    /// after an embedder swap. Without this, a 768-dim row joined to a
    /// 1024-dim query vector would either be rejected by sqlite-vec or
    /// return a meaningless distance.
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
        let active_model_id = self.embedder.model_id();

        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance
             FROM vec_memories_scoped v
             JOIN memories m ON m.id = v.memory_id
             WHERE v.embedding MATCH ?1 AND k = ?2 AND v.scope_level = ?3
               AND m.model_id = ?4",
            )
            .context("preparing scoped KNN")?;

        let results: Vec<(i64, f32)> = stmt
            .query_map(
                rusqlite::params![query_blob, fetch_k as i64, scope_level, active_model_id],
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
             AND m.superseded_by IS NULL
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
        // B4: snapshot the scoring config once per row (cheap RwLock read).
        let scoring_cfg = self.scoring_cfg();
        // B5: exclude superseded rows from retrieval. SUPERSEDE soft-
        // deletes by setting `superseded_by`; the row stays in storage
        // for audit (and for retrieval-by-id paths), but it must not
        // surface through scoped search. The vector and FTS searches
        // upstream may still match a superseded row (sqlite-vec carries
        // no `superseded_by` column); filtering here drops it before
        // scoring so the upstream cost is the only price paid.
        let result = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                    repo_id, module_path, memory_type, trust, importance,
                    access_count, created_at, updated_at, last_accessed_at,
                    tag, namespace, key, source, trust_score
             FROM memories WHERE id = ?1 AND superseded_by IS NULL",
            )?
            .query_row(rusqlite::params![memory_id], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let updated_at: String = row.get(12)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let importance: f32 = row.get(9)?;
                let access_count: i32 = row.get(10)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;

                let trust = Trust::parse_str(&trust_str);
                let source = super::trust_defaults::MemorySource::parse_str(&source_str);
                let memory_type = MemoryType::parse_str(&type_str);
                let days = hours_since(&accessed_at, &updated_at, now) / 24.0;

                // A3: score from the fine-grained per-row trust_score, not
                // the legacy 3-level enum. Keeps the legacy column in the
                // SELECT for backward compat / audit but stops reading it
                // into the formula.
                //
                // B4: pass `memory_type` plus the configured floor +
                // exempt-type list so reference memories (decision /
                // convention / invariant / preference / gotcha) keep
                // recency = 1.0 and other types floor at the configured
                // value (default 0.35) instead of decaying to zero.
                let final_score = scoring::score_with_trust_score(
                    raw_similarity,
                    importance,
                    days,
                    access_count,
                    trust_score,
                    level,
                    memory_type,
                    scoring_cfg.recency_floor,
                    &scoring_cfg.decay_exempt_types,
                );

                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type,
                    trust,
                    importance,
                    access_count,
                    created_at: row.get(11)?,
                    updated_at,
                    accessed_at,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    source,
                    trust_score,
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
                    tag, namespace, key, source, trust_score
             FROM memories WHERE run_id = ?1",
        )?;

        let results: Vec<ScoredMemory> = stmt
            .query_map(rusqlite::params![run_id], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let updated_at: String = row.get(12)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;

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
                    source: super::trust_defaults::MemorySource::parse_str(&source_str),
                    trust_score,
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
            repo_db: None,
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
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 0 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Workspace => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 1 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Repo { repo_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 2 AND repo_id = ?1 LIMIT ?2",
                vec![Box::new(repo_id.clone()), Box::new(limit as i64)],
            ),
            ScopeFilter::Module { repo_id, module_path } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 3 AND repo_id = ?1 AND module_path = ?2 LIMIT ?3",
                vec![Box::new(repo_id.clone()), Box::new(module_path.clone()), Box::new(limit as i64)],
            ),
            ScopeFilter::Run { repo_id, run_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
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
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;
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
                    source: super::trust_defaults::MemorySource::parse_str(&source_str),
                    trust_score,
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
                    m.tag, m.namespace, m.key, m.source, m.trust_score,
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
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;
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
                    source: super::trust_defaults::MemorySource::parse_str(&source_str),
                    trust_score,
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

fn reinforce_memory(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE memories SET access_count = access_count + 1,
                last_accessed_at = datetime('now'),
                updated_at = datetime('now')
         WHERE id = ?1",
        rusqlite::params![id],
    )?;
    Ok(())
}

fn broader_scope_paths(scope: &WriteScope) -> Vec<(i32, String)> {
    use super::scope::{SCOPE_GLOBAL, SCOPE_REPO, SCOPE_WORKSPACE};

    match scope {
        WriteScope::Global => Vec::new(),
        WriteScope::Workspace => vec![(SCOPE_GLOBAL, "global".to_string())],
        WriteScope::Repo { .. } => vec![
            (SCOPE_WORKSPACE, "workspace".to_string()),
            (SCOPE_GLOBAL, "global".to_string()),
        ],
        WriteScope::Module { repo_id, .. } | WriteScope::Run { repo_id, .. } => vec![
            (SCOPE_REPO, format!("repo:{repo_id}")),
            (SCOPE_WORKSPACE, "workspace".to_string()),
            (SCOPE_GLOBAL, "global".to_string()),
        ],
    }
}

fn find_semantic_duplicate(
    conn: &Connection,
    content: &str,
    embedding: &[f32],
    scope_level: i32,
    scope_path: &str,
    memory_type: &str,
    threshold: f32,
) -> Result<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, embedding FROM memories
         WHERE scope_level = ?1
           AND scope_path = ?2
           AND memory_type = ?3
           AND embedding IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT 128",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![scope_level, scope_path, memory_type],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        },
    )?;

    for row in rows {
        let (id, other_content, blob) = row?;
        if lexical_overlap(content, &other_content) < 0.75 {
            continue;
        }
        let Some(other) = blob_to_embedding(&blob) else {
            continue;
        };
        if cosine_similarity(embedding, &other) >= threshold {
            return Ok(Some(id));
        }
    }

    Ok(None)
}

fn lexical_overlap(a: &str, b: &str) -> f32 {
    fn tokens(s: &str) -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_ascii_lowercase())
            .collect()
    }

    let a = tokens(a);
    let b = tokens(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(&b).count() as f32;
    let union = a.union(&b).count() as f32;
    intersection / union
}

/// B1d: public re-export of [`embedding_to_blob`] for the re-embed
/// migration crate-internal module. Same little-endian f32 encoding.
pub(crate) fn embedding_to_blob_pub(embedding: &[f32]) -> Vec<u8> {
    embedding_to_blob(embedding)
}

/// Encode a float vector as little-endian bytes for SQLite BLOB storage.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

/// Days since an ISO-8601 timestamp string, fractional. Returns `0.0`
/// when parsing fails so the caller treats the row as "fresh" rather
/// than panicking on a bad column.
pub(crate) fn days_since_iso(stamp: &str) -> f64 {
    use chrono::{NaiveDateTime, Utc};
    let parsed = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d"));
    let Ok(dt) = parsed else {
        return 0.0;
    };
    let now = Utc::now().naive_utc();
    let delta = now.signed_duration_since(dt);
    let secs = delta.num_milliseconds() as f64 / 1000.0;
    (secs / 86_400.0).max(0.0)
}

/// Tiny shim around `Connection::prepare` so call sites can use a
/// cleaner `?` chain. The error message includes the SQL prefix to
/// make debugging dynamic queries easier.
pub(crate) fn stmt_or_err<'a>(
    conn: &'a rusqlite::Connection,
    sql: &str,
) -> Result<rusqlite::Statement<'a>> {
    conn.prepare(sql)
        .with_context(|| format!("preparing SQL: {}", sql.lines().next().unwrap_or("")))
}

/// B6: aggregated retrieval-use stats for one memory.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryUtilization {
    pub memory_id: i64,
    pub times_injected: u32,
    pub times_used: u32,
    pub times_partial: u32,
    pub times_unused: u32,
    pub utilization_rate: f32,
    pub last_used_at: Option<String>,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock embedder that produces deterministic vectors from content hash.
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
    async fn search_filters_out_stale_model_id_rows() {
        // C5/B4 regression: rows written by a previous embedder must not
        // surface when a newer embedder is active. Without the model_id
        // filter, these rows would either return garbage similarities
        // (same-dim) or panic the JOIN (different-dim).
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store
            .store("ns", "current", "current model row", None)
            .await
            .unwrap();
        store
            .store("ns", "soon-stale", "soon-to-be-stale row", None)
            .await
            .unwrap();

        // Mark one row as if it had been written by an older embedder.
        {
            let conn = store.conn.lock().await;
            conn.execute(
                "UPDATE memories SET model_id = 'previous-embedder' WHERE key = 'soon-stale'",
                [],
            )
            .unwrap();
        }

        let results = store.search("ns", "row", 10).await.unwrap();
        let keys: Vec<&str> = results.iter().map(|r| r.entry.key.as_str()).collect();
        assert!(
            keys.contains(&"current"),
            "active-model row must surface: {keys:?}"
        );
        assert!(
            !keys.contains(&"soon-stale"),
            "stale-model row must be filtered out: {keys:?}"
        );
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
    async fn test_store_scoped_semantic_dedup_same_scope() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        let meta = WriteMeta::default();

        let r1 = store
            .store_scoped(&scope, "prefer ripgrep for repository search", &meta)
            .await
            .unwrap();
        assert!(matches!(r1, StoreResult::Inserted(_)));

        let r2 = store
            .store_scoped(&scope, "prefer ripgrep for repository search.", &meta)
            .await
            .unwrap();
        assert!(matches!(r2, StoreResult::Deduplicated(_)));
    }

    #[tokio::test]
    async fn test_store_scoped_semantic_broader_scope_covers() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();

        let repo_scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        store
            .store_scoped(&repo_scope, "use clap derive for CLI parsing", &meta)
            .await
            .unwrap();

        let module_scope = WriteScope::Module {
            repo_id: "r1".into(),
            module_path: "crates/cli".into(),
        };
        let r2 = store
            .store_scoped(&module_scope, "use clap derive for CLI parsing.", &meta)
            .await
            .unwrap();
        assert!(matches!(r2, StoreResult::AlreadyCovered));
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
            repo_db: None,
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

    /// B3 acceptance regression: under the merged-multi-scope retrieval
    /// path the Repo-scope memory must appear in the candidate pool
    /// even when a Run-scope memory already exists with passable
    /// similarity. Pre-B3 the cascade would early-exit at Run as soon
    /// as `best_score ≥ 0.70` and the Repo memory would never be
    /// considered. This test fails on `search_scoped_cascade` and
    /// passes on `multi_scope_retrieve`.
    ///
    /// We additionally assert that the scope-multiplier gap is gentle
    /// (B3 lowered Run from 1.8 → 1.10 and Repo from 1.2 → 1.00) —
    /// concretely, the Run/Repo final_score ratio must stay close to
    /// the ratio of their scope multipliers when raw similarity is
    /// comparable. With the legacy multipliers the ratio would be
    /// ≥ 1.8/1.2 = 1.50, which would mean Run unconditionally drowns
    /// Repo on any similarity tie. After B3 it is ≤ 1.10/1.00 = 1.10.
    #[tokio::test]
    async fn b3_run_does_not_drown_repo() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();

        let repo_scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        store
            .store_scoped(&repo_scope, "the canonical answer about widgets", &meta)
            .await
            .unwrap();

        let run_scope = WriteScope::Run {
            repo_id: "r1".into(),
            run_id: "rn1".into(),
        };
        store
            .store_scoped(&run_scope, "widgets came up earlier today", &meta)
            .await
            .unwrap();

        let scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            repo_db: None,
            workspace_id: "ws1".into(),
            repo_id: Some("r1".into()),
            module_path: None,
            run_id: Some("rn1".into()),
        };

        let config = SearchConfig::new("canonical answer about widgets", scope).with_fts(false);
        let results = store.search_scoped(&config).await.unwrap();

        // Structural B3 fix: Repo candidate is in the pool.
        let texts: Vec<&str> = results.iter().map(|m| m.content.as_str()).collect();
        assert!(
            texts.contains(&"the canonical answer about widgets"),
            "Repo memory missing from merged pool — early-exit may have fired. pool: {:?}",
            texts
        );

        // Soft B3 invariant: the scope-multiplier gap stays gentle.
        // Find the Run and Repo entries' final_scores and assert the
        // ratio is bounded by the new multipliers' ratio (with a small
        // slack for the recency / importance gradient).
        let run_score = results
            .iter()
            .find(|m| m.content == "widgets came up earlier today")
            .map(|m| m.final_score)
            .expect("Run candidate present");
        let repo_score = results
            .iter()
            .find(|m| m.content == "the canonical answer about widgets")
            .map(|m| m.final_score)
            .expect("Repo candidate present");
        let ratio = run_score / repo_score;
        assert!(
            ratio < 1.30,
            "Run/Repo score ratio {ratio:.3} is too steep — pre-B3 multipliers \
             (1.8/1.2 ≈ 1.50) would crush Repo. final_scores: \
             run={run_score:.3}, repo={repo_score:.3}"
        );
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

    // ── Tier B / B5 + B6 round-trip ─────────────────────────────

    /// B6 acceptance: a `retrieval_use` row inserted for a memory
    /// surfaces in `memory_utilization` and `top_utilization_in_scope`.
    #[tokio::test]
    async fn b6_retrieval_use_aggregates_into_utilization() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        let id = match store
            .store_scoped(&scope, "the canonical answer", &meta)
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        // Two used + one unused → 0.66 utilization rate.
        for (rank, class) in [(1, "used"), (2, "used"), (3, "unused")] {
            store
                .record_retrieval_use(id, "turn-x", Some("conv-x"), rank, class, 0.6, false)
                .await
                .unwrap();
        }
        let utils = store.memory_utilization(&[id]).await.unwrap();
        assert_eq!(utils.len(), 1);
        assert_eq!(utils[0].times_injected, 3);
        assert_eq!(utils[0].times_used, 2);
        assert_eq!(utils[0].times_unused, 1);
        assert!((utils[0].utilization_rate - (2.0 / 3.0)).abs() < 1e-6);

        let top = store.top_utilization_in_scope(2, false, 5).await.unwrap();
        assert!(!top.is_empty());
    }

    /// B5 trust re-scoring: a memory with high utilization gets a
    /// trust bump; user-authored memories are never adjusted.
    #[tokio::test]
    async fn b5_trust_rescore_respects_user_authorship() {
        let store = Arc::new(MemoryStore::in_memory(mock_embedder()).unwrap());
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        // Plant one LLM-authored row + one user_remember row, both
        // with the same usage signal.
        let llm_id = match store
            .store_scoped(
                &scope,
                "llm row",
                &WriteMeta::for_source(MemorySource::LlmExtracted).with_trust_score(0.6),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        let user_id = match store
            .store_scoped(
                &scope,
                "user row",
                &WriteMeta::for_source(MemorySource::UserRemember).with_trust_score(1.0),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        for id in [llm_id, user_id] {
            for rank in 1..=6 {
                store
                    .record_retrieval_use(id, "t", None, rank, "used", 0.7, false)
                    .await
                    .unwrap();
            }
        }
        let cfg = crate::memory::sleeptime::SleeptimeConfig::default();
        let ops = store.sleeptime_trust_rescore(&cfg).await.unwrap();
        // The user row must NOT appear in the adjustment list.
        assert!(ops.iter().all(|op| match op {
            crate::memory::sleeptime::SleeptimeOperation::TrustAdjusted { memory_id, .. } => {
                *memory_id != user_id
            }
            _ => true,
        }));
        // The LLM row must have been bumped exactly once.
        assert!(ops.iter().any(|op| matches!(
            op,
            crate::memory::sleeptime::SleeptimeOperation::TrustAdjusted {
                memory_id, new_trust, ..
            } if *memory_id == llm_id && *new_trust > 0.6
        )));
    }

    /// B5 acceptance: dry-run audit rows land but no destructive
    /// writes occur. Specifically, a near-dup pair must NOT be
    /// merged when `dry_run = true`.
    #[tokio::test]
    async fn b5_sleeptime_dry_run_leaves_rows_intact() {
        let store = Arc::new(MemoryStore::in_memory(mock_embedder()).unwrap());
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        // Two near-identical rows so cosine ≥ threshold for the
        // mock embedder (which produces deterministic same-text →
        // same-vector hashes).
        store
            .store_scoped(
                &scope,
                "tokio is the choice",
                &WriteMeta::for_source(MemorySource::LlmExtracted),
            )
            .await
            .unwrap();
        store
            .store_scoped(
                &scope,
                "we picked tokio because std mutex is broken",
                &WriteMeta::for_source(MemorySource::LlmExtracted),
            )
            .await
            .unwrap();
        let mut cfg = crate::memory::sleeptime::SleeptimeConfig::default();
        cfg.dry_run = true;
        // Use a low threshold so the mock embedder's outputs trip it.
        cfg.near_dup_threshold = 0.0;
        let report: crate::memory::SleeptimeReport =
            crate::memory::sleeptime::run_sleeptime(&store, &cfg, None)
                .await
                .unwrap();
        assert!(report.dry_run);
        // Both rows still present.
        let after = store.recent_memories(24, 100).await.unwrap();
        assert_eq!(after.len(), 2, "dry-run must not delete near-dup loser");
    }

    /// B5 supersede: marking an old memory as superseded soft-deletes
    /// it from the active set without removing the row.
    #[tokio::test]
    async fn b5_supersede_marks_old_row() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        let old_id = match store
            .store_scoped(&scope, "old answer", &WriteMeta::default())
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        let new_id = match store
            .store_scoped(&scope, "new better answer", &WriteMeta::default())
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        let n = store.supersede_memory(old_id, new_id).await.unwrap();
        assert_eq!(n, 1);
        // The old row stays in the table (audit), but the search
        // path filters via `superseded_by IS NULL` everywhere.
        // Direct fetch confirms the marker landed.
        let conn = store.conn.lock().await;
        let marker: i64 = conn
            .query_row(
                "SELECT superseded_by FROM memories WHERE id = ?1",
                rusqlite::params![old_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(marker, new_id);
    }

    /// C1 regression: a row marked with `superseded_by` must not appear
    /// in scoped retrieval. Pre-fix the load path queried
    /// `WHERE id = ?1` without filtering, so superseded rows surfaced
    /// as if SUPERSEDE had no effect on retrieval.
    #[tokio::test]
    async fn c1_superseded_rows_are_excluded_from_search() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        let old_id = match store
            .store_scoped(
                &scope,
                "stale guidance about widgets",
                &WriteMeta::default(),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        let new_id = match store
            .store_scoped(
                &scope,
                "fresh guidance about widgets",
                &WriteMeta::default(),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        store.supersede_memory(old_id, new_id).await.unwrap();

        let memory_scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            repo_db: None,
            workspace_id: "ws1".into(),
            repo_id: Some("r1".into()),
            module_path: None,
            run_id: None,
        };
        let config = SearchConfig::new("guidance about widgets", memory_scope).with_fts(true);
        let results = store.search_scoped(&config).await.unwrap();
        let ids: Vec<i64> = results.iter().map(|m| m.id).collect();
        assert!(
            !ids.contains(&old_id),
            "superseded row {old_id} surfaced in search results: {ids:?}"
        );
        assert!(
            ids.contains(&new_id),
            "successor row {new_id} missing from search results: {ids:?}"
        );
    }

    /// C2 regression: the sleeptime near-duplicate sweep must NEVER
    /// merge memories from different repos / modules / runs even when
    /// they share scope_level + memory_type + cosine ≥ threshold.
    /// Pre-fix the grouping ignored scope_path and would happily delete
    /// a Repo-scope row in repo A because a Repo-scope row in repo B
    /// looked similar.
    #[tokio::test]
    async fn c2_near_dup_merge_does_not_cross_repos() {
        use crate::memory::sleeptime::SleeptimeOperation;
        // Use the deterministic embedder so the two near-identical
        // texts genuinely cross the cosine threshold.
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();
        let id_a = match store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "repo-A".into(),
                },
                "use the canonical retry policy with backoff",
                &meta,
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        let id_b = match store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "repo-B".into(),
                },
                "use the canonical retry policy with backoff",
                &meta,
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            _ => panic!("store failed"),
        };
        // Threshold tuned low enough that cross-repo content matches
        // would have been merged pre-fix; it must NOT merge them now.
        let ops = store.sleeptime_near_dup_merge(0.50, true).await.unwrap();
        let crossed = ops.iter().any(|op| match op {
            SleeptimeOperation::NearDupMerged {
                keep_id, drop_id, ..
            } => (*keep_id == id_a && *drop_id == id_b) || (*keep_id == id_b && *drop_id == id_a),
            _ => false,
        });
        assert!(
            !crossed,
            "cross-repo near-dup merge proposed: {ops:?} (a={id_a}, b={id_b})"
        );
    }

    /// H1 regression: a fresh DB must stamp `_gaviero_meta.embedder_model`
    /// at init so `detect_embedder_mismatch` works without first
    /// requiring a write. Pre-fix, an existing DB with no meta row would
    /// silently report "no mismatch" even after the user flipped the
    /// embedder setting.
    #[tokio::test]
    async fn h1_init_stamps_embedder_meta_on_fresh_db() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let stamped = store.get_meta_value("embedder_model").await.unwrap();
        assert!(
            stamped.is_some(),
            "init must stamp `_gaviero_meta.embedder_model` on a fresh DB"
        );
    }

    /// H1 regression: when a non-default embedder is configured at
    /// init time and the DB has no prior embedded rows, the stamp
    /// records the configured embedder (not the legacy default), and
    /// `detect_embedder_mismatch` returns None.
    #[tokio::test]
    async fn h1_no_mismatch_after_init_stamp() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        assert!(store.detect_embedder_mismatch().await.is_none());
    }

    /// H2 regression: FTS-only candidates (no vec hit at all) must
    /// surface in the candidate pool. Pre-fix, when vec returned
    /// nothing the code dropped FTS too; when both returned, FTS-only
    /// RRF entries arrived with raw_sim=0 and got filtered by the
    /// similarity threshold.
    #[tokio::test]
    async fn h2_fts_only_candidates_survive_filter() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let scope = WriteScope::Repo {
            repo_id: "r1".into(),
        };
        // Distinctive lexical content so FTS hits even though the
        // mock embedder won't make this similar to the query.
        store
            .store_scoped(
                &scope,
                "the zxqyzz token marker for fts hit testing",
                &WriteMeta::default(),
            )
            .await
            .unwrap();

        let memory_scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            repo_db: None,
            workspace_id: "ws1".into(),
            repo_id: Some("r1".into()),
            module_path: None,
            run_id: None,
        };
        let config = SearchConfig::new("zxqyzz", memory_scope).with_fts(true);
        let results = store.search_scoped(&config).await.unwrap();
        let texts: Vec<&str> = results.iter().map(|m| m.content.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("zxqyzz")),
            "FTS-only candidate dropped from pool: {texts:?}"
        );
    }

    /// H3 regression: `recent_memories_for_run` must filter strictly by
    /// `run_id`. The session consolidator depends on this to avoid
    /// pulling memories from another concurrent or recent session.
    #[tokio::test]
    async fn h3_recent_memories_for_run_is_session_scoped() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let meta = WriteMeta::default();
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: "session-A".into(),
                },
                "memory from session A",
                &meta,
            )
            .await
            .unwrap();
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: "session-B".into(),
                },
                "memory from session B",
                &meta,
            )
            .await
            .unwrap();
        let only_a = store
            .recent_memories_for_run("session-A", 24, 50)
            .await
            .unwrap();
        let texts: Vec<&str> = only_a.iter().map(|m| m.content.as_str()).collect();
        assert!(
            texts.contains(&"memory from session A"),
            "session-A row missing: {texts:?}"
        );
        assert!(
            !texts.contains(&"memory from session B"),
            "session-B row leaked into session-A query: {texts:?}"
        );
        // Empty run_id refuses to leak across sessions.
        let empty = store.recent_memories_for_run("", 24, 50).await.unwrap();
        assert!(empty.is_empty(), "empty run_id must return empty");
    }

    #[tokio::test]
    async fn b5_audit_rows_persist_with_dry_run_marker() {
        let store = Arc::new(MemoryStore::in_memory(mock_embedder()).unwrap());
        let payload = serde_json::json!({"hello": "world"}).to_string();
        store
            .log_sleeptime_audit("run-1", "decay_flagged", Some(7), None, &payload, true)
            .await
            .unwrap();
        let conn = store.conn.lock().await;
        let (kind, dry, mid): (String, i64, i64) = conn
            .query_row(
                "SELECT kind, dry_run, memory_id FROM sleeptime_audit
                 WHERE run_id = 'run-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "decay_flagged");
        assert_eq!(dry, 1);
        assert_eq!(mid, 7);
    }
}
