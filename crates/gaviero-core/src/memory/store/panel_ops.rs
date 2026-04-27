//! Tier A / A4 + C1.5 TUI memory panel surface, plus the small
//! read helpers the panel and the B6 telemetry classifier rely on.
//!
//! Carved out of `store/mod.rs` (Phase 4.6). Owns:
//!
//! - panel **read** queries — `recent_memories_by_kind`,
//!   `recent_memories`, `recent_memories_for_run`, `scope_summary`,
//!   `get_content`, `get_memory_kind`, `find_memory_by_tag`,
//!   `embedding_for`,
//! - panel **edit** writes — `delete_memory_by_id`, `set_trust_score`,
//!   `change_memory_scope`, `update_memory_text`,
//! - two `#[doc(hidden)]` test-only helpers (`force_age_for_test`,
//!   `count_audit_for_test`).
//!
//! Direct hard-deletes (`delete_memory_by_id`) are kept here even
//! though they sound like a deletions concern — they're the panel's
//! `d` action, and unlike the audited C2 path they don't touch the
//! `deletions` table. The audited path lives in
//! `store/deletions_ops.rs`.

use anyhow::{Context, Result, anyhow};

use super::{MemoryStore, blob_to_embedding};
use crate::memory::scope::{MemoryType, StoreResult, Trust, WriteMeta, WriteScope};
use crate::memory::scoring::ScoredMemory;
use crate::memory::trust_defaults::MemorySource;

impl MemoryStore {
    // ── Panel reads (Tier A / A4 + C1.5) ─────────────────────────

    /// C1.5: Memories written within the last `hours` hours of a
    /// specific `kind`, newest first. Drives the per-kind tab content
    /// in the TUI memory panel — Records, History, or Summaries each
    /// get their own filtered list. Composes with [`Self::recent_memories`]
    /// (which is unfiltered).
    pub async fn recent_memories_by_kind(
        &self,
        kind: super::super::kind::MemoryKind,
        hours: u32,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;
        let since = format!("-{hours} hours");
        let kind_str = kind.as_str();
        let mut stmt = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories
                 WHERE created_at >= datetime('now', ?1)
                   AND memory_kind = ?2
                 ORDER BY id DESC
                 LIMIT ?3",
            )
            .context("preparing recent_memories_by_kind")?;
        let rows = stmt
            .query_map(rusqlite::params![since, kind_str, limit as i64], |row| {
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
                    source: MemorySource::parse_str(&source_str),
                    trust_score,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })
            .context("running recent_memories_by_kind")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading recent_memories_by_kind row")?);
        }
        Ok(out)
    }

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
                    source: MemorySource::parse_str(&source_str),
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
                    source: MemorySource::parse_str(&source_str),
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

    /// C1: read the lifecycle class for a row. Returns `None` if the
    /// row does not exist. Used by the writer task's panel-edit guard
    /// to reject mutations on history rows before the SQL trigger
    /// (C1.3) catches them at the DB level.
    pub async fn get_memory_kind(
        &self,
        memory_id: i64,
    ) -> Result<Option<super::super::kind::MemoryKind>> {
        use std::str::FromStr;
        let conn = self.conn.lock().await;
        let row: Result<String, rusqlite::Error> = conn.query_row(
            "SELECT memory_kind FROM memories WHERE id = ?1",
            rusqlite::params![memory_id],
            |r| r.get(0),
        );
        match row {
            Ok(s) => Ok(super::super::kind::MemoryKind::from_str(&s).ok()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("reading memory_kind: {e}")),
        }
    }

    /// C1: locate a single row by its `tag` column and return the
    /// triple `(id, memory_kind, content)`. Used by the C1.5 panel to
    /// find history rows by their `history:<session>:<turn>` tag, and
    /// by the writer-task tests to assert that the C1.2 history-row
    /// write landed correctly. Returns `None` if no row matches.
    pub async fn find_memory_by_tag(
        &self,
        tag: &str,
    ) -> Result<Option<(i64, super::super::kind::MemoryKind, String)>> {
        use std::str::FromStr;
        let conn = self.conn.lock().await;
        let row: Result<(i64, String, String), rusqlite::Error> = conn.query_row(
            "SELECT id, memory_kind, content FROM memories WHERE tag = ?1 LIMIT 1",
            rusqlite::params![tag],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok((id, kind_str, content)) => match super::super::kind::MemoryKind::from_str(&kind_str) {
                Ok(kind) => Ok(Some((id, kind, content))),
                Err(e) => Err(anyhow!("invalid memory_kind '{kind_str}': {e}")),
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("finding memory by tag: {e}")),
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
    pub async fn change_memory_scope(
        &self,
        memory_id: i64,
        new_scope: &WriteScope,
    ) -> Result<i64> {
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
        let source_kind = MemorySource::parse_str(&source_str);
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
        let source_kind = MemorySource::parse_str(&source_str);
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
}
