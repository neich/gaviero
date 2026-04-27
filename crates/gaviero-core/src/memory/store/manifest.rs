//! Tier S / S4: injection manifest persistence.
//!
//! Carved out of `store/mod.rs` (Phase 4.1) so the manifest API has
//! a single home. The writer task's `InjectionManifest` handler is
//! the only producer; the TUI memory panel and the
//! `gaviero-cli memory manifest` CLI are the only consumers.
//!
//! All methods attach to [`MemoryStore`] via an `impl` block; the
//! struct definition and its `conn` field stay in `store/mod.rs`.

use anyhow::{Context, Result};
use serde_json::Value as JsonValue;

use super::MemoryStore;

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
    pub(crate) fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
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

impl MemoryStore {
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
        payload: &JsonValue,
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
}
