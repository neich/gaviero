//! Tier C / C1.4 history compression on `MemoryStore`.
//!
//! Carved out of `store/mod.rs` (Phase 4.4). Owns the four methods
//! that read, compress, enumerate, and measure compressed History
//! bodies. `compress_history_row` is one of the two sanctioned
//! callers of `schema::drop_history_immutable_triggers`; the C2.4
//! grep invariant test (in `tests/c24_trigger_disable_invariant.rs`)
//! enforces that this list stays at exactly two entries inside the
//! `store/` subtree.

use anyhow::{Context, Result, anyhow};

use super::MemoryStore;
use crate::memory::compression;
use crate::memory::schema;

impl MemoryStore {
    /// C1.4: read a History row's transcript, decompressing
    /// transparently when `compressed = 1`. SHA-256 is verified against
    /// the row's `content_hash` on every decompress; mismatch returns
    /// an error (data-integrity alarm) rather than corrupted bytes.
    /// Returns `Ok(None)` when no row exists for `memory_id`.
    pub async fn read_history_content(&self, memory_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let row: Result<(String, Option<Vec<u8>>, i64), rusqlite::Error> = conn.query_row(
            "SELECT content, content_blob, compressed
               FROM memories
              WHERE id = ?1 AND memory_kind = 'history'",
            rusqlite::params![memory_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok((content, blob, compressed)) => {
                if compressed == 0 {
                    Ok(Some(content))
                } else {
                    // Compressed path: the row's own `content` column
                    // carries the placeholder string with the embedded
                    // SHA-256 of the original transcript. We verify
                    // the decompressed bytes against that SHA before
                    // returning — never propagate a mismatch.
                    let blob = blob.ok_or_else(|| {
                        anyhow!(
                            "history row {memory_id} marked compressed=1 but content_blob is NULL"
                        )
                    })?;
                    let expected_sha = compression::parse_compressed_placeholder(&content)
                        .ok_or_else(|| {
                            anyhow!(
                                "history row {memory_id} marked compressed=1 but content \
                                 is not a recognized placeholder: {content:?}"
                            )
                        })?;
                    let decoded = compression::decompress_with_verify(&blob, &expected_sha)?;
                    Ok(Some(decoded))
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("reading history row: {e}")),
        }
    }

    /// C1.4: opportunistic compression of one History row.
    ///
    /// Encodes the row's `content` to zstd, verifies the round-trip
    /// SHA-256 against the row's `content_hash`, then in **one
    /// transaction**: drops the C1.3 immutability triggers, UPDATEs
    /// the row to install the blob (and a sha-prefixed placeholder in
    /// `content`), reinstalls the triggers, and commits.
    ///
    /// **Trigger-disable callsite #1.** This function and the C2.4
    /// `RedactHistory` handler are the only two places authorized to
    /// drop the C1.3 triggers. Both perform a single UPDATE inside one
    /// transaction with no `await` and no userspace call between drop
    /// and reinstall — the privileged window is microseconds long. CI
    /// grep verifies this list of callsites is closed.
    ///
    /// Returns `Ok(false)` when the row is already compressed, missing,
    /// or not a history row. Returns `Ok(true)` after a successful
    /// compression. The original `content_hash` is preserved across
    /// the operation, so the C1 audit invariant ("content cannot
    /// change") still holds: the canonical SHA of the decompressed
    /// blob equals the SHA of the original transcript.
    pub async fn compress_history_row(&self, memory_id: i64) -> Result<bool> {
        // Read first (outside the transaction) so we can do the
        // CPU-bound zstd encode without holding the SQLite mutex.
        // The `content_hash` column is a normalized (trim+lowercase)
        // hash kept for dedup; the integrity SHA we embed in the blob
        // placeholder is the *raw* content's SHA, computed inside
        // `compress_with_verify`.
        let read = {
            let conn = self.conn.lock().await;
            let row: Result<(String, i64, i64), rusqlite::Error> = conn.query_row(
                "SELECT content, compressed, memory_kind = 'history'
                   FROM memories WHERE id = ?1",
                rusqlite::params![memory_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?)),
            );
            match row {
                Ok(t) => Some(t),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(anyhow!("reading row for compression: {e}")),
            }
        };
        let (content, compressed, is_history) = match read {
            Some(t) => t,
            None => return Ok(false),
        };
        if is_history == 0 || compressed != 0 {
            return Ok(false);
        }

        // CPU-bound encode + verify, lock-free.
        let blob = compression::compress_with_verify(&content)?;
        let placeholder = compression::compressed_content_placeholder(&blob.sha_hex);

        // Privileged write window: tight scope so the lock guard and
        // transaction don't bleed past the commit. No `await` inside.
        let n = {
            let mut conn = self.conn.lock().await;
            let tx = conn.transaction().context("compress: begin transaction")?;
            // Drop the C1.3 triggers ONLY for the duration of this
            // write (single UPDATE, no await between drop + reinstall).
            schema::drop_history_immutable_triggers(&tx)
                .context("compress: drop triggers")?;
            let updated = tx.execute(
                "UPDATE memories
                    SET content = ?1,
                        content_blob = ?2,
                        compressed = 1,
                        updated_at = datetime('now')
                  WHERE id = ?3 AND memory_kind = 'history' AND compressed = 0",
                rusqlite::params![placeholder, blob.bytes, memory_id],
            )?;
            // Reinstall before commit — never leave the trigger off
            // across a commit boundary, even on success.
            schema::install_history_immutable_triggers(&tx)
                .context("compress: reinstall triggers")?;
            tx.commit().context("compress: commit transaction")?;
            updated
        };

        if n == 0 {
            // Race: another caller compressed the row between our read
            // and our write. Not an error.
            return Ok(false);
        }

        // Post-commit verification: read the blob back and re-verify.
        // Belt and braces — the in-memory verify in compress_with_verify
        // already ran, but a defensive re-decode catches any
        // hypothetical SQLite-side corruption (e.g. NUL truncation in
        // the BLOB binding) before the user discovers it on read.
        match self.read_history_content(memory_id).await {
            Ok(Some(s)) if s == content => Ok(true),
            Ok(Some(_)) => Err(anyhow!(
                "post-compression verification failed on row {memory_id}: \
                 decoded content does not equal original"
            )),
            Ok(None) => Err(anyhow!(
                "post-compression verification: row {memory_id} disappeared"
            )),
            Err(e) => Err(e),
        }
    }

    /// C1.4: pick history rows older than `older_than_days` that still
    /// hold uncompressed bodies. Returns ids ordered oldest-first,
    /// capped at `limit`. Used by the sleeptime pass to compress in
    /// batches without a giant transaction. Cutoff is evaluated by
    /// SQLite's `datetime('now', '-N days')` so the comparison happens
    /// inside the DB.
    pub async fn list_history_rows_to_compress(
        &self,
        older_than_days: u32,
        limit: usize,
    ) -> Result<Vec<i64>> {
        let conn = self.conn.lock().await;
        let cutoff_expr = format!("datetime('now', '-{} days')", older_than_days);
        let sql = format!(
            "SELECT id FROM memories
              WHERE memory_kind = 'history'
                AND compressed = 0
                AND created_at < {cutoff_expr}
              ORDER BY created_at ASC
              LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// C1.4: byte-length of a history row's `content_blob`. Returns
    /// `Ok(None)` for missing or non-history rows. Used by the
    /// sleeptime telemetry to record compression-ratio per row.
    pub async fn history_compressed_blob_len(&self, memory_id: i64) -> Result<Option<usize>> {
        let conn = self.conn.lock().await;
        let row: Result<Option<i64>, rusqlite::Error> = conn.query_row(
            "SELECT length(content_blob) FROM memories
              WHERE id = ?1 AND memory_kind = 'history'",
            rusqlite::params![memory_id],
            |r| r.get(0),
        );
        match row {
            Ok(Some(n)) => Ok(Some(n as usize)),
            Ok(None) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("reading content_blob length: {e}")),
        }
    }
}
