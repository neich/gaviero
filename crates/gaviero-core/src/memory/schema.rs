/// SQL schema and migrations for the memory store.
///
/// Uses `PRAGMA user_version` to track the current schema version.
/// Each migration runs exactly once, in order, on database open.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version. Increment when adding a new migration.
const CURRENT_VERSION: u32 = 3;

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
        .and_then(|mut stmt| {
            stmt.query_row([], |_| Ok(true))
        })
        .unwrap_or(false);

    if !has_privacy {
        conn.execute_batch(
            "ALTER TABLE memories ADD COLUMN privacy TEXT NOT NULL DEFAULT 'public';",
        )
        .context("adding privacy column")?;
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_privacy ON memories(privacy);",
    )
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
            conn.execute_batch(&format!(
                "ALTER TABLE memories ADD COLUMN {col} {typedef};"
            ))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: register sqlite-vec extension and open an in-memory connection.
    fn setup_conn() -> Connection {
        // Register BEFORE opening the connection (auto_extension applies to new opens)
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(
                std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()),
            ));
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
        for col in &["importance", "access_count", "last_accessed_at", "source_file", "source_hash"] {
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
}
