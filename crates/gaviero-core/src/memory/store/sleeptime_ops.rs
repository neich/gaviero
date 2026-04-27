//! Tier B / B5 sleeptime operations on `MemoryStore`.
//!
//! Carved out of `store/mod.rs` (Phase 4.3). Houses the four
//! sleeptime steps (decay sweep, near-dup merge, promote, trust
//! re-score), plus the C2.5 summary-prune and the `supersede_memory`
//! soft-delete used by both sleeptime merge logic and the session
//! consolidator. The audit-row helper `log_sleeptime_audit` lives
//! here too — it's the persistence half of every sleeptime op.

use anyhow::{Context, Result};

use super::{MemoryStore, blob_to_embedding, cosine_similarity, days_since_iso};
use crate::memory::scope::MemoryType;
use crate::memory::sleeptime::{SleeptimeConfig, SleeptimeOperation, pick_merge_winner};
use crate::memory::trust_defaults::MemorySource;

impl MemoryStore {
    /// Insert one row into `sleeptime_audit`. Idempotent at the row
    /// level — repeated runs append; the run_id ties them back.
    pub async fn log_sleeptime_audit(
        &self,
        run_id: &str,
        kind: &str,
        memory_id: Option<i64>,
        related_id: Option<i64>,
        payload: &str,
        dry_run: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sleeptime_audit
                 (run_id, kind, memory_id, related_id, payload, dry_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![run_id, kind, memory_id, related_id, payload, dry_run as i64],
        )
        .context("inserting sleeptime_audit row")?;
        Ok(())
    }

    /// C2.5: soft-delete every Summary row older than `older_than_days`.
    /// Each match is routed through [`Self::soft_delete_memory`] tagged
    /// `DeletedBy::SleeptimePrune` so it inherits the shorter 14-day
    /// audit retention (per plan §C2). Returns the number of rows
    /// soft-deleted. `dry_run = true` returns the would-be count
    /// without touching anything. Records and History are untouched.
    pub async fn sleeptime_prune_old_summaries(
        &self,
        older_than_days: u32,
        dry_run: bool,
    ) -> Result<usize> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare(
                "SELECT id FROM memories
                  WHERE memory_kind = 'summary'
                    AND created_at < datetime('now', ?1)
                  ORDER BY id ASC",
            )?;
            stmt.query_map(
                rusqlite::params![format!("-{} days", older_than_days)],
                |r| r.get::<_, i64>(0),
            )?
            .filter_map(|r| r.ok())
            .collect()
        };
        if dry_run {
            return Ok(ids.len());
        }
        let mut n = 0;
        for id in ids {
            let reason = format!("sleeptime summary prune (>{older_than_days}d)");
            match self
                .soft_delete_memory(
                    id,
                    super::super::deletions::DeletedBy::SleeptimePrune,
                    Some(&reason),
                    None,
                )
                .await
            {
                Ok(_) => n += 1,
                Err(e) => {
                    tracing::warn!(
                        target: "memory_sleeptime",
                        memory_id = id,
                        error = %e,
                        "summary prune: soft-delete failed"
                    );
                }
            }
        }
        Ok(n)
    }

    /// B5 step 1: decay sweep. Walks every memory, computes recency
    /// under the current B4 floor, and returns
    /// [`SleeptimeOperation::DecayFlagged`] for rows at the floor.
    /// **Never deletes.** `dry_run` only affects the audit-log marker
    /// — flagging is itself non-destructive.
    pub async fn sleeptime_decay_sweep(
        &self,
        _dry_run: bool,
    ) -> Result<Vec<SleeptimeOperation>> {
        use crate::memory::scoring::recency_factor;
        let cfg = self.scoring_cfg();
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, COALESCE(last_accessed_at, updated_at)
                 FROM memories
                 WHERE superseded_by IS NULL
                 ORDER BY id",
            )
            .context("preparing decay sweep")?;
        let mut ops = Vec::new();
        let mut rows = stmt.query([]).context("running decay sweep")?;
        while let Some(row) = rows.next().context("decay sweep row")? {
            let id: i64 = row.get(0)?;
            let mt: String = row.get(1)?;
            let stamp: String = row.get(2)?;
            let memory_type = MemoryType::parse_str(&mt);
            let days = days_since_iso(&stamp);
            let r = recency_factor(
                days,
                memory_type,
                cfg.recency_floor,
                &cfg.decay_exempt_types,
            );
            if r <= cfg.recency_floor + 1e-6 {
                ops.push(SleeptimeOperation::DecayFlagged {
                    memory_id: id,
                    recency: r,
                });
            }
        }
        Ok(ops)
    }

    /// B5 step 2: near-duplicate merge within a scope level. Pairs
    /// with cosine ≥ `threshold` and matching memory_type are merged
    /// — the source-aware winner is retained (`user_remember` always
    /// wins). Vec rows for the loser are dropped. Returns one
    /// [`SleeptimeOperation::NearDupMerged`] per pair processed.
    pub async fn sleeptime_near_dup_merge(
        &self,
        threshold: f32,
        dry_run: bool,
    ) -> Result<Vec<SleeptimeOperation>> {
        // Pull all rows with their embeddings + source/trust. We hold
        // the lock for the read pass, then drop it before each merge so
        // `delete_memory_by_id` can re-acquire.
        struct Row {
            id: i64,
            scope_level: i32,
            // B5 safety: full scope identity (scope_path encodes
            // repo:<id>[:module:<path>] or run:<id>) is required to
            // ensure near-dup merges never cross repo / module / run
            // boundaries. Two memories at scope_level=Repo in different
            // repos must NOT be considered near-duplicates.
            scope_path: String,
            memory_type: MemoryType,
            content_hash: String,
            source: MemorySource,
            trust_score: f32,
            embedding: Vec<f32>,
        }

        let rows: Vec<Row> = {
            let conn = self.conn.lock().await;
            let mut stmt = conn
                .prepare(
                    "SELECT id, scope_level, scope_path, memory_type, content_hash,
                            source, trust_score, embedding
                     FROM memories
                     WHERE superseded_by IS NULL AND embedding IS NOT NULL",
                )
                .context("preparing near_dup_merge")?;
            let mut out = Vec::new();
            let mut q = stmt.query([]).context("running near_dup_merge")?;
            while let Some(r) = q.next().context("near_dup row")? {
                let id: i64 = r.get(0)?;
                let scope_level: i32 = r.get(1)?;
                let scope_path: Option<String> = r.get(2)?;
                let mt: String = r.get(3)?;
                let ch: Option<String> = r.get(4)?;
                let src: String = r.get(5)?;
                let trust: f32 = r.get(6)?;
                let emb_blob: Vec<u8> = r.get(7)?;
                let embedding = blob_to_embedding(&emb_blob).unwrap_or_default();
                if embedding.is_empty() {
                    continue;
                }
                out.push(Row {
                    id,
                    scope_level,
                    scope_path: scope_path.unwrap_or_default(),
                    memory_type: MemoryType::parse_str(&mt),
                    content_hash: ch.unwrap_or_default(),
                    source: MemorySource::parse_str(&src),
                    trust_score: trust,
                    embedding,
                });
            }
            out
        };

        let mut ops: Vec<SleeptimeOperation> = Vec::new();
        let mut dropped: std::collections::HashSet<i64> = std::collections::HashSet::new();

        for i in 0..rows.len() {
            if dropped.contains(&rows[i].id) {
                continue;
            }
            for j in (i + 1)..rows.len() {
                if dropped.contains(&rows[j].id) {
                    continue;
                }
                // Group strictly by scope identity: same scope_level is
                // necessary but not sufficient (Repo-level rows in two
                // different repos share scope_level but not scope_path).
                // Empty scope_path falls through this check — pre-A4
                // legacy rows lacking scope metadata never merge across
                // each other, which is conservative on purpose.
                if rows[i].scope_level != rows[j].scope_level
                    || rows[i].scope_path != rows[j].scope_path
                    || rows[i].scope_path.is_empty()
                    || rows[i].memory_type != rows[j].memory_type
                {
                    continue;
                }
                if !rows[i].content_hash.is_empty() && rows[i].content_hash == rows[j].content_hash
                {
                    // Exact-content dups already handled by write-time
                    // dedup; skip here so the merge counter stays
                    // honest about *near*-dups.
                    continue;
                }
                let cos = cosine_similarity(&rows[i].embedding, &rows[j].embedding);
                if cos < threshold {
                    continue;
                }
                let (keep_id, drop_id) = pick_merge_winner(
                    rows[i].id,
                    rows[i].source,
                    rows[i].trust_score,
                    rows[j].id,
                    rows[j].source,
                    rows[j].trust_score,
                );
                let (keep_source, drop_source) = if keep_id == rows[i].id {
                    (rows[i].source, rows[j].source)
                } else {
                    (rows[j].source, rows[i].source)
                };
                ops.push(SleeptimeOperation::NearDupMerged {
                    keep_id,
                    drop_id,
                    cosine: cos,
                    keep_source,
                    drop_source,
                });
                dropped.insert(drop_id);
                if !dry_run {
                    // C2.5: route the loser through the audit table so
                    // the operation is restorable within the retention
                    // window. `merged_into = keep_id` rides inside
                    // `original_row_json` so a future restore can carry
                    // the merge edge through dedup. Best-effort —
                    // failures are logged but don't abort the sweep.
                    let reason =
                        format!("sleeptime near-dup merge into keep_id={keep_id} cosine={cos:.3}");
                    if let Err(e) = self
                        .soft_delete_memory(
                            drop_id,
                            super::super::deletions::DeletedBy::SleeptimeMerge,
                            Some(&reason),
                            Some(keep_id),
                        )
                        .await
                    {
                        tracing::warn!(
                            target: "memory_sleeptime",
                            drop_id = drop_id,
                            error = %e,
                            "near_dup merge: soft-delete loser failed"
                        );
                    }
                }
            }
        }
        Ok(ops)
    }

    /// B5 step 3: cross-scope promotion. The pre-existing consolidator
    /// already handles "3+ run-scope hits → repo"; this stub promotes
    /// `decision|convention|invariant` types after a single hit so
    /// high-value reference rows widen sooner. Returns the operations
    /// applied. Currently a no-op skeleton — wiring to the existing
    /// promotion path is a follow-up; the sleeptime caller still gets
    /// a structured empty result.
    pub async fn sleeptime_promote(
        &self,
        _dry_run: bool,
    ) -> Result<Vec<SleeptimeOperation>> {
        Ok(Vec::new())
    }

    /// B5 step 4: trust re-scoring driven by retrieval-use telemetry
    /// (B6) and the manifest hit count fallback. Returns one
    /// [`SleeptimeOperation::TrustAdjusted`] per adjusted row.
    pub async fn sleeptime_trust_rescore(
        &self,
        cfg: &SleeptimeConfig,
    ) -> Result<Vec<SleeptimeOperation>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.trust_score, m.source,
                        COUNT(ru.id)                                       AS injections,
                        SUM(CASE WHEN ru.classification = 'used'    THEN 1 ELSE 0 END) AS used,
                        SUM(CASE WHEN ru.classification = 'unused'  THEN 1 ELSE 0 END) AS unused
                 FROM memories m
                 LEFT JOIN retrieval_use ru ON ru.memory_id = m.id
                 WHERE m.superseded_by IS NULL
                 GROUP BY m.id",
            )
            .context("preparing trust_rescore")?;
        let mut q = stmt.query([]).context("running trust_rescore")?;
        let mut updates: Vec<(i64, f32, f32, Option<f32>, u32)> = Vec::new();
        while let Some(r) = q.next().context("trust_rescore row")? {
            let id: i64 = r.get(0)?;
            let trust: f32 = r.get(1)?;
            let source_str: String = r.get(2)?;
            let injections: u32 = r.get::<_, i64>(3)? as u32;
            let used: u32 = r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u32;
            let _unused: u32 = r.get::<_, Option<i64>>(5)?.unwrap_or(0) as u32;
            if injections < cfg.trust_min_injections {
                continue;
            }
            let source = MemorySource::parse_str(&source_str);
            let user_authored =
                matches!(source, MemorySource::UserRemember | MemorySource::UserPanel);
            if user_authored {
                continue;
            }
            let rate = used as f32 / injections as f32;
            let delta = if rate > cfg.utilization_used_threshold {
                cfg.trust_adjust_delta
            } else if rate < cfg.utilization_unused_threshold {
                -cfg.trust_adjust_delta
            } else {
                continue;
            };
            let new_trust = (trust + delta).clamp(cfg.trust_floor, cfg.trust_ceiling_llm);
            if (new_trust - trust).abs() < 1e-6 {
                continue;
            }
            updates.push((id, trust, new_trust, Some(rate), injections));
        }
        drop(q);
        drop(stmt);
        // Apply updates in a fresh write-pass.
        let mut ops = Vec::with_capacity(updates.len());
        for (id, old_trust, new_trust, rate, injections) in updates {
            if !cfg.dry_run {
                conn.execute(
                    "UPDATE memories SET trust_score = ?1, updated_at = datetime('now')
                     WHERE id = ?2",
                    rusqlite::params![new_trust, id],
                )
                .context("applying trust_rescore update")?;
            }
            ops.push(SleeptimeOperation::TrustAdjusted {
                memory_id: id,
                old_trust,
                new_trust,
                utilization_rate: rate,
                injections,
            });
        }
        Ok(ops)
    }

    /// B5 SUPERSEDE: mark `old_id` as superseded by `new_id`. Soft
    /// delete — the row stays for audit / retrieval-by-id but is
    /// excluded from search via the `superseded_by IS NULL` predicate
    /// the sleeptime loops use.
    pub async fn supersede_memory(&self, old_id: i64, new_id: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let n = conn
            .execute(
                "UPDATE memories SET superseded_by = ?1, updated_at = datetime('now')
                 WHERE id = ?2",
                rusqlite::params![new_id, old_id],
            )
            .context("marking superseded")?;
        Ok(n)
    }
}
