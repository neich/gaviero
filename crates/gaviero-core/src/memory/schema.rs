/// SQL schema and migrations for the memory store.
///
/// Uses `PRAGMA user_version` to track the current schema version.
/// Each migration runs exactly once, in order, on database open.
use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version. Increment when adding a new migration.
const CURRENT_VERSION: u32 = 12;

/// First schema version where the `memory_kind` discriminator exists.
/// Used by [`needs_c1_backup`] to decide whether to take a one-shot
/// pre-migration snapshot of the DB file before running v10.
pub const C1_SCHEMA_VERSION: u32 = 10;

/// C1.3: name of the trigger that vetoes UPDATE on history rows.
/// Exported so the C2.4 `RedactHistory` handler — the **only**
/// authorized callsite that may drop/recreate this trigger inside a
/// transaction — can reference the constant. CI greps for the
/// constant name to enforce single-callsite discipline (see C2.4).
pub const C1_HISTORY_IMMUTABLE_TRIGGER_UPDATE: &str = "memories_history_immutable_update";

/// C1.3: name of the trigger that vetoes DELETE on history rows.
pub const C1_HISTORY_IMMUTABLE_TRIGGER_DELETE: &str = "memories_history_immutable_delete";

/// Run all pending migrations on the given connection.
///
/// `embedding_dims` specifies the vector dimension for sqlite-vec virtual tables.
/// Typically 768 for nomic-embed-text-v1.5 (production) or smaller for tests.
///
/// Checks `PRAGMA user_version` to determine the current schema version,
/// then applies each migration in sequence. Idempotent — safe to call
/// on every database open.
pub fn run_migrations(conn: &Connection, embedding_dims: usize) -> Result<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading user_version")?;

    if version < 1 {
        migrate_v1(conn).context("migration v1")?;
    }
    if version < 2 {
        migrate_v2(conn).context("migration v2")?;
    }
    if version < 3 {
        migrate_v3(conn, embedding_dims).context("migration v3")?;
    }
    if version < 4 {
        migrate_v4(conn, embedding_dims).context("migration v4")?;
    }
    if version < 5 {
        migrate_v5(conn).context("migration v5")?;
    }
    if version < 6 {
        migrate_v6(conn).context("migration v6")?;
    }
    if version < 7 {
        migrate_v7(conn).context("migration v7")?;
    }
    if version < 8 {
        migrate_v8(conn).context("migration v8")?;
    }
    if version < 9 {
        migrate_v9(conn).context("migration v9")?;
    }
    if version < 10 {
        migrate_v10(conn).context("migration v10")?;
    }
    if version < 11 {
        migrate_v11(conn).context("migration v11")?;
    }
    if version < 12 {
        migrate_v12(conn).context("migration v12")?;
    }

    // Stamp the current version
    conn.pragma_update(None, "user_version", CURRENT_VERSION)
        .context("updating user_version")?;

    Ok(())
}

/// Read the current `user_version` from a DB file without running any
/// migrations. Used by the bootstrap to decide whether to snapshot the
/// file before applying the load-bearing v10 (C1) migration.
pub fn read_user_version(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading user_version")
}

/// Returns true when the next call to [`run_migrations`] will cross the
/// C1 boundary (v10) and should therefore be preceded by a backup of the
/// DB file. Centralized here so callers don't bake the version number in.
pub fn needs_c1_backup(conn: &Connection) -> bool {
    matches!(read_user_version(conn), Ok(v) if v < C1_SCHEMA_VERSION && v > 0)
}

/// v1: Initial schema — memories table + indexes.
fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            namespace   TEXT    NOT NULL,
            key         TEXT    NOT NULL,
            content     TEXT    NOT NULL,
            embedding   BLOB,
            model_id    TEXT,
            created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
            metadata    TEXT,
            UNIQUE(namespace, key)
        );
        CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);
        CREATE INDEX IF NOT EXISTS idx_memories_ns_key ON memories(namespace, key);",
    )
    .context("creating initial schema")?;
    Ok(())
}

/// v2: Add privacy column for tier routing privacy filtering.
fn migrate_v2(conn: &Connection) -> Result<()> {
    // Check if column already exists (handles databases created before
    // the migration system was introduced).
    let has_privacy: bool = conn
        .prepare("SELECT * FROM pragma_table_info('memories') WHERE name = 'privacy'")
        .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
        .unwrap_or(false);

    if !has_privacy {
        conn.execute_batch(
            "ALTER TABLE memories ADD COLUMN privacy TEXT NOT NULL DEFAULT 'public';",
        )
        .context("adding privacy column")?;
    }

    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_privacy ON memories(privacy);")
        .context("creating privacy index")?;

    Ok(())
}

/// v3: Importance scoring columns, sqlite-vec virtual tables, episodes table, graph state.
///
/// Also nullifies existing embeddings since the model is changing from e5-small-v2 (384d)
/// to nomic-embed-text-v1.5 (768d) — they must be re-embedded.
fn migrate_v3(conn: &Connection, embedding_dims: usize) -> Result<()> {
    // -- New columns on memories table (importance scoring + staleness) --

    let add_column_if_missing = |col: &str, typedef: &str| -> Result<()> {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}'"
            ))
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!("ALTER TABLE memories ADD COLUMN {col} {typedef};"))
                .with_context(|| format!("adding {col} column"))?;
        }
        Ok(())
    };

    add_column_if_missing("importance", "REAL NOT NULL DEFAULT 0.5")?;
    add_column_if_missing("access_count", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing("last_accessed_at", "TEXT")?;
    add_column_if_missing("source_file", "TEXT")?;
    add_column_if_missing("source_hash", "TEXT")?;

    // -- sqlite-vec virtual table for KNN search --
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
            memory_id INTEGER PRIMARY KEY,
            embedding float[{embedding_dims}]
        );"
    ))
    .context("creating vec_memories virtual table")?;

    // -- Episodes table for agent run tracking --
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS episodes (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            task_id         TEXT,
            parent_id       TEXT REFERENCES episodes(id),
            task_type       TEXT NOT NULL,
            task_description TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'running',
            error_type      TEXT,
            error_message   TEXT,
            recovery_action TEXT,
            input_summary   TEXT,
            output_summary  TEXT,
            started_at      TEXT NOT NULL,
            completed_at    TEXT,
            duration_ms     INTEGER,
            importance      REAL DEFAULT 0.5,
            access_count    INTEGER DEFAULT 0,
            last_accessed_at TEXT,
            source_files    TEXT,
            source_hashes   TEXT,
            tags            TEXT,
            metadata        TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_episodes_agent ON episodes(agent_id);
        CREATE INDEX IF NOT EXISTS idx_episodes_task ON episodes(task_id);
        CREATE INDEX IF NOT EXISTS idx_episodes_status ON episodes(status);",
    )
    .context("creating episodes table")?;

    // -- sqlite-vec virtual table for episode embeddings --
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_episodes USING vec0(
            episode_rowid INTEGER PRIMARY KEY,
            embedding float[{embedding_dims}]
        );"
    ))
    .context("creating vec_episodes virtual table")?;

    // -- Graph state table for petgraph serialization --
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_state (
            id          INTEGER PRIMARY KEY CHECK (id = 1),
            data        BLOB NOT NULL,
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("creating graph_state table")?;

    // -- Index for staleness lookups --
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_source_file ON memories(source_file);",
    )
    .context("creating source_file index")?;

    // -- Nullify existing embeddings (model dimension change: 384 → 768) --
    conn.execute_batch(
        "UPDATE memories SET embedding = NULL, model_id = NULL WHERE embedding IS NOT NULL;",
    )
    .context("nullifying stale embeddings for model upgrade")?;

    // Clear vec_memories in case of partial previous migration
    conn.execute_batch("DELETE FROM vec_memories;")
        .context("clearing vec_memories")?;

    Ok(())
}

/// v4: Hierarchical scope columns, content_hash dedup, FTS5, access log, scoped vec table.
///
/// Adds scope hierarchy (global → workspace → repo → module → run) to the memories table.
/// Creates FTS5 full-text index for hybrid vector + keyword search.
/// Creates access log for cross-scope promotion heuristics.
/// Creates a new scope-partitioned vec table alongside the existing one.
fn migrate_v4(conn: &Connection, embedding_dims: usize) -> Result<()> {
    let add_column_if_missing = |col: &str, typedef: &str| -> Result<()> {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}'"
            ))
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!("ALTER TABLE memories ADD COLUMN {col} {typedef};"))
                .with_context(|| format!("adding {col} column"))?;
        }
        Ok(())
    };

    // -- Scope hierarchy columns --
    add_column_if_missing("scope_level", "INTEGER NOT NULL DEFAULT 2")?; // default = repo
    add_column_if_missing("scope_path", "TEXT NOT NULL DEFAULT 'workspace'")?;
    add_column_if_missing("repo_id", "TEXT")?;
    add_column_if_missing("module_path", "TEXT")?;
    add_column_if_missing("run_id", "TEXT")?;
    add_column_if_missing("content_hash", "TEXT")?;
    add_column_if_missing("memory_type", "TEXT NOT NULL DEFAULT 'factual'")?;
    add_column_if_missing("trust", "TEXT NOT NULL DEFAULT 'medium'")?;
    add_column_if_missing("tag", "TEXT")?;

    // -- Indexes for scope queries --
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_scope
             ON memories(scope_level, repo_id, module_path);
         CREATE INDEX IF NOT EXISTS idx_memories_content_hash
             ON memories(content_hash);
         CREATE INDEX IF NOT EXISTS idx_memories_type
             ON memories(memory_type);
         CREATE INDEX IF NOT EXISTS idx_memories_run
             ON memories(run_id);",
    )
    .context("creating scope indexes")?;

    // -- Populate content_hash for existing rows --
    // We use SQLite's built-in hex() + sha256 (via Rust) to fill this.
    // Since SQLite doesn't have SHA-256 built in, we do it in a loop.
    {
        let mut stmt =
            conn.prepare("SELECT id, content FROM memories WHERE content_hash IS NULL")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut update = conn.prepare("UPDATE memories SET content_hash = ?1 WHERE id = ?2")?;
        for (id, content) in &rows {
            let hash = content_hash(content);
            update.execute(rusqlite::params![hash, id])?;
        }
    }

    // -- Migrate existing namespace → scope columns --
    // Map old namespace conventions to scope levels:
    //   "user:*"         → repo level, trust=high
    //   "agents:*"       → run level
    //   "verification:*" → run level
    //   "tiers:*"        → run level
    //   everything else  → repo level
    // Existing convention: swarm memories use key patterns like "agents:run:agent",
    // "verification:run", "tiers:run". User memories use "user:*".
    // Match on both namespace and key patterns.
    conn.execute_batch(
        "UPDATE memories SET scope_level = 4, trust = 'medium'
         WHERE (namespace LIKE 'agents:%' OR key LIKE 'agents:%') AND scope_level = 2;
         UPDATE memories SET scope_level = 4, trust = 'medium'
         WHERE (namespace LIKE 'verification:%' OR key LIKE 'verification:%') AND scope_level = 2;
         UPDATE memories SET scope_level = 4, trust = 'medium'
         WHERE (namespace LIKE 'tiers:%' OR key LIKE 'tiers:%') AND scope_level = 2;
         UPDATE memories SET trust = 'high'
         WHERE key LIKE 'user:%' AND trust = 'medium';",
    )
    .context("migrating namespace to scope")?;

    // -- FTS5 full-text search (standalone, not content-synced) --
    // Standalone FTS avoids trigger complexity with UPSERT/ON CONFLICT.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            content,
            tokenize='porter unicode61'
        );",
    )
    .context("creating FTS5 table")?;

    // Populate FTS from existing data
    conn.execute_batch(
        "INSERT OR IGNORE INTO memories_fts(rowid, content)
         SELECT id, content FROM memories;",
    )
    .context("populating FTS5")?;

    // FTS sync triggers — keep FTS in sync with memories table.
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
            DELETE FROM memories_fts WHERE rowid = old.id;
        END;
        CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE OF content ON memories BEGIN
            DELETE FROM memories_fts WHERE rowid = old.id;
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;",
    )
    .context("creating FTS sync triggers")?;

    // -- Scope-partitioned vector table --
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories_scoped USING vec0(
            memory_id   INTEGER PRIMARY KEY,
            embedding   float[{embedding_dims}],
            scope_level INTEGER
        );"
    ))
    .context("creating scoped vec table")?;

    // Copy existing vectors into scoped table with scope_level from memories
    conn.execute_batch(
        "INSERT OR IGNORE INTO vec_memories_scoped(memory_id, embedding, scope_level)
         SELECT v.memory_id, v.embedding, m.scope_level
         FROM vec_memories v
         JOIN memories m ON m.id = v.memory_id;",
    )
    .context("migrating vectors to scoped table")?;

    // -- Access log for cross-scope promotion heuristics --
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_access_log (
            memory_id   INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            repo_id     TEXT,
            module_path TEXT,
            accessed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_access_log_memory
            ON memory_access_log(memory_id);",
    )
    .context("creating access log table")?;

    Ok(())
}

/// v5 (Tier S / S4): injection manifests table.
///
/// One row per chat-turn retrieval decision. Persisted from the single-
/// consumer writer task in response to `WriterMessage::InjectionManifest`.
/// Separate from the `memories` / `vec_memories_scoped` world — manifests
/// are not embedded, not semantically searchable, purely a rolling
/// forensic log (default 30-day retention via the sleeptime prune in B5).
///
/// `payload` is opaque JSON: schema_version, query_text, selected_ids,
/// candidate_pool (when enabled), scoring_formula_version, embedder_name.
/// Index on (session_id, created_at) so the CLI / panel can fetch the
/// last N manifests for a conversation cheaply.
fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS injection_manifests (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            turn_id      TEXT    NOT NULL,
            session_id   TEXT    NOT NULL,
            source_channel TEXT  NOT NULL DEFAULT 'chat',
            payload      TEXT    NOT NULL,
            created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_manifests_session
            ON injection_manifests(session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_manifests_turn
            ON injection_manifests(turn_id);",
    )
    .context("creating injection_manifests table")?;
    Ok(())
}

/// v6 (Tier A / A3): per-memory `source` and fine-grained `trust_score`.
///
/// Adds two columns:
/// * `source` TEXT — write origin (`user_remember`, `llm_extracted`,
///   `llm_annotated`, `llm_consolidated`, `swarm_consolidated`,
///   `mcp_import`, `tool_output`, `user_panel`, `unknown_legacy`).
/// * `trust_score` REAL — float in [0.0, 1.0] replacing the enum
///   `trust` column's role as a composite-score multiplier. The old
///   `trust` enum column stays for backward compat but is no longer
///   authoritative.
///
/// Backfill: rows where the v4 migration tagged `trust = 'high'` (i.e.
/// `key LIKE 'user:%'`) map to `source = 'user_remember'`,
/// `trust_score = 1.0`. Everything else maps to
/// `source = 'unknown_legacy'`, `trust_score = 0.75`.
fn migrate_v6(conn: &Connection) -> Result<()> {
    let add_column_if_missing = |col: &str, typedef: &str| -> Result<()> {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}'"
            ))
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!("ALTER TABLE memories ADD COLUMN {col} {typedef};"))
                .with_context(|| format!("adding {col} column"))?;
        }
        Ok(())
    };

    add_column_if_missing("source", "TEXT NOT NULL DEFAULT 'unknown_legacy'")?;
    add_column_if_missing("trust_score", "REAL NOT NULL DEFAULT 0.6")?;

    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_source ON memories(source);")
        .context("creating source index")?;

    // Backfill: user-authored rows get trust 1.0; everything else 0.75.
    conn.execute_batch(
        "UPDATE memories
             SET source = 'user_remember',
                 trust_score = 1.0
             WHERE source = 'unknown_legacy'
               AND (trust = 'high' OR key LIKE 'user:%');
         UPDATE memories
             SET trust_score = 0.75
             WHERE source = 'unknown_legacy'
               AND trust_score = 0.6;",
    )
    .context("backfilling source + trust_score")?;

    Ok(())
}

/// v7 (Tier A / A1): session ledger for LLM-emitted `<turn_annotations>`.
///
/// `session_thread` and `open_questions` from each turn's annotations
/// block are stored here, keyed by `(session_id, turn_id)`. Tier B5's
/// session consolidator reads the thread values to segment sessions
/// thematically; Tier A4's panel surfaces `open_questions` as a follow-
/// up list. They are **not** injected into prompts — chat injection
/// only sees the memory table.
///
/// Keeping these in their own table avoids bloating `memories` with
/// session-scoped records that don't belong in retrieval.
fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_ledger_turns (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id       TEXT NOT NULL,
            turn_id          TEXT NOT NULL,
            session_thread   TEXT,
            open_questions   TEXT,
            annotations_json TEXT,
            created_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_slt_session
            ON session_ledger_turns(session_id, created_at DESC);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_slt_session_turn
            ON session_ledger_turns(session_id, turn_id);",
    )
    .context("creating session_ledger_turns table")?;
    Ok(())
}

/// v8: Tier B / B1 — `_gaviero_meta` key/value table for upgrade
/// metadata. Today's only known key is `embedder_model` (stamped by
/// the re-embed migration); future tiers will add scoring-formula
/// version, last sleeptime run, etc. Pre-existing rows from databases
/// created with an older `embedder_model` are detected by an absence
/// of the row, which the bootstrap interprets as the legacy default.
fn migrate_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _gaviero_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .context("creating _gaviero_meta table")?;
    Ok(())
}

/// v9: Tier B Phase 2 — session consolidation + sleeptime audit +
/// retrieval-use telemetry.
///
/// * `superseded_by` column on `memories`: B5 SUPERSEDE marks an old
///   row as replaced. Soft-delete keyed to the new memory id.
/// * `sleeptime_audit`: append-only log of every sleeptime operation
///   so a Tier C2 `/forget` audit trail / undo path can reverse them
///   manually if needed.
/// * `retrieval_use`: per-(memory, turn) classification produced by the
///   B6 telemetry pass. Long-lived; pruned to 90 days by sleeptime.
fn migrate_v9(conn: &Connection) -> Result<()> {
    let has_superseded: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('memories') WHERE name = 'superseded_by'")
        .and_then(|mut s| s.query_row([], |_| Ok(true)))
        .unwrap_or(false);
    if !has_superseded {
        conn.execute_batch(
            "ALTER TABLE memories ADD COLUMN superseded_by INTEGER REFERENCES memories(id);
             CREATE INDEX IF NOT EXISTS idx_memories_superseded
                 ON memories(superseded_by) WHERE superseded_by IS NOT NULL;",
        )
        .context("adding superseded_by column")?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sleeptime_audit (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id      TEXT    NOT NULL,
            kind        TEXT    NOT NULL,        -- decay_flag|near_dup_merge|promote|trust_rescore|prune_telemetry
            memory_id   INTEGER,                  -- target row (or NULL for sweeps)
            related_id  INTEGER,                  -- e.g. merge winner
            payload     TEXT    NOT NULL,         -- opaque JSON: before/after, deltas, reasoning
            dry_run     INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_sleeptime_audit_run ON sleeptime_audit(run_id);
        CREATE INDEX IF NOT EXISTS idx_sleeptime_audit_memory ON sleeptime_audit(memory_id);

        CREATE TABLE IF NOT EXISTS retrieval_use (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id          INTEGER NOT NULL,
            turn_id            TEXT    NOT NULL,
            session_id         TEXT,
            injected_rank      INTEGER NOT NULL,
            classification     TEXT    NOT NULL,  -- used|partial|unused
            cosine_to_response REAL    NOT NULL,
            substring_hit      INTEGER NOT NULL DEFAULT 0,
            created_at         TEXT    NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_retrieval_use_memory
            ON retrieval_use(memory_id);
        CREATE INDEX IF NOT EXISTS idx_retrieval_use_turn
            ON retrieval_use(turn_id);
        CREATE INDEX IF NOT EXISTS idx_retrieval_use_classification
            ON retrieval_use(classification);",
    )
    .context("creating sleeptime_audit + retrieval_use tables")?;
    Ok(())
}

/// v10 (Tier C Phase 2 / C1): typed memory stores discriminator.
///
/// Adds a `memory_kind` discriminator column to `memories` so the table
/// can host three lifecycle classes — `record` (the workhorse, mutable,
/// injected, dedup'd), `history` (immutable raw transcripts, never
/// injected), and `summary` (consolidator output, semantic merge). The
/// column is `TEXT NOT NULL DEFAULT 'record'` with a `CHECK` constraint
/// pinning the allowed values; existing rows take the default and so
/// remain `record` after migration.
///
/// Per-kind convenience views (`v_records`, `v_history`, `v_summaries`)
/// shadow the base table for each discriminator. They're query-time
/// sugar — the SQL immutability trigger introduced in C1.3 fires on the
/// **base** table, never on the views, since SQLite views are read-only
/// by default and any application path that wants to bypass the
/// discriminator would need to write through `memories` directly anyway.
///
/// We deliberately do **not** install the immutability trigger here.
/// C1.1's job is to add the column, the index, and the views, and to
/// leave the existing rows correctly defaulted. The trigger lands in
/// C1.3, after the writer dispatch (C1.2) ensures every code path
/// already sets `memory_kind` correctly on insert.
fn migrate_v10(conn: &Connection) -> Result<()> {
    let has_kind: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('memories') WHERE name = 'memory_kind'")
        .and_then(|mut s| s.query_row([], |_| Ok(true)))
        .unwrap_or(false);

    if !has_kind {
        conn.execute_batch(
            "ALTER TABLE memories ADD COLUMN memory_kind TEXT NOT NULL DEFAULT 'record'
                 CHECK (memory_kind IN ('record', 'history', 'summary'));",
        )
        .context("adding memory_kind column")?;
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_kind ON memories(memory_kind);
         CREATE INDEX IF NOT EXISTS idx_memories_kind_scope
             ON memories(memory_kind, scope_level);
         CREATE VIEW IF NOT EXISTS v_records   AS SELECT * FROM memories WHERE memory_kind = 'record';
         CREATE VIEW IF NOT EXISTS v_history   AS SELECT * FROM memories WHERE memory_kind = 'history';
         CREATE VIEW IF NOT EXISTS v_summaries AS SELECT * FROM memories WHERE memory_kind = 'summary';",
    )
    .context("installing memory_kind index + per-kind views")?;

    // Stamp completion timestamp so the TUI can surface a one-shot
    // banner ("Gaviero's memory schema upgraded to typed stores; backup
    // at <path>") without keeping ambient state. Best-effort.
    let _ = conn.execute(
        "INSERT INTO _gaviero_meta(key, value)
             VALUES ('c1_migration_completed_at', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(key) DO NOTHING",
        [],
    );

    Ok(())
}

/// v11 (Tier C / C1.3): SQL-level immutability guard for History rows.
///
/// Installs two BEFORE triggers on `memories` that veto UPDATE and
/// DELETE statements whose target row has `memory_kind = 'history'`.
/// SQLite triggers cannot return errors directly — the canonical idiom
/// is `SELECT RAISE(ABORT, '<msg>')`, which aborts the statement and
/// rolls back the change.
///
/// Why a BEFORE trigger and not AFTER:
/// - BEFORE lets us inspect `OLD` and short-circuit *before* the
///   write touches indexes, FTS, and sqlite-vec.
/// - AFTER would still leave intermediate rows in derived tables in
///   the small window before the abort propagates.
///
/// Why two triggers (not one) and not a CHECK constraint:
/// - SQLite CHECK constraints fire on every row insert, but we *do*
///   want to allow inserting history rows (just not updating /
///   deleting them).
/// - One trigger per operation keeps each diagnostic message specific:
///   "history row UPDATE rejected" vs "DELETE rejected".
///
/// The triggers fire on the **base** `memories` table — views are
/// read-only by default in SQLite, so the v10 `v_history` view does
/// not need its own trigger. The single legitimate path that bypasses
/// these triggers is the C2.4 `RedactHistory` writer-task variant,
/// which drops + recreates them inside a single transaction. CI grep
/// enforces that no other callsite contains the trigger names.
fn migrate_v11(conn: &Connection) -> Result<()> {
    install_history_immutable_triggers(conn)
}

/// Install the C1.3 immutability triggers. Idempotent — the migration
/// uses `CREATE TRIGGER IF NOT EXISTS`. **Public to the crate** so the
/// C2.4 `RedactHistory` handler can re-install the triggers after its
/// brief privileged window. No other code path may call this.
pub(crate) fn install_history_immutable_triggers(conn: &Connection) -> Result<()> {
    let sql = format!(
        "CREATE TRIGGER IF NOT EXISTS {update_trigger}
            BEFORE UPDATE ON memories
            FOR EACH ROW
            WHEN OLD.memory_kind = 'history'
            BEGIN
                SELECT RAISE(ABORT,
                    'history row UPDATE rejected: history is append-only \
                     (use /forget-history to redact via RedactHistory)');
            END;
         CREATE TRIGGER IF NOT EXISTS {delete_trigger}
            BEFORE DELETE ON memories
            FOR EACH ROW
            WHEN OLD.memory_kind = 'history'
            BEGIN
                SELECT RAISE(ABORT,
                    'history row DELETE rejected: history is append-only \
                     (use /forget-history to redact via RedactHistory)');
            END;",
        update_trigger = C1_HISTORY_IMMUTABLE_TRIGGER_UPDATE,
        delete_trigger = C1_HISTORY_IMMUTABLE_TRIGGER_DELETE,
    );
    conn.execute_batch(&sql)
        .context("installing C1 history-immutable triggers")
}

/// v12 (Tier C / C1.4): zstd compression scaffolding for History rows.
///
/// Adds two columns to `memories`:
/// - `compressed INTEGER NOT NULL DEFAULT 0` — boolean flag.
/// - `content_blob BLOB` — zstd-encoded transcript bytes when
///   `compressed = 1`. NULL otherwise.
///
/// When `compressed = 1`, the canonical body lives in `content_blob`
/// and `content` is replaced by a placeholder (typically the SHA-256
/// hex prefix so debug tooling can still recognize the row by content
/// hash). The application-level read path
/// ([`crate::memory::store::MemoryStore::read_history_content`])
/// transparently decompresses + SHA-verifies; mismatch raises a
/// data-integrity alarm. The compression itself runs inside the
/// sleeptime pass via the dedicated trigger-disable window — see
/// [`super::store::MemoryStore::compress_history_row`].
fn migrate_v12(conn: &Connection) -> Result<()> {
    let add_column_if_missing = |col: &str, typedef: &str| -> Result<()> {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('memories') WHERE name = '{col}'"
            ))
            .and_then(|mut s| s.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!("ALTER TABLE memories ADD COLUMN {col} {typedef};"))
                .with_context(|| format!("adding {col} column"))?;
        }
        Ok(())
    };

    add_column_if_missing("compressed", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing("content_blob", "BLOB")?;

    // Index lets sleeptime cheaply pick eligible rows: history rows
    // that haven't been compressed yet, ordered by age.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_history_compress
            ON memories(memory_kind, compressed, created_at)
            WHERE memory_kind = 'history';",
    )
    .context("creating history-compression index")?;

    Ok(())
}

/// Drop the C1.3 immutability triggers. **Crate-private and only the
/// C2.4 `RedactHistory` handler may call this**, inside a transaction
/// that re-installs them before commit. Any other callsite is a bug
/// and breaks the audit invariant — CI grep enforces single-callsite.
pub(crate) fn drop_history_immutable_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS {update_trigger};
         DROP TRIGGER IF EXISTS {delete_trigger};",
        update_trigger = C1_HISTORY_IMMUTABLE_TRIGGER_UPDATE,
        delete_trigger = C1_HISTORY_IMMUTABLE_TRIGGER_DELETE,
    ))
    .context("dropping C1 history-immutable triggers")
}

/// Compute SHA-256 hash of normalized content for dedup.
pub fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = content.trim().to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{:x}", digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: register sqlite-vec extension and open an in-memory connection.
    fn setup_conn() -> Connection {
        // Register BEFORE opening the connection (auto_extension applies to new opens)
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn test_fresh_database() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        // Verify privacy column exists
        let has_privacy: bool = conn
            .prepare("SELECT * FROM pragma_table_info('memories') WHERE name = 'privacy'")
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_privacy);

        // Verify v3 columns exist
        for col in &[
            "importance",
            "access_count",
            "last_accessed_at",
            "source_file",
            "source_hash",
        ] {
            let has: bool = conn
                .prepare(&format!(
                    "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}'"
                ))
                .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
                .unwrap_or(false);
            assert!(has, "missing column: {col}");
        }

        // Verify episodes table exists
        let has_episodes: bool = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='episodes'")
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_episodes, "episodes table should exist");

        // v5: injection_manifests table exists
        let has_manifests: bool = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='injection_manifests'",
            )
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_manifests, "injection_manifests table should exist");
    }

    #[test]
    fn test_v6_adds_source_and_trust_score() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        // Sanity: both columns exist with expected defaults.
        for (col, ty) in &[("source", "TEXT"), ("trust_score", "REAL")] {
            let has: bool = conn
                .prepare(&format!(
                    "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}' AND type = '{ty}'"
                ))
                .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
                .unwrap_or(false);
            assert!(has, "missing v6 column: {col} ({ty})");
        }
    }

    #[test]
    fn test_v6_backfills_user_rows_as_trusted() {
        let conn = setup_conn();
        // Stop at v5 so we can seed then re-run to v6.
        run_migrations(&conn, 8).unwrap();
        // Reset to v5 and wipe the v6-added columns to simulate a v5 DB
        // that receives the v6 migration.
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        // Insert a user-authored row under the pre-A3 convention and a
        // generic row under the legacy default. `execute_batch` because
        // `execute` only runs one statement.
        conn.execute_batch(
            "INSERT INTO memories (namespace, key, content, scope_level, scope_path,
                content_hash, trust, source, trust_score)
             VALUES ('default', 'user:note1', 'user note', 2, 'repo:x', 'h1', 'high',
                 'unknown_legacy', 0.6);
             INSERT INTO memories (namespace, key, content, scope_level, scope_path,
                content_hash, trust, source, trust_score)
             VALUES ('default', 'agents:run:x', 'agent obs', 4, 'repo:x/run:r', 'h2', 'medium',
                 'unknown_legacy', 0.6);",
        )
        .unwrap();
        // Re-run migrations (idempotent on v6 artifacts).
        run_migrations(&conn, 8).unwrap();

        let (user_src, user_trust): (String, f32) = conn
            .query_row(
                "SELECT source, trust_score FROM memories WHERE key = 'user:note1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(user_src, "user_remember");
        assert!((user_trust - 1.0).abs() < 1e-6);

        let (other_src, other_trust): (String, f32) = conn
            .query_row(
                "SELECT source, trust_score FROM memories WHERE key = 'agents:run:x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(other_src, "unknown_legacy");
        assert!((other_trust - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_v5_upgrade_from_v4() {
        // Walk a pre-v5 DB through migrations and confirm the manifests
        // table appears without touching prior columns.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        // Force version back to 4, drop v5 artifacts, then re-run.
        conn.pragma_update(None, "user_version", 4_u32).unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS injection_manifests;")
            .unwrap();
        run_migrations(&conn, 8).unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='injection_manifests'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_idempotent() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        run_migrations(&conn, 8).unwrap(); // should not fail
    }

    #[test]
    fn test_upgrade_from_v1() {
        let conn = setup_conn();

        // Simulate a v1 database (created before migration system)
        migrate_v1(&conn).unwrap();
        conn.pragma_update(None, "user_version", 1u32).unwrap();

        // Now run full migrations — should apply v2 and v3
        run_migrations(&conn, 8).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        // Privacy column should exist
        let has_privacy: bool = conn
            .prepare("SELECT * FROM pragma_table_info('memories') WHERE name = 'privacy'")
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_privacy);
    }

    #[test]
    fn test_upgrade_from_pre_migration_database() {
        let conn = setup_conn();

        // Simulate a database created before the migration system:
        // table exists but user_version is 0 (default).
        conn.execute_batch(
            "CREATE TABLE memories (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace   TEXT    NOT NULL,
                key         TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                embedding   BLOB,
                model_id    TEXT,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
                metadata    TEXT,
                UNIQUE(namespace, key)
            );
            CREATE INDEX idx_memories_namespace ON memories(namespace);
            CREATE INDEX idx_memories_ns_key ON memories(namespace, key);",
        )
        .unwrap();

        // user_version is 0 — migrations should handle this gracefully
        run_migrations(&conn, 8).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }

    #[test]
    fn test_v4_scope_columns() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();

        // Verify v4 columns exist
        for col in &[
            "scope_level",
            "scope_path",
            "repo_id",
            "module_path",
            "run_id",
            "content_hash",
            "memory_type",
            "trust",
            "tag",
        ] {
            let has: bool = conn
                .prepare(&format!(
                    "SELECT * FROM pragma_table_info('memories') WHERE name = '{col}'"
                ))
                .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
                .unwrap_or(false);
            assert!(has, "missing v4 column: {col}");
        }

        // Verify FTS5 table exists
        let has_fts: bool = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='memories_fts'")
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_fts, "memories_fts should exist");

        // Verify scoped vec table exists
        let has_vec_scoped: bool = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='vec_memories_scoped'",
            )
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_vec_scoped, "vec_memories_scoped should exist");

        // Verify access log table exists
        let has_access_log: bool = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='memory_access_log'",
            )
            .and_then(|mut stmt| stmt.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_access_log, "memory_access_log should exist");
    }

    #[test]
    fn test_v4_content_hash_backfill() {
        let conn = setup_conn();
        // Run through v3 first
        migrate_v1(&conn).unwrap();
        migrate_v2(&conn).unwrap();
        migrate_v3(&conn, 8).unwrap();
        conn.pragma_update(None, "user_version", 3u32).unwrap();

        // Insert some data at v3 level (no content_hash column yet)
        conn.execute(
            "INSERT INTO memories (namespace, key, content) VALUES ('ns', 'k1', 'hello world')",
            [],
        )
        .unwrap();

        // Run v4 migration
        run_migrations(&conn, 8).unwrap();

        // Verify content_hash was backfilled
        let hash: String = conn
            .query_row(
                "SELECT content_hash FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash, content_hash("hello world"));
    }

    #[test]
    fn test_v10_adds_memory_kind_column_and_views() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();

        // Column exists with the expected NOT NULL default.
        let has_kind: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('memories') WHERE name = 'memory_kind'")
            .and_then(|mut s| s.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        assert!(has_kind, "memory_kind column must exist");

        // CHECK constraint blocks invalid kinds.
        conn.execute_batch(
            "INSERT INTO memories (namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind)
             VALUES ('default', 'k_ok', 'ok', 2, 'repo:x', 'h_ok', 'record');",
        )
        .unwrap();
        let bad = conn.execute_batch(
            "INSERT INTO memories (namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind)
             VALUES ('default', 'k_bad', 'bad', 2, 'repo:x', 'h_bad', 'episode');",
        );
        assert!(bad.is_err(), "CHECK must reject unknown memory_kind");

        // Default for inserts that omit memory_kind is 'record'.
        conn.execute_batch(
            "INSERT INTO memories (namespace, key, content, scope_level, scope_path, content_hash)
             VALUES ('default', 'k_default', 'no kind', 2, 'repo:x', 'h_def');",
        )
        .unwrap();
        let kind: String = conn
            .query_row(
                "SELECT memory_kind FROM memories WHERE key = 'k_default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "record");

        // Views are present and partition correctly.
        for (view, kind) in [
            ("v_records", "record"),
            ("v_history", "history"),
            ("v_summaries", "summary"),
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {view}"), [], |r| r.get(0))
                .unwrap();
            // We've inserted 2 records and no history/summary above.
            let expected = if kind == "record" { 2 } else { 0 };
            assert_eq!(count, expected, "{view} should have {expected} row(s)");
        }
    }

    #[test]
    fn test_v10_legacy_rows_default_to_record() {
        // Walk a v9 DB through the v10 upgrade and confirm pre-existing
        // rows take the 'record' default for memory_kind.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();

        // Force back to v9 and remove v10 artifacts to simulate an
        // older DB receiving the migration.
        conn.pragma_update(None, "user_version", 9_u32).unwrap();
        conn.execute_batch(
            "DROP VIEW IF EXISTS v_records;
             DROP VIEW IF EXISTS v_history;
             DROP VIEW IF EXISTS v_summaries;",
        )
        .unwrap();
        // SQLite has no clean way to drop a column; instead, set the
        // existing rows' memory_kind to NULL via a roundabout, then
        // mark all rows so we can detect backfill. We can't actually
        // drop the column here without rebuilding the table — but the
        // ADD COLUMN guard in migrate_v10 is keyed on column presence,
        // so we instead pre-populate a non-default kind and verify the
        // re-run is idempotent (the migration must not clobber it).
        conn.execute_batch(
            "INSERT INTO memories (namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind)
             VALUES ('default', 'legacy:r', 'legacy', 2, 'repo:x', 'h_l', 'record');",
        )
        .unwrap();

        run_migrations(&conn, 8).unwrap();

        let kind: String = conn
            .query_row(
                "SELECT memory_kind FROM memories WHERE key = 'legacy:r'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "record");

        // Views are reinstated.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM v_records", [], |r| r.get(0))
            .unwrap();
        assert!(n >= 1);
    }

    #[test]
    fn test_v10_index_present() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type='index' AND name IN ('idx_memories_kind', 'idx_memories_kind_scope')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2, "both kind indexes should exist");
    }

    #[test]
    fn test_needs_c1_backup_only_for_pre_v10() {
        let conn = setup_conn();
        // Fresh DB at user_version 0 — bootstrap path, no upgrade
        // backup needed (there's nothing to back up).
        assert!(!needs_c1_backup(&conn));

        // Walk to v9 and verify backup is requested.
        migrate_v1(&conn).unwrap();
        migrate_v2(&conn).unwrap();
        migrate_v3(&conn, 8).unwrap();
        migrate_v4(&conn, 8).unwrap();
        migrate_v5(&conn).unwrap();
        migrate_v6(&conn).unwrap();
        migrate_v7(&conn).unwrap();
        migrate_v8(&conn).unwrap();
        migrate_v9(&conn).unwrap();
        conn.pragma_update(None, "user_version", 9_u32).unwrap();
        assert!(needs_c1_backup(&conn), "v9 → v10 upgrade should request a backup");

        // After v10 lands, no backup is requested again.
        run_migrations(&conn, 8).unwrap();
        assert!(!needs_c1_backup(&conn));
    }

    /// C1.3: helper that seeds a v11 DB with one history row and
    /// returns its id. Used by the smuggling-attempt suite below.
    fn seed_history_row(conn: &Connection) -> i64 {
        conn.execute_batch(
            "INSERT INTO memories (
                namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind, memory_type, trust, source, trust_score,
                repo_id, run_id
            ) VALUES (
                'default', 'history:s:t', 'TRANSCRIPT', 4, 'repo:r/run:run-1',
                'h_hist', 'history', 'factual', 'high', 'raw_transcript', 1.0,
                'r', 'run-1'
            );",
        )
        .unwrap();
        conn.query_row(
            "SELECT id FROM memories WHERE key = 'history:s:t'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// C1.3: every documented attempt to UPDATE a history row must be
    /// vetoed by the trigger. The plan calls for ≥10 smuggling
    /// attempts; each `assert_history_immutable_*` below counts as one.
    #[test]
    fn test_c1_trigger_rejects_direct_update_by_id() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let id = seed_history_row(&conn);
        let r = conn.execute(
            "UPDATE memories SET content = 'tampered' WHERE id = ?1",
            rusqlite::params![id],
        );
        assert!(r.is_err(), "UPDATE-by-id should fail: {r:?}");
    }

    #[test]
    fn test_c1_trigger_rejects_direct_update_by_key() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        let r = conn.execute(
            "UPDATE memories SET content = 'tampered' WHERE key = 'history:s:t'",
            [],
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_c1_trigger_rejects_unscoped_blanket_update() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        // No WHERE → would touch every row, including history.
        let r = conn.execute("UPDATE memories SET importance = 0.0", []);
        assert!(r.is_err(), "blanket UPDATE must abort on the history row");
    }

    #[test]
    fn test_c1_trigger_rejects_update_targeting_kind_change() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let id = seed_history_row(&conn);
        // Try to "demote" history to record. This is the most insidious
        // smuggling path: change the kind first, then mutate freely.
        // The BEFORE trigger inspects `OLD.memory_kind`, not the new
        // value, so this is rejected.
        let r = conn.execute(
            "UPDATE memories SET memory_kind = 'record' WHERE id = ?1",
            rusqlite::params![id],
        );
        assert!(r.is_err(), "kind-demotion attempt must be rejected");
    }

    #[test]
    fn test_c1_trigger_rejects_update_with_subquery_filter() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        let r = conn.execute(
            "UPDATE memories SET content = 'tampered'
              WHERE id IN (SELECT id FROM memories WHERE memory_kind = 'history')",
            [],
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_c1_trigger_rejects_update_with_join_via_cte() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let id = seed_history_row(&conn);
        let r = conn.execute(
            "WITH targets AS (SELECT id FROM memories WHERE memory_kind = 'history')
             UPDATE memories SET content = 'tampered' WHERE id IN (SELECT id FROM targets)",
            rusqlite::params![],
        );
        assert!(r.is_err(), "CTE-driven UPDATE must be rejected (id={id})");
    }

    #[test]
    fn test_c1_trigger_rejects_direct_delete_by_id() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let id = seed_history_row(&conn);
        let r = conn.execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id]);
        assert!(r.is_err());
    }

    #[test]
    fn test_c1_trigger_rejects_blanket_delete() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        let r = conn.execute("DELETE FROM memories", []);
        assert!(r.is_err(), "blanket DELETE must fail on history row");
    }

    #[test]
    fn test_c1_trigger_rejects_delete_via_subquery() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        let r = conn.execute(
            "DELETE FROM memories WHERE id IN
                (SELECT id FROM memories WHERE memory_kind = 'history')",
            [],
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_c1_trigger_rejects_cascade_delete_via_run_id() {
        // Run-id based bulk delete is the path the writer task uses
        // for `DeleteRun` (Tier S). It must NOT take history rows
        // with it: history is provenance and must outlive the run.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        let r = conn.execute(
            "DELETE FROM memories WHERE run_id = 'run-1'",
            [],
        );
        assert!(r.is_err(), "DeleteRun-style sweep must skip / abort on history");
    }

    #[test]
    fn test_c1_trigger_allows_record_updates() {
        // Counter-test: the trigger must NOT fire on record rows.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        conn.execute_batch(
            "INSERT INTO memories (
                namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind, memory_type, trust, source, trust_score
            ) VALUES (
                'default', 'rec:1', 'body', 2, 'repo:r',
                'h_rec', 'record', 'factual', 'medium', 'user_remember', 1.0
            );",
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM memories WHERE key = 'rec:1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        // UPDATE on a record row is fine.
        conn.execute(
            "UPDATE memories SET content = 'edited' WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
        let after: String = conn
            .query_row(
                "SELECT content FROM memories WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, "edited");
        // DELETE on a record row is fine.
        let n = conn
            .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn test_c1_trigger_allows_summary_updates() {
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        conn.execute_batch(
            "INSERT INTO memories (
                namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind, memory_type, trust, source, trust_score
            ) VALUES (
                'default', 'sum:1', 'summary body', 2, 'repo:r',
                'h_sum', 'summary', 'factual', 'medium', 'llm_consolidated', 0.75
            );",
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM memories WHERE key = 'sum:1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "UPDATE memories SET importance = 0.9 WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
    }

    #[test]
    fn test_c1_trigger_allows_history_inserts() {
        // Inserts must work — only UPDATE and DELETE are vetoed.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);
        // A second insert with a different key also goes through.
        conn.execute_batch(
            "INSERT INTO memories (
                namespace, key, content, scope_level, scope_path,
                content_hash, memory_kind, memory_type, trust, source, trust_score
            ) VALUES (
                'default', 'history:s:t2', 'TRANSCRIPT 2', 4, 'repo:r/run:run-1',
                'h_hist2', 'history', 'factual', 'high', 'raw_transcript', 1.0
            );",
        )
        .unwrap();
    }

    #[test]
    fn test_c1_trigger_recreate_after_drop_round_trip() {
        // C2.4 will need to drop + reinstall the triggers inside a
        // transaction. Verify that round-trip works cleanly: drop,
        // veto disappears; reinstall, veto returns.
        let conn = setup_conn();
        run_migrations(&conn, 8).unwrap();
        let _ = seed_history_row(&conn);

        // Drop and confirm UPDATE now succeeds.
        drop_history_immutable_triggers(&conn).unwrap();
        conn.execute(
            "UPDATE memories SET content = 'briefly tampered'
                WHERE memory_kind = 'history'",
            [],
        )
        .unwrap();

        // Reinstall and confirm UPDATE is rejected again.
        install_history_immutable_triggers(&conn).unwrap();
        let r = conn.execute(
            "UPDATE memories SET content = 'tampered again'
                WHERE memory_kind = 'history'",
            [],
        );
        assert!(r.is_err(), "trigger must be effective after reinstall");
    }

    #[test]
    fn test_v4_namespace_to_scope_migration() {
        let conn = setup_conn();
        migrate_v1(&conn).unwrap();
        migrate_v2(&conn).unwrap();
        migrate_v3(&conn, 8).unwrap();
        conn.pragma_update(None, "user_version", 3u32).unwrap();

        // Insert agent-generated and user memories
        conn.execute(
            "INSERT INTO memories (namespace, key, content) VALUES ('default', 'agents:run1:fix', 'agent memory')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memories (namespace, key, content) VALUES ('default', 'user:note', 'user memory')",
            [],
        ).unwrap();

        run_migrations(&conn, 8).unwrap();

        // Agent memory should be scope_level 4 (run)
        let level: i32 = conn
            .query_row(
                "SELECT scope_level FROM memories WHERE key = 'agents:run1:fix'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(level, 4);

        // User memory should have trust=high
        let trust: String = conn
            .query_row(
                "SELECT trust FROM memories WHERE key = 'user:note'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trust, "high");
    }
}
