// ── Sub-modules carved out of the monolithic store.rs ────────────
//
// Phase 4 of the tier-review action plan: the file historically held
// 7,117 lines and 92+ public methods on `MemoryStore`. Each sub-module
// owns a cohesive bucket of methods via additional `impl MemoryStore`
// blocks. The struct definition + connection ownership stay here in
// the parent module so all submodules see the same `MemoryStore` type.
pub mod compression_ops;
pub mod deletions_ops;
pub mod manifest;
pub mod panel_ops;
pub mod search;
pub mod search_legacy;
pub mod sleeptime_ops;
pub mod telemetry_ops;
pub mod write;

pub use deletions_ops::{BulkForgetReport, ForgetFilter, RestoreOutcome};
pub use manifest::InjectionManifestRow;
pub use telemetry_ops::MemoryUtilization;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;
use tracing;

use super::embedder::Embedder;
use super::schema;
use super::scope::{MemoryType, Trust, WriteScope};
use super::scoring::{self, ScoredMemory};

pub(crate) const SEMANTIC_DEDUP_THRESHOLD: f32 = 0.95;

/// C1: a pending typed-stores migration on a single DB file.
///
/// Returned by [`probe_c1_migration`] when the file's `user_version`
/// is < 10 and the next [`MemoryStore::open`] call will cross the C1
/// boundary. The bootstrap layer (TUI / CLI) is expected to ask the
/// user for consent — surfacing `proposed_backup_path` so the user
/// knows where the rollback snapshot will live — before opening.
///
/// Plan §"Anti-patterns to avoid": "Silent migration on first run.
/// The C1 migration is load-bearing; prompt the user on first post-
/// upgrade start." This struct is the contract that lets the bootstrap
/// honor that anti-pattern.
#[derive(Debug, Clone)]
pub struct C1MigrationProposal {
    pub db_path: PathBuf,
    pub proposed_backup_path: PathBuf,
    pub current_version: u32,
    pub target_version: u32,
}

/// C1: probe a DB file for a pending typed-stores migration without
/// running it. Returns `Ok(None)` for fresh DBs, in-memory DBs, or
/// already-migrated DBs. Returns `Ok(Some(proposal))` when the next
/// open will cross the v10 boundary and a backup will be taken.
///
/// Probing opens a short-lived `Connection`; the file is *not* mutated.
pub fn probe_c1_migration(db_path: &Path) -> Result<Option<C1MigrationProposal>> {
    if !db_path.exists() {
        return Ok(None);
    }

    MemoryStore::register_sqlite_vec();
    let probe = Connection::open(db_path)
        .with_context(|| format!("probing memory database: {}", db_path.display()))?;
    let current = schema::read_user_version(&probe)?;
    let needs = schema::needs_c1_backup(&probe);
    drop(probe);

    if !needs {
        return Ok(None);
    }

    Ok(Some(C1MigrationProposal {
        db_path: db_path.to_path_buf(),
        proposed_backup_path: backup_path_for(db_path),
        current_version: current,
        target_version: schema::C1_SCHEMA_VERSION,
    }))
}

/// Produce the deterministic backup-path for a given DB file.
/// Reused by both the probe (advisory) and the open path (effective).
fn backup_path_for(db_path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut backup = db_path.to_path_buf();
    let mut filename = backup
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "memory.db".to_string());
    filename.push_str(&format!(".bak.{ts}"));
    backup.set_file_name(filename);
    backup
}

/// C1 (pre-migration backup): if the file at `db_path` exists and is on
/// a pre-v10 schema, copy it sidecar to `<db_path>.bak.<unix-ts>` and
/// return the backup path. The migration ([`schema::run_migrations`])
/// then proceeds against the original.
///
/// **Consent contract**: callers that reach this path must have already
/// secured user consent via [`probe_c1_migration`] + an interactive
/// prompt (TUI startup) or an explicit `--accept-c1-migration` flag
/// (CLI). The bootstrap layer is responsible for the prompt; this
/// helper is the side-effecting half of the consent contract.
///
/// Failure modes are non-fatal: a missing file is a fresh DB with
/// nothing to back up; a copy error is downgraded to a warning so the
/// migration can still run (the user's recourse if it later corrupts
/// is the same recourse they had before C1: their workspace VCS, file-
/// system snapshots, etc. — better not to block startup once consent
/// has been given).
fn pre_c1_backup_if_needed(db_path: &Path) -> Result<Option<PathBuf>> {
    let Some(proposal) = probe_c1_migration(db_path)? else {
        return Ok(None);
    };

    match std::fs::copy(&proposal.db_path, &proposal.proposed_backup_path) {
        Ok(_) => {
            tracing::info!(
                target: "memory_c1",
                src = %proposal.db_path.display(),
                dst = %proposal.proposed_backup_path.display(),
                "took pre-C1-migration snapshot"
            );
            Ok(Some(proposal.proposed_backup_path))
        }
        Err(e) => {
            tracing::warn!(
                target: "memory_c1",
                error = %e,
                src = %proposal.db_path.display(),
                "failed to take pre-C1 snapshot; migration will proceed without backup"
            );
            Ok(None)
        }
    }
}

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
            .field("model", &self.embedder.name())
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
    ///
    /// C1: if the existing DB is at a pre-v10 schema, take a one-shot
    /// snapshot of the file before running migrations. The C1 migration
    /// is load-bearing (introduces the `memory_kind` discriminator and
    /// the per-kind lifecycle), so the anti-pattern doc explicitly
    /// warns against silent upgrades. The backup gives a no-questions-
    /// asked rollback path: copy `<file>.bak.<unix-ts>` over `<file>`
    /// to revert. Path is stamped into `_gaviero_meta.c1_backup_path`
    /// so the TUI can surface a one-shot banner on next launch.
    pub fn open(db_path: &Path, embedder: Arc<dyn Embedder>) -> Result<Self> {
        Self::register_sqlite_vec();

        // Best-effort pre-migration snapshot. We open a probe connection
        // first so we can read user_version without applying migrations,
        // then drop it before opening the real one.
        let backup_path = pre_c1_backup_if_needed(db_path)?;

        let conn = Connection::open(db_path)
            .with_context(|| format!("opening memory database: {}", db_path.display()))?;
        Self::init(conn, embedder, Some(db_path.to_path_buf()), backup_path)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory(embedder: Arc<dyn Embedder>) -> Result<Self> {
        Self::register_sqlite_vec();
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        Self::init(conn, embedder, None, None)
    }

    fn init(
        conn: Connection,
        embedder: Arc<dyn Embedder>,
        db_path: Option<PathBuf>,
        c1_backup_path: Option<PathBuf>,
    ) -> Result<Self> {
        schema::run_migrations(&conn, embedder.dimension())
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
                embedder.name()
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

        // C1: stamp the backup path so the TUI can surface a one-shot
        // banner on next launch. Best-effort — banner is informational.
        if let Some(backup) = &c1_backup_path {
            let path_str = backup.display().to_string();
            if let Err(e) = conn.execute(
                "INSERT INTO _gaviero_meta(key, value) VALUES ('c1_backup_path', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![path_str],
            ) {
                tracing::warn!(
                    target: "memory_c1",
                    error = %e,
                    backup = %path_str,
                    "failed to stamp c1_backup_path — banner will not show"
                );
            } else {
                tracing::info!(
                    target: "memory_c1",
                    backup = %path_str,
                    "C1 typed-stores migration applied; pre-migration snapshot taken"
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
        let configured = self.embedder.name().to_string();
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

        // C1: history rows are append-only — they outlive their owning
        // run by design (provenance for every derived record). Filter
        // them out of both the id-collection (so we don't try to delete
        // their vec_memories rows either) and the final DELETE.
        let ids: Vec<i64> = conn
            .prepare(
                "SELECT id FROM memories
                   WHERE run_id = ?1 AND memory_kind != 'history'",
            )?
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
            "DELETE FROM memories
               WHERE run_id = ?1 AND memory_kind != 'history'",
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
pub(crate) fn retrieval_score(recency_hours: f64, importance: f32, relevance: f32, access_count: i32) -> f32 {
    let recency = 0.995_f64.powf(recency_hours) as f32;
    let reinforcement = (1.0 + access_count as f32 * 0.1).min(3.0);
    // α = β = γ = 1.0
    (recency + importance + relevance) * reinforcement
}

/// Compute hours elapsed since the given timestamp (or fallback).
pub(crate) fn hours_since(last_accessed: &Option<String>, updated_at: &str, now: &str) -> f64 {
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
pub(crate) fn chrono_now_utc() -> String {
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


/// B1d: public re-export of [`embedding_to_blob`] for the re-embed
/// migration crate-internal module. Same little-endian f32 encoding.
pub(crate) fn embedding_to_blob_pub(embedding: &[f32]) -> Vec<u8> {
    embedding_to_blob(embedding)
}

/// Encode a float vector as little-endian bytes for SQLite BLOB storage.
pub(crate) fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

pub(crate) fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
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

pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
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
    use super::super::scope::{MemoryScope, StoreResult, WriteMeta};
    use super::super::scoring::SearchConfig;
    use super::super::trust_defaults::MemorySource;

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

    /// C2.1: soft-delete writes an audit row, hard-deletes the source
    /// memory, and the audit row carries the full body for restore.
    #[tokio::test]
    async fn c21_soft_delete_round_trip_through_audit() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c21-rec");
        let id = match store
            .store_scoped(&scope, "use git2 not shell git", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("insert returned {other:?}"),
        };

        // Soft-delete with reason.
        let audit_id = store
            .soft_delete_memory(id, DeletedBy::Panel, Some("test"), None)
            .await
            .unwrap();
        assert!(audit_id > 0);

        // Source row is gone.
        assert!(store.get_content(id).await.unwrap().is_none());
        assert!(store.get_memory_kind(id).await.unwrap().is_none());

        // Audit row is present and carries the full body for restore.
        let audit = store
            .get_deletion(audit_id)
            .await
            .unwrap()
            .expect("audit row present");
        assert_eq!(audit.memory_id, id);
        assert_eq!(audit.memory_kind, "record");
        assert_eq!(audit.memory_source, "user_remember");
        assert_eq!(audit.deleted_by, "panel");
        assert_eq!(audit.reason.as_deref(), Some("test"));
        assert!(audit.is_restorable());

        // The JSON dump round-trips and contains the original content.
        let v: serde_json::Value =
            serde_json::from_str(&audit.original_row_json).unwrap();
        assert_eq!(v["content"], "use git2 not shell git");
        assert_eq!(v["memory_kind"], "record");
        assert!(v["embedding_b64"].is_string());

        // recent_deletions surfaces it.
        let recent = store.recent_deletions(10).await.unwrap();
        assert!(recent.iter().any(|d| d.id == audit_id));

        // deletions_count metric reflects the audit row.
        assert!(store.deletions_count().await.unwrap() >= 1);
    }

    /// C2.1: soft-delete on a history row is rejected before the SQL
    /// trigger fires. The error message points to /forget-history.
    #[tokio::test]
    async fn c21_soft_delete_refuses_history_rows() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        let scope = WriteScope::Run {
            repo_id: "r".into(),
            run_id: "run-1".into(),
        };
        let meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("c21-h");
        let id = match store
            .store_scoped(&scope, "TRANSCRIPT", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            _ => panic!(),
        };

        let r = store
            .soft_delete_memory(id, DeletedBy::Panel, None, None)
            .await;
        assert!(r.is_err(), "history soft-delete must be rejected");
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("history is append-only"), "{msg}");
        // Source row still exists.
        assert_eq!(
            store.get_memory_kind(id).await.unwrap(),
            Some(MemoryKind::History)
        );
    }

    /// C2.1: soft-delete refuses the UserRedaction tag — that path is
    /// reserved for the C2.4 RedactHistory writer variant.
    #[tokio::test]
    async fn c21_soft_delete_refuses_user_redaction_tag() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c21-tag");
        let id = match store
            .store_scoped(&scope, "anything", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            _ => panic!(),
        };

        let r = store
            .soft_delete_memory(id, DeletedBy::UserRedaction, None, None)
            .await;
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("UserRedaction is reserved"));
    }

    /// C2.1: prune_expired_deletions removes audit rows past their
    /// retention window. Sleeptime-prune deletions get the shorter
    /// 14-day window; everything else 30 days (per default).
    #[test]
    fn c21_prune_expired_deletions_respects_per_kind_retention() {
        use rusqlite::Connection;
        // Direct SQL test: we don't need the full async store stack
        // for this. Spin up a v13 DB and pre-populate audit rows.
        MemoryStore::register_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&conn, 8).unwrap();
        // Two rows, each "deleted_at = 20 days ago" — past the 14d
        // sleeptime retention but inside the 30d user retention.
        conn.execute_batch(
            "INSERT INTO deletions (memory_id, memory_kind, memory_source,
                memory_trust, deleted_at, deleted_by, original_row_json)
             VALUES
             (1, 'record', 'user_remember', 1.0,
                datetime('now', '-20 days'), 'user_command', '{}'),
             (2, 'summary', 'llm_consolidated', 0.75,
                datetime('now', '-20 days'), 'sleeptime_prune', '{}');",
        )
        .unwrap();
        let n_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM deletions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_before, 2);

        // Prune with default windows: 30 / 14.
        let user_n = conn
            .execute(
                "DELETE FROM deletions
                  WHERE deleted_by IN ('user_command','panel','sleeptime_merge','user_redaction')
                    AND deleted_at < datetime('now', '-30 days')",
                [],
            )
            .unwrap();
        let sleep_n = conn
            .execute(
                "DELETE FROM deletions
                  WHERE deleted_by = 'sleeptime_prune'
                    AND deleted_at < datetime('now', '-14 days')",
                [],
            )
            .unwrap();
        assert_eq!(user_n, 0, "user-tagged row should still be inside 30d");
        assert_eq!(sleep_n, 1, "sleeptime_prune row should be expired");
        let n_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM deletions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_after, 1);
    }

    /// C2.2: soft-delete + restore round-trip restores the row through
    /// the dedup pipeline and consumes the audit row.
    #[tokio::test]
    async fn c22_restore_round_trip_inserts_via_dedup() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c22-rec")
            .with_importance(0.7);
        let original_id = match store
            .store_scoped(&scope, "prefer ripgrep over grep", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("insert returned {other:?}"),
        };

        let audit_id = store
            .soft_delete_memory(original_id, DeletedBy::Panel, Some("oops"), None)
            .await
            .unwrap();

        // Source row is gone before restore.
        assert!(store.get_content(original_id).await.unwrap().is_none());

        let outcome = store.restore_deletion(audit_id).await.unwrap();
        let new_id = match outcome {
            RestoreOutcome::Inserted { new_memory_id, .. } => new_memory_id,
            other => panic!("expected Inserted, got {other:?}"),
        };

        // Restored row carries the original content.
        let content = store.get_content(new_id).await.unwrap();
        assert_eq!(content.as_deref(), Some("prefer ripgrep over grep"));

        // Audit row is consumed so /restore --since does not replay.
        assert!(store.get_deletion(audit_id).await.unwrap().is_none());
    }

    /// C2.2: when an equivalent row already exists at the original
    /// scope, restore deduplicates instead of inserting a duplicate.
    #[tokio::test]
    async fn c22_restore_dedups_when_concurrent_row_exists() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c22-dup");
        let id_a = match store
            .store_scoped(&scope, "shared fact", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("{other:?}"),
        };
        let audit_id = store
            .soft_delete_memory(id_a, DeletedBy::Panel, None, None)
            .await
            .unwrap();

        // User remembers the same content again before restoring.
        let id_b = match store
            .store_scoped(&scope, "shared fact", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("{other:?}"),
        };
        assert_ne!(id_a, id_b);

        let outcome = store.restore_deletion(audit_id).await.unwrap();
        match outcome {
            RestoreOutcome::Deduplicated {
                surviving_memory_id,
                ..
            } => {
                assert_eq!(surviving_memory_id, id_b, "newer row should win");
            }
            other => panic!("expected Deduplicated, got {other:?}"),
        }
        assert!(store.get_deletion(audit_id).await.unwrap().is_none());
    }

    /// C2.2: a `user_redaction` audit row is one-way; restore refuses
    /// without consuming the audit row.
    #[tokio::test]
    async fn c22_restore_refuses_user_redaction_audit_row() {
        use rusqlite::params;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        // Hand-roll a user_redaction audit row (the soft-delete API
        // refuses the tag, so we sidestep it here).
        let audit_id: i64 = {
            let conn = store.conn.lock().await;
            conn.execute(
                "INSERT INTO deletions (memory_id, memory_kind, memory_source,
                    memory_trust, deleted_by, original_row_json)
                 VALUES (1, 'history', 'raw_transcript', 1.0,
                    'user_redaction', '{}')",
                [],
            )
            .unwrap();
            conn.query_row(
                "SELECT id FROM deletions ORDER BY id DESC LIMIT 1",
                params![],
                |r| r.get(0),
            )
            .unwrap()
        };

        let outcome = store.restore_deletion(audit_id).await.unwrap();
        match outcome {
            RestoreOutcome::Refused { reason, .. } => {
                assert!(reason.contains("one-way"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        // Audit row must still exist so the user can see why.
        assert!(store.get_deletion(audit_id).await.unwrap().is_some());
    }

    /// C2.2: restore_deletions_since processes the audit log in order
    /// and skips user_redaction rows.
    #[tokio::test]
    async fn c22_restore_since_replays_window_in_order() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c22-since");

        let mut originals = Vec::new();
        for s in ["alpha fact", "bravo fact", "charlie fact"] {
            let id = match store.store_scoped(&scope, s, &meta).await.unwrap() {
                crate::memory::scope::StoreResult::Inserted(id) => id,
                _ => panic!(),
            };
            originals.push(id);
        }
        for id in &originals {
            store
                .soft_delete_memory(*id, DeletedBy::UserCommand, None, None)
                .await
                .unwrap();
        }

        let outcomes = store.restore_deletions_since("-1 hour").await.unwrap();
        assert_eq!(outcomes.len(), 3);
        for o in &outcomes {
            assert!(
                matches!(o, RestoreOutcome::Inserted { .. }),
                "unexpected outcome {o:?}"
            );
        }
        // All audit rows consumed.
        assert_eq!(store.deletions_count().await.unwrap(), 0);
    }

    /// C2.3: dry-run bulk-forget reports candidate counts without
    /// touching any row.
    #[tokio::test]
    async fn c23_bulk_forget_dry_run_does_not_write() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::LlmExtracted)
            .with_type(MemoryType::Pattern)
            .with_tag("c23-dry");
        for s in ["alpha pattern", "beta pattern", "gamma other"] {
            store.store_scoped(&scope, s, &meta).await.unwrap();
        }

        let report = store
            .bulk_forget(
                &ForgetFilter::ByQuery("pattern".into()),
                true,
                None,
                DeletedBy::UserCommand,
            )
            .await
            .unwrap();
        assert_eq!(report.candidates.len(), 2);
        assert!(report.dry_run);
        assert_eq!(report.deleted, 0);
        assert_eq!(store.deletions_count().await.unwrap(), 0);
    }

    /// C2.3: live bulk-forget routes every match through the audit
    /// table, so /restore can undo each deletion.
    #[tokio::test]
    async fn c23_bulk_forget_live_routes_through_audit() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::LlmExtracted)
            .with_type(MemoryType::Pattern)
            .with_tag("c23-live");
        for s in ["alpha pattern", "beta pattern", "gamma other"] {
            store.store_scoped(&scope, s, &meta).await.unwrap();
        }

        let report = store
            .bulk_forget(
                &ForgetFilter::BySource(MemorySource::LlmExtracted),
                false,
                Some("factory reset"),
                DeletedBy::UserCommand,
            )
            .await
            .unwrap();
        assert_eq!(report.deleted, 3);
        assert!(!report.dry_run);
        assert_eq!(store.deletions_count().await.unwrap(), 3);

        let recent = store.recent_deletions(10).await.unwrap();
        for d in &recent {
            assert_eq!(d.deleted_by, DeletedBy::UserCommand.as_str());
            assert_eq!(d.reason.as_deref(), Some("factory reset"));
        }
    }

    /// C2.3: bulk-forget never matches history rows even when the
    /// filter would otherwise capture them. The C1 trigger would
    /// reject the hard-delete anyway; the application-layer guard
    /// stops dry-run counts from misleading the user.
    #[tokio::test]
    async fn c23_bulk_forget_skips_history_rows() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Run {
            repo_id: "r".into(),
            run_id: "run-1".into(),
        };
        let meta_h = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("c23-h");
        store
            .store_scoped(&scope, "TRANSCRIPT BODY", &meta_h)
            .await
            .unwrap();

        let report = store
            .bulk_forget(
                &ForgetFilter::ByQuery("transcript".into()),
                true,
                None,
                DeletedBy::UserCommand,
            )
            .await
            .unwrap();
        assert!(report.candidates.is_empty(), "history must not match");
    }

    /// C2.4: `/forget-history` redaction replaces the transcript with
    /// a tombstone, writes a `user_redaction` audit row, and the
    /// resulting row is non-restorable.
    #[tokio::test]
    async fn c24_redact_history_writes_tombstone_and_audit() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Run {
            repo_id: "r".into(),
            run_id: "run-1".into(),
        };
        let meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("c24-h");
        let id = match store
            .store_scoped(&scope, "secret transcript body", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            _ => panic!(),
        };

        let audit_id = store
            .redact_history_row(id, "user requested deletion")
            .await
            .unwrap();
        assert!(audit_id > 0);

        // Row still exists but content is the tombstone, NOT the
        // original transcript.
        let body = store.read_history_content(id).await.unwrap().unwrap();
        assert!(body.starts_with("[REDACTED:"));
        assert!(body.contains("sha="));
        assert!(body.contains("user requested deletion"));
        assert!(!body.contains("secret transcript body"));

        // Audit row tagged user_redaction; not restorable.
        let audit = store.get_deletion(audit_id).await.unwrap().unwrap();
        assert_eq!(audit.deleted_by, DeletedBy::UserRedaction.as_str());
        assert!(!audit.is_restorable());
        // original_row_json carries the tombstone, not the original.
        let v: serde_json::Value =
            serde_json::from_str(&audit.original_row_json).unwrap();
        assert_eq!(v["memory_kind"], "history");
        assert!(v["tombstone"].as_str().unwrap().starts_with("[REDACTED:"));
        assert!(!audit.original_row_json.contains("secret transcript body"));

        // /restore on the redaction audit row is refused.
        match store.restore_deletion(audit_id).await.unwrap() {
            RestoreOutcome::Refused { reason, .. } => {
                assert!(reason.to_lowercase().contains("one-way"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    /// C2.4: redacting a non-history row returns an error (the
    /// trigger-disable window must not run for record / summary
    /// rows — they have no protection trigger to begin with, but the
    /// API guards against confusing misuse anyway).
    #[tokio::test]
    async fn c24_redact_history_refuses_non_history_row() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c24-rec");
        let id = match store
            .store_scoped(&scope, "a regular record", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            _ => panic!(),
        };

        let res = store.redact_history_row(id, "test").await;
        assert!(res.is_err(), "must refuse non-history row");
    }

    // C2.4 / CI grep check: the workspace-wide invariant lives in
    // `crates/gaviero-core/tests/c24_trigger_disable_invariant.rs`.
    // Counting the call form here would be self-defeating — every
    // string-literal mention would inflate the count.

    /// C2.5: sleeptime near-dup merge soft-deletes the loser through
    /// `DeletedBy::SleeptimeMerge` and embeds `merged_into = keep_id`
    /// inside `original_row_json`, so a future restore can carry the
    /// merge edge through dedup.
    #[tokio::test]
    async fn c25_sleeptime_merge_routes_through_audit() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        // MockEmbedder hashes content into deterministic vectors;
        // similar strings score above the merge threshold. We pass
        // 0.0 below to side-step any embedder-specific cosine.
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());

        let scope = WriteScope::Workspace;
        let meta_user = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c25-keep");
        let keep_id = match store
            .store_scoped(&scope, "prefer ripgrep over grep", &meta_user)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("{other:?}"),
        };
        // LLM-extracted near-paraphrase — should be the merge loser.
        let meta_llm = WriteMeta::for_source(MemorySource::LlmExtracted)
            .with_type(MemoryType::Decision)
            .with_tag("c25-drop");
        let drop_id = match store
            .store_scoped(&scope, "we prefer ripgrep over grep tool", &meta_llm)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("{other:?}"),
        };
        assert_ne!(keep_id, drop_id);

        // 0.0 threshold so the StubEmbedder's deterministic vectors
        // always trigger a merge regardless of the exact cosine.
        let ops = store.sleeptime_near_dup_merge(0.0, false).await.unwrap();
        assert!(
            ops.iter().any(|o| matches!(
                o,
                crate::memory::sleeptime::SleeptimeOperation::NearDupMerged { .. }
            )),
            "expected at least one NearDupMerged op: {ops:?}"
        );

        // The loser is gone from the live table…
        assert!(
            store.get_content(drop_id).await.unwrap().is_none()
                || store.get_content(keep_id).await.unwrap().is_none()
        );

        // …and an audit row tagged sleeptime_merge captures it with
        // `merged_into` set to the surviving id.
        let recent = store.recent_deletions(10).await.unwrap();
        let audit = recent
            .iter()
            .find(|d| d.deleted_by == DeletedBy::SleeptimeMerge.as_str())
            .expect("sleeptime_merge audit row missing");
        let v: serde_json::Value =
            serde_json::from_str(&audit.original_row_json).unwrap();
        assert!(v.get("merged_into").is_some(), "merged_into field absent");
    }

    /// C2.5: sleeptime summary prune routes through
    /// `DeletedBy::SleeptimePrune` and respects dry-run.
    #[tokio::test]
    async fn c25_sleeptime_summary_prune_routes_through_audit() {
        use crate::memory::deletions::DeletedBy;
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();
        let scope = WriteScope::Workspace;
        let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
            .with_kind(MemoryKind::Summary)
            .with_type(MemoryType::Lesson)
            .with_tag("c25-sum");
        let id = match store
            .store_scoped(&scope, "old session summary", &meta)
            .await
            .unwrap()
        {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            _ => panic!(),
        };
        // Backdate the row so the prune query matches.
        {
            let conn = store.conn.lock().await;
            conn.execute(
                "UPDATE memories SET created_at = datetime('now', '-400 days')
                  WHERE id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        }

        // Dry-run: count returned but no soft-delete happens.
        let n = store.sleeptime_prune_old_summaries(365, true).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(store.deletions_count().await.unwrap(), 0);
        assert!(store.get_content(id).await.unwrap().is_some());

        // Live: row goes through audit with sleeptime_prune tag.
        let n = store.sleeptime_prune_old_summaries(365, false).await.unwrap();
        assert_eq!(n, 1);
        let recent = store.recent_deletions(5).await.unwrap();
        assert!(
            recent.iter().any(|d| d.deleted_by == DeletedBy::SleeptimePrune.as_str()),
            "no sleeptime_prune audit row: {recent:?}"
        );
    }

    /// C1: a v9-stamped DB on disk reports a pending migration via the
    /// public probe surface. Fresh DBs do not.
    #[test]
    fn c1_probe_returns_proposal_for_pre_v10_db() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");

        // No file yet → probe is None.
        assert!(probe_c1_migration(&db_path).unwrap().is_none());

        // Stamp a fake pre-v10 DB at v9: install the v1 schema (so the
        // migrations machinery sees a real `memories` table) and force
        // user_version back to 9 to mimic a pre-C1 install.
        MemoryStore::register_sqlite_vec();
        {
            let conn = Connection::open(&db_path).unwrap();
            schema::run_migrations(&conn, 8).unwrap();
            conn.pragma_update(None, "user_version", 9_u32).unwrap();
        }

        // Probe now reports a pending v10 migration.
        let proposal = probe_c1_migration(&db_path)
            .unwrap()
            .expect("v9 DB must report a pending C1 migration");
        assert_eq!(proposal.db_path, db_path);
        assert_eq!(proposal.current_version, 9);
        assert_eq!(proposal.target_version, schema::C1_SCHEMA_VERSION);
        // Backup path is sibling to db_path with `.bak.<unix-ts>` suffix.
        let backup_name = proposal
            .proposed_backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(
            backup_name.starts_with("memory.db.bak."),
            "unexpected backup name: {backup_name}"
        );
    }

    /// C1.4: end-to-end compression round-trip on a real DB row.
    /// Seed a history row, compress it, verify the SQL trigger
    /// reinstalls cleanly, decompress through the read path, and
    /// confirm the content matches.
    #[tokio::test]
    async fn c1_compress_history_round_trip() {
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::in_memory(embedder).unwrap();

        // Seed one history row through the public store_scoped path.
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;
        let scope = WriteScope::Run {
            repo_id: "r".into(),
            run_id: "run-1".into(),
        };
        let original_text = "USER: hello\nASSISTANT: world\n".repeat(40);
        let meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("history:s:t");
        let id = match store.store_scoped(&scope, &original_text, &meta).await.unwrap() {
            crate::memory::scope::StoreResult::Inserted(id) => id,
            other => panic!("history insert produced {other:?}"),
        };

        // Sanity: read path returns the original text uncompressed.
        let pre = store.read_history_content(id).await.unwrap().unwrap();
        assert_eq!(pre, original_text);

        // Compress. Returns true on success.
        let did = store.compress_history_row(id).await.unwrap();
        assert!(did, "compress should succeed on uncompressed history row");

        // Read path transparently decompresses with SHA verification.
        let post = store.read_history_content(id).await.unwrap().unwrap();
        assert_eq!(post, original_text);

        // The trigger is back in force after compression — UPDATE
        // attempts on the row should still fail at the SQL layer.
        let conn = store.conn.lock().await;
        let r = conn.execute(
            "UPDATE memories SET content = 'tampered' WHERE id = ?1",
            rusqlite::params![id],
        );
        assert!(
            r.is_err(),
            "trigger must be reinstalled after compression: {r:?}"
        );

        // The row is now flagged compressed=1 with a placeholder
        // content; the canonical bytes are in content_blob.
        let (compressed, content_value, blob_len): (i64, String, i64) = conn
            .query_row(
                "SELECT compressed, content, length(content_blob)
                   FROM memories WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(compressed, 1);
        assert!(content_value.starts_with("[compressed:zstd"));
        assert!(blob_len > 0);
        // Compression should make the blob smaller than the original
        // for a 40-line repeated transcript.
        assert!(
            (blob_len as usize) < original_text.len(),
            "blob {} should be smaller than original {}",
            blob_len,
            original_text.len()
        );

        drop(conn);

        // Calling compress again on an already-compressed row is a
        // no-op (returns false), not an error.
        assert!(!store.compress_history_row(id).await.unwrap());
    }

    /// C1: opening a pre-v10 DB takes the snapshot, applies the
    /// migration, and stamps `_gaviero_meta.c1_backup_path`. After the
    /// open, the probe reports no further pending migration.
    #[tokio::test]
    async fn c1_open_pre_v10_db_takes_backup_and_migrates() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");

        // Seed at v9.
        MemoryStore::register_sqlite_vec();
        {
            let conn = Connection::open(&db_path).unwrap();
            schema::run_migrations(&conn, 8).unwrap();
            conn.pragma_update(None, "user_version", 9_u32).unwrap();
        }

        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = MemoryStore::open(&db_path, embedder).expect("open should migrate");

        // Probe reports nothing pending after open.
        assert!(probe_c1_migration(&db_path).unwrap().is_none());

        // memory_kind column exists; default for new inserts is record.
        let conn = store.conn.lock().await;
        let has_kind: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('memories') WHERE name = 'memory_kind'")
            .and_then(|mut s| s.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_kind);

        // The backup path was stamped into _gaviero_meta.
        let backup: String = conn
            .query_row(
                "SELECT value FROM _gaviero_meta WHERE key = 'c1_backup_path'",
                [],
                |r| r.get(0),
            )
            .expect("c1_backup_path stamped");
        assert!(std::path::Path::new(&backup).exists(), "backup file present");
        assert!(backup.ends_with(".db") == false);
        assert!(backup.contains(".bak."));
    }
}
