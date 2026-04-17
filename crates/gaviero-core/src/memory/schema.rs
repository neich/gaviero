/// SQL schema and migrations for the memory store.
///
/// Uses `PRAGMA user_version` to track the current schema version.
/// Each migration runs exactly once, in order, on database open.
use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version. Increment when adding a new migration.
const CURRENT_VERSION: u32 = 4;

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

    // Stamp the current version
    conn.pragma_update(None, "user_version", CURRENT_VERSION)
        .context("updating user_version")?;

    Ok(())
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
