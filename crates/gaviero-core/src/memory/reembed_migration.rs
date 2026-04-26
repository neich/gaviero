//! Tier B / B1d: One-shot re-embed migration for embedder upgrades.
//!
//! When the user flips `memory.embedder.model` (e.g. nomic →
//! gte-modernbert) every existing row's embedding becomes meaningless —
//! cosine similarity across embedder versions is undefined. This module
//! provides the migration path:
//!
//! 1. Take a `.bak-<timestamp>` copy of the workspace `memory.db`.
//! 2. Stream rows in batches; embed each row's `content` with the new
//!    embedder OUTSIDE the SQLite mutex.
//! 3. Update `memories.embedding`, `memories.model_id`, and the two
//!    vec virtual tables (`vec_memories_scoped` and the legacy
//!    `vec_memories`) inside one short critical section per batch.
//! 4. Stamp `_gaviero_meta.embedder_model` only after the final batch
//!    so an interrupted run leaves the meta row pointing at the old
//!    model and is naturally idempotent on resume.
//!
//! Failures during the run are non-fatal: each row is independent.
//! Rollback: stop Gaviero, restore the `.bak` file, flip the setting
//! back.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use super::embedder::Embedder;
use super::store::MemoryStore;

/// Per-batch progress callback signature: `(rows_done, total_rows)`.
pub type ProgressFn = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// Outcome of a re-embed run.
#[derive(Debug, Clone)]
pub struct ReembedReport {
    pub total: usize,
    pub re_embedded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub backup_path: Option<PathBuf>,
}

/// Sidecar copy of the workspace memory.db before the run starts. The
/// path is `<db>.bak-<unix_ts>`. Returns `None` when the source file
/// does not exist (in-memory store).
pub fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = db_path.with_extension(format!("db.bak-{ts}"));
    std::fs::copy(db_path, &backup)
        .with_context(|| format!("backing up {} → {}", db_path.display(), backup.display()))?;
    Ok(Some(backup))
}

/// Re-embed every row in `memories` with `new_embedder`, replacing the
/// blob in `memories.embedding` and the matching rows in
/// `vec_memories_scoped` / `vec_memories`. The embedder must produce
/// vectors of the same dimensionality as the current schema; mismatched
/// dimensions abort with an error before any write.
///
/// Embedding runs outside the SQLite mutex (golden rule). Each batch
/// holds the lock briefly to commit `batch_size` rows; the rest of the
/// process can read in the gaps.
pub async fn reembed_all(
    store: &Arc<MemoryStore>,
    new_embedder: Arc<dyn Embedder>,
    batch_size: usize,
    progress: Option<ProgressFn>,
) -> Result<ReembedReport> {
    if new_embedder.dimensions() != store.embedder().dimensions() {
        anyhow::bail!(
            "re-embed aborted: new embedder ({}, dim={}) does not match schema dim={} — \
             schema migration is out of scope for B1",
            new_embedder.model_id(),
            new_embedder.dimensions(),
            store.embedder().dimensions()
        );
    }

    // B1 acceptance: take a `.bak` *before* any write. For on-disk
    // stores the rollback path is "stop Gaviero, restore the backup".
    // In-memory stores skip silently — they vanish on process exit
    // anyway, so a backup would be meaningless.
    let backup_path = match store.db_path() {
        Some(p) => backup_db(p).context("taking memory.db backup before re-embed")?,
        None => None,
    };

    let rows = store.reembed_fetch_rows().await?;
    let total = rows.len();
    let mut report = ReembedReport {
        total,
        re_embedded: 0,
        skipped: 0,
        failed: 0,
        backup_path,
    };

    let new_model_id = new_embedder.model_id().to_string();
    let batch_size = batch_size.max(1);
    let mut done = 0usize;

    for chunk in rows.chunks(batch_size) {
        // Embed outside the lock.
        let mut updates: Vec<(i64, Vec<u8>)> = Vec::with_capacity(chunk.len());
        for (id, content, current_model) in chunk {
            if current_model.as_deref() == Some(new_model_id.as_str()) {
                report.skipped += 1;
                continue;
            }
            match new_embedder.embed_document(content).await {
                Ok(embedding) => {
                    updates.push((*id, super::store::embedding_to_blob_pub(&embedding)));
                }
                Err(e) => {
                    tracing::warn!(
                        target: "memory_reembed",
                        memory_id = id,
                        error = %e,
                        "re-embed failed for row"
                    );
                    report.failed += 1;
                }
            }
        }

        if !updates.is_empty() {
            store
                .reembed_apply_batch(&updates, &new_model_id)
                .await
                .context("applying re-embed batch")?;
            report.re_embedded += updates.len();
        }

        done += chunk.len();
        if let Some(cb) = &progress {
            cb(done, total);
        }
    }

    // Final stamp.
    store
        .set_meta_value("embedder_model", &new_model_id)
        .await
        .context("stamping _gaviero_meta.embedder_model")?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_path_is_dot_bak_with_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("memory.db");
        std::fs::write(&db, b"hello").unwrap();
        let backup = backup_db(&db).unwrap().unwrap();
        let name = backup.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("memory.db.bak-"), "got {name}");
        assert!(backup.exists());
        assert_eq!(std::fs::read(&backup).unwrap(), b"hello");
    }

    #[test]
    fn backup_returns_none_when_db_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("absent.db");
        let backup = backup_db(&db).unwrap();
        assert!(backup.is_none());
    }
}
