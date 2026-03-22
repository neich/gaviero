/// SQL schema and migrations for the memory store.
///
/// Uses `PRAGMA user_version` to track the current schema version.
/// Each migration runs exactly once, in order, on database open.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version. Increment when adding a new migration.
const CURRENT_VERSION: u32 = 2;

/// Run all pending migrations on the given connection.
///
/// Checks `PRAGMA user_version` to determine the current schema version,
/// then applies each migration in sequence. Idempotent — safe to call
/// on every database open.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading user_version")?;

    if version < 1 {
        migrate_v1(conn).context("migration v1")?;
    }
    if version < 2 {
        migrate_v2(conn).context("migration v2")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_database() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

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
    }

    #[test]
    fn test_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // should not fail
    }

    #[test]
    fn test_upgrade_from_v1() {
        let conn = Connection::open_in_memory().unwrap();

        // Simulate a v1 database (created before migration system)
        migrate_v1(&conn).unwrap();
        conn.pragma_update(None, "user_version", 1u32).unwrap();

        // Now run full migrations — should apply v2 only
        run_migrations(&conn).unwrap();

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
        let conn = Connection::open_in_memory().unwrap();

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
        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }
}
