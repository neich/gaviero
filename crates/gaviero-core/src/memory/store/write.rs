//! Write-side surface on `MemoryStore`.
//!
//! Carved out of `store/mod.rs` (Phase 4.8). Houses every method
//! that mutates the memories table or its derived vec / fts indexes:
//!
//! - **Legacy namespace CRUD**: `store`, `store_with_privacy`,
//!   `store_with_options`, `get`, `list_keys`, `delete`,
//!   `clear_namespace`, `reindex`. These predate the scoped pipeline
//!   and are kept for the swarm context bundle and a handful of
//!   pre-A2 callers.
//! - **B1 re-embed migration**: `reembed_fetch_rows`,
//!   `reembed_apply_batch`.
//! - **`_gaviero_meta` accessors**: `set_meta_value`,
//!   `get_meta_value` (used by the embedder-stamp path, the C1
//!   backup path, and the Phase 2 sleeptime scheduler).
//! - **Source-staleness helpers**: `check_staleness`, `mark_stale`.
//! - **Scoped write**: `has_content_hash`, `store_scoped` — the
//!   modern hot path with SHA-256 + semantic dedup.
//!
//! Private helpers (`reinforce_memory`, `broader_scope_paths`,
//!   `find_semantic_duplicate`, `lexical_overlap`) live here too;
//!   `store_scoped` is their only caller.
//!
//! Unrelated reads (`recent_memories*`, `get_memory_kind`,
//! `get_content`, `embedding_for`, `find_memory_by_tag`) live in
//! `store/panel_ops.rs`. Search reads live in `store/search.rs`
//! and `store/search_legacy.rs`.

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{
    MemoryEntry, MemoryStore, SEMANTIC_DEDUP_THRESHOLD, StoreOptions, blob_to_embedding,
    cosine_similarity, embedding_to_blob, file_hash,
};
use crate::memory::scope::{StoreResult, WriteMeta, WriteScope};
use crate::memory::schema;

impl MemoryStore {
    // ── Legacy namespace writes ────────────────────────────────────

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
        let model_id = self.embedder.name().to_string();

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
        let model_id = self.embedder.name().to_string();
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
    /// embeds outside the lock and writes via [`Self::reembed_apply_batch`].
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

    // ── _gaviero_meta accessors ────────────────────────────────────

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

    // ── Source-staleness ──────────────────────────────────────────

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

    /// Probe whether a content-hash row already exists at the given
    /// `(scope_level, scope_path)` in this store. Used by the
    /// [`super::super::stores::MemoryStores`] registry to detect
    /// cross-DB coverage by a broader scope before delegating the
    /// actual write to the target store.
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
        let kind_str = meta.kind.as_str();
        // C1: History rows are append-only with no dedup. Each turn
        // writes a fresh transcript row even if its content somehow
        // hashes equal to an earlier one. Records and Summaries dedup
        // only against same-kind rows so a record's hash does not
        // collide with a history row carrying the same body.
        let skip_dedup = meta.kind == crate::memory::kind::MemoryKind::History;

        // Compute embedding outside the lock
        let embedding = self
            .embedder
            .embed_document(content)
            .await
            .context("computing embedding for scoped store")?;
        let embedding_blob = embedding_to_blob(&embedding);
        let model_id = self.embedder.name().to_string();

        let conn = self.conn.lock().await;

        let ancestor_paths = broader_scope_paths(scope);

        if !skip_dedup {
            // Check for exact duplicate at same scope (and same kind).
            let existing: Option<i64> = conn
                .prepare(
                    "SELECT id FROM memories
                     WHERE content_hash = ?1 AND scope_path = ?2 AND memory_kind = ?3",
                )?
                .query_row(rusqlite::params![hash, scope_path, kind_str], |row| {
                    row.get(0)
                })
                .ok();

            if let Some(id) = existing {
                // Reinforce: bump access count and timestamp
                reinforce_memory(&conn, id)?;
                tracing::debug!(id, "scoped store: deduplicated at same scope");
                return Ok(StoreResult::Deduplicated(id));
            }

            for (level, ancestor_path) in &ancestor_paths {
                let covered: bool = conn
                    .prepare(
                        "SELECT 1 FROM memories
                         WHERE content_hash = ?1 AND scope_level = ?2 AND scope_path = ?3
                           AND memory_kind = ?4
                         LIMIT 1",
                    )?
                    .query_row(
                        rusqlite::params![hash, level, ancestor_path, kind_str],
                        |_| Ok(true),
                    )
                    .unwrap_or(false);
                if covered {
                    tracing::debug!(level, "scoped store: already covered at broader scope");
                    return Ok(StoreResult::AlreadyCovered);
                }
            }
        }

        if !skip_dedup
            && scope_level != crate::memory::scope::SCOPE_RUN
            && let Some(id) = find_semantic_duplicate(
                &conn,
                content,
                &embedding,
                scope_level,
                &scope_path,
                meta.memory_type.as_str(),
                SEMANTIC_DEDUP_THRESHOLD,
            )?
        {
            reinforce_memory(&conn, id)?;
            tracing::debug!(
                id,
                scope_path,
                threshold = SEMANTIC_DEDUP_THRESHOLD,
                "scoped store: semantic duplicate at same scope"
            );
            return Ok(StoreResult::Deduplicated(id));
        }

        // Check for semantic coverage at any broader ancestor scope.
        if !skip_dedup {
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
        }

        // Insert new memory
        let namespace = meta.tag.as_deref().unwrap_or("default");
        let key = format!("scoped:{}:{}", scope_path, &hash[..12]);

        conn.execute(
            "INSERT INTO memories (
                namespace, key, content, embedding, model_id,
                scope_level, scope_path, repo_id, module_path, run_id,
                content_hash, memory_type, trust, tag,
                importance, privacy, source, trust_score, memory_kind
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'public', ?16, ?17, ?18)",
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
                meta.kind.as_str(),
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
}

// ── Private helpers used only by `store_scoped` ───────────────────

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
    use crate::memory::scope::{SCOPE_GLOBAL, SCOPE_REPO, SCOPE_WORKSPACE};

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
    // C1: dedup never crosses kinds. Records dedup against records,
    // Summaries against Summaries, History never enters this path.
    // (Caller is responsible for routing — this filter is defense.)
    let mut stmt = conn.prepare(
        "SELECT id, content, embedding FROM memories
         WHERE scope_level = ?1
           AND scope_path = ?2
           AND memory_type = ?3
           AND memory_kind != 'history'
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
