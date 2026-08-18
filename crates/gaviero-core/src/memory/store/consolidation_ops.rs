//! Audit I/O for the session consolidator.
//!
//! The consolidator applies operations an LLM proposed. Before Tier H it
//! did so silently: every apply was `let _ = …`, so a store error, an
//! out-of-range candidate, or an invented memory id all looked exactly
//! like success. Nothing recorded what had been done, which made the
//! behaviour both undiagnosable and irreversible.
//!
//! Every attempted operation now writes one row to `sleeptime_audit`
//! with `origin = 'consolidation'` — applied or not, and why not. That
//! table is the single audit surface for automatic memory mutation
//! (schema v14); `origin` discriminates the producer.

use std::collections::HashSet;

use anyhow::{Context, Result};

use super::MemoryStore;

/// One attempted consolidator operation, as recorded.
///
/// Borrowed rather than owned because every field is already in hand at
/// the call site and the row is written immediately.
pub struct ConsolidationAuditRow<'a> {
    /// The session the operation came from. Attribution only — several
    /// consolidations of one conversation share it.
    pub run_id: &'a str,
    /// The single consolidation invocation this operation belonged to.
    ///
    /// This, not `run_id`, is what `/consolidate history` groups by and
    /// `/consolidate rollback` targets. See `migrate_v16` for what
    /// conflating the two cost.
    pub batch_id: &'a str,
    /// Operation kind: `add`, `merge`, `supersede`, `drop`, or
    /// `session_summary`.
    pub kind: &'a str,
    /// The row the operation produced or targeted, when there is one.
    pub memory_id: Option<i64>,
    /// The other row involved — the merge target, or the superseded id.
    pub related_id: Option<i64>,
    /// The operation itself, as JSON. Stored in the existing `payload`
    /// column, whose contract is "the opaque JSON for this row".
    pub op_json: &'a str,
    /// State before the change, for rollback to restore.
    pub before_json: Option<&'a str>,
    /// Snapshot of post-op state for inspection. Rollback does not
    /// consult this column (MERGE trust is restored from `before_json`).
    pub after_json: Option<&'a str>,
    /// Whether the operation actually landed.
    pub applied: bool,
    /// Why it did not, when `applied` is false.
    pub error: Option<&'a str>,
    /// The consolidator prompt version that produced the operation.
    pub prompt_version: &'a str,
    /// Scope the write landed in, e.g. `repo` or `module:crates/foo`.
    pub scope: &'a str,
    /// What the consolidator said this operation would achieve.
    pub expected_outcome: Option<&'a str>,
}

impl MemoryStore {
    /// Record one attempted consolidator operation.
    ///
    /// Append-only; a failed operation is as much a result as a
    /// successful one and is written with `applied = 0` plus its cause.
    pub async fn log_consolidation_audit(&self, row: ConsolidationAuditRow<'_>) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sleeptime_audit
                 (run_id, batch_id, kind, memory_id, related_id, payload, dry_run,
                  origin, before_json, after_json, applied, error,
                  prompt_version, scope, expected_outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0,
                     'consolidation', ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                row.run_id,
                row.batch_id,
                row.kind,
                row.memory_id,
                row.related_id,
                row.op_json,
                row.before_json,
                row.after_json,
                row.applied as i64,
                row.error,
                row.prompt_version,
                row.scope,
                row.expected_outcome,
            ],
        )
        .context("inserting consolidation audit row")?;
        Ok(())
    }

    /// Every consolidation audit row for one *invocation*, oldest first.
    ///
    /// Ordered by insertion so a rollback can walk them in reverse and
    /// undo the batch in the opposite order to how it was applied.
    ///
    /// `batch_key` matches `COALESCE(batch_id, run_id)`, so a pre-v16
    /// row — which has no `batch_id` — is still reachable under the
    /// `run_id` it was written with.
    pub async fn consolidation_audit_for_batch(
        &self,
        batch_key: &str,
    ) -> Result<Vec<ConsolidationAuditRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, run_id, kind, memory_id, related_id, payload, before_json,
                        after_json, applied, error, prompt_version, scope,
                        expected_outcome, created_at
                   FROM sleeptime_audit
                  WHERE origin = 'consolidation'
                    AND COALESCE(batch_id, run_id) = ?1
                  ORDER BY id ASC",
            )
            .context("preparing consolidation audit query")?;
        let rows = stmt
            .query_map(rusqlite::params![batch_key], |r| {
                Ok(ConsolidationAuditRecord {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    kind: r.get(2)?,
                    memory_id: r.get(3)?,
                    related_id: r.get(4)?,
                    op_json: r.get(5)?,
                    before_json: r.get(6)?,
                    after_json: r.get(7)?,
                    applied: r.get::<_, i64>(8)? != 0,
                    error: r.get(9)?,
                    prompt_version: r.get(10)?,
                    scope: r.get(11)?,
                    expected_outcome: r.get(12)?,
                    created_at: r.get(13)?,
                })
            })
            .context("querying consolidation audit rows")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading consolidation audit rows")?;
        Ok(rows)
    }

    /// Every consolidation audit row for a *session*, across all of its
    /// invocations, oldest first.
    ///
    /// Distinct from [`Self::consolidation_audit_for_batch`]: this is
    /// "everything this conversation's consolidations ever did", which
    /// is the right question for inspection but the wrong one for
    /// rollback.
    pub async fn consolidation_audit_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<ConsolidationAuditRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, run_id, kind, memory_id, related_id, payload, before_json,
                        after_json, applied, error, prompt_version, scope,
                        expected_outcome, created_at
                   FROM sleeptime_audit
                  WHERE origin = 'consolidation' AND run_id = ?1
                  ORDER BY id ASC",
            )
            .context("preparing consolidation audit query")?;
        let rows = stmt
            .query_map(rusqlite::params![run_id], |r| {
                Ok(ConsolidationAuditRecord {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    kind: r.get(2)?,
                    memory_id: r.get(3)?,
                    related_id: r.get(4)?,
                    op_json: r.get(5)?,
                    before_json: r.get(6)?,
                    after_json: r.get(7)?,
                    applied: r.get::<_, i64>(8)? != 0,
                    error: r.get(9)?,
                    prompt_version: r.get(10)?,
                    scope: r.get(11)?,
                    expected_outcome: r.get(12)?,
                    created_at: r.get(13)?,
                })
            })
            .context("querying consolidation audit rows")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading consolidation audit rows")?;
        Ok(rows)
    }
}

/// Origin tag for rows produced by `/consolidate rollback`.
pub const ROLLBACK_ORIGIN: &str = "consolidation_rollback";

/// One inverse operation performed by a rollback, as recorded.
pub struct RollbackAuditRow<'a> {
    /// The session the undone run belonged to. Attribution only.
    pub run_id: &'a str,
    /// The invocation being undone. `(origin, batch_id)` is what makes
    /// "has *this* run been rolled back" answerable — reusing `run_id`
    /// alone made the answer "has any run of this conversation been
    /// rolled back", which then leaked into the consolidator prompt.
    pub batch_id: &'a str,
    /// The kind of operation being reversed, e.g. `add`.
    pub kind: &'a str,
    pub memory_id: Option<i64>,
    pub related_id: Option<i64>,
    pub op_json: &'a str,
    /// Whether the *inverse* landed.
    pub applied: bool,
    pub error: Option<&'a str>,
}

impl MemoryStore {
    /// Record one inverse operation performed by a rollback.
    ///
    /// Rollback rows carry the undone run's `batch_id` and are
    /// distinguished by `origin`, so "has this run been rolled back"
    /// answers itself with one indexed lookup.
    pub async fn log_consolidation_rollback(&self, row: RollbackAuditRow<'_>) -> Result<()> {
        let RollbackAuditRow {
            run_id,
            batch_id,
            kind,
            memory_id,
            related_id,
            op_json,
            applied,
            error,
        } = row;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sleeptime_audit
                 (run_id, batch_id, kind, memory_id, related_id, payload, dry_run,
                  origin, applied, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9)",
            rusqlite::params![
                run_id,
                batch_id,
                kind,
                memory_id,
                related_id,
                op_json,
                ROLLBACK_ORIGIN,
                applied as i64,
                error,
            ],
        )
        .context("inserting consolidation rollback audit row")?;
        Ok(())
    }

    /// Audit ids of originally-applied ops that already have a
    /// successful inverse (`applied = 1`) whose payload names them as
    /// `reverses_audit_id`.
    ///
    /// The rollback latch is "every originally-applied op is in this
    /// set", not "any rollback row exists" — a partial undo must be
    /// allowed to finish the rest. History still uses an EXISTS check
    /// so a partial undo is visible there.
    pub async fn successful_rollback_targets(&self, batch_key: &str) -> Result<HashSet<i64>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM sleeptime_audit
                  WHERE origin = ?1 AND COALESCE(batch_id, run_id) = ?2 AND applied = 1",
            )
            .context("preparing successful-rollback-target query")?;
        let payloads = stmt
            .query_map(rusqlite::params![ROLLBACK_ORIGIN, batch_key], |r| {
                r.get::<_, String>(0)
            })
            .context("querying successful rollback payloads")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading successful rollback payloads")?;
        drop(stmt);
        drop(conn);

        let mut out = HashSet::new();
        for payload in payloads {
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&payload) else {
                continue;
            };
            if let Some(id) = parsed.get("reverses_audit_id").and_then(|v| v.as_i64()) {
                out.insert(id);
            }
        }
        Ok(out)
    }

    /// Has this consolidation *invocation* produced any rollback row?
    ///
    /// Used by history's `rolled_back` EXISTS check. The apply latch is
    /// stricter — [`Self::successful_rollback_targets`] — so a partial
    /// undo can finish the remaining inverses.
    pub async fn consolidation_run_was_rolled_back(&self, batch_key: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sleeptime_audit
                  WHERE origin = ?1 AND COALESCE(batch_id, run_id) = ?2",
                rusqlite::params![ROLLBACK_ORIGIN, batch_key],
                |r| r.get(0),
            )
            .context("checking for a prior rollback")?;
        Ok(n > 0)
    }

    /// The most recent consolidation runs, newest first — one row per
    /// invocation.
    ///
    /// Backs `/consolidate history`, and is the input PR-7 folds into
    /// the prompt so the consolidator can see how its previous batches
    /// fared. Grouping by `COALESCE(batch_id, run_id)` keeps pre-v16
    /// rows, which have no batch, collapsed under their session exactly
    /// as they were recorded.
    pub async fn recent_consolidation_runs(&self, limit: usize) -> Result<Vec<ConsolidationRun>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(a.batch_id, a.run_id)             AS batch_key,
                        MAX(a.run_id)                             AS run_id,
                        COUNT(*)                                  AS ops,
                        SUM(CASE WHEN applied = 1 THEN 1 ELSE 0 END) AS applied,
                        MIN(a.created_at)                         AS started_at,
                        MAX(a.prompt_version)                     AS prompt_version,
                        MAX(a.scope)                              AS scope,
                        EXISTS (SELECT 1 FROM sleeptime_audit rb
                                 WHERE rb.origin = ?1
                                   AND COALESCE(rb.batch_id, rb.run_id)
                                       = COALESCE(a.batch_id, a.run_id)) AS rolled_back
                   FROM sleeptime_audit a
                  WHERE a.origin = 'consolidation'
                  GROUP BY batch_key
                  ORDER BY MIN(a.created_at) DESC, batch_key DESC
                  LIMIT ?2",
            )
            .context("preparing consolidation history query")?;
        let rows = stmt
            .query_map(rusqlite::params![ROLLBACK_ORIGIN, limit as i64], |r| {
                Ok(ConsolidationRun {
                    batch_id: r.get(0)?,
                    run_id: r.get(1)?,
                    ops: r.get::<_, i64>(2)? as usize,
                    applied: r.get::<_, i64>(3)? as usize,
                    started_at: r.get(4)?,
                    prompt_version: r.get(5)?,
                    scope: r.get(6)?,
                    rolled_back: r.get::<_, i64>(7)? != 0,
                })
            })
            .context("querying consolidation history")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading consolidation history")?;
        Ok(rows)
    }

    /// Most eligible rows scanned when ranking by relevance.
    ///
    /// Ranking reads each candidate row's embedding and scores it in
    /// Rust, so the cost is linear in the eligible set. At ~2 800 rows
    /// (this workspace) that is a few megabytes read once per session —
    /// fine. The cap stops it degrading without bound as a store grows;
    /// above it the scan takes the most recent rows, which is a
    /// defensible bias for "what might this session contradict".
    const RELEVANCE_SCAN_CAP: usize = 5_000;

    /// Memories the consolidator may legitimately `MERGE` into or
    /// `SUPERSEDE`, for the scope its summary will land in.
    ///
    /// Everything at or broader than `scope_level` is fair game — a
    /// module-scoped session can still supersede a stale repo-wide
    /// belief — but nothing narrower, and nothing already superseded or
    /// run-local.
    ///
    /// `query` is the session's own material (its candidate texts). When
    /// present the rows are ranked by cosine similarity to it; without
    /// it they fall back to importance order.
    ///
    /// **Why relevance and not importance.** Importance order looks
    /// sensible and is useless here. In this workspace 327 of 2 782
    /// eligible rows sit at importance ≥ 0.99 — all of them bulk
    /// `llm_consolidated` swarm-output dumps — while genuinely useful
    /// sources top out lower (`user_remember` 0.8, `llm_extracted`
    /// 0.72, `llm_annotated` 0.67). With a 20-row cap the model was
    /// therefore shown the *same 20 topically-irrelevant rows every
    /// session*, and correctly refused to merge into any of them:
    /// four consecutive live runs produced zero MERGE and zero
    /// SUPERSEDE. Populating the list (the earlier C-5 fix) was
    /// necessary but not sufficient — the rows also have to be about
    /// the same subject as the session.
    ///
    /// Ranking failures degrade to importance order rather than
    /// aborting: a consolidator with a worse target list is still worth
    /// running.
    pub async fn consolidation_existing_memories(
        &self,
        scope_level: i32,
        repo_id: Option<&str>,
        limit: usize,
        query: Option<&str>,
    ) -> Result<Vec<ExistingMemoryRow>> {
        // Embed before taking the lock — never hold the store mutex
        // across an embedding call.
        let query_vec = match query.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => match self.embedder.embed_query(q).await {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        target: "memory_consolidator",
                        error = %e,
                        "could not embed the session for ranking; falling back to importance order"
                    );
                    None
                }
            },
            None => None,
        };

        let conn = self.conn.lock().await;
        match &query_vec {
            None => Self::existing_by_importance(&conn, scope_level, repo_id, limit),
            Some(v) => {
                let ranked = Self::existing_by_relevance(
                    &conn,
                    scope_level,
                    repo_id,
                    limit,
                    v,
                    self.embedder.name(),
                )?;
                // An embedder swap, or a store whose rows predate
                // embedding, can leave nothing comparable. Better the
                // old ordering than an empty list, which tells the model
                // not to emit MERGE or SUPERSEDE at all.
                if ranked.is_empty() {
                    Self::existing_by_importance(&conn, scope_level, repo_id, limit)
                } else {
                    Ok(ranked)
                }
            }
        }
    }

    /// Pre-ranking behaviour, kept as the fallback path.
    fn existing_by_importance(
        conn: &rusqlite::Connection,
        scope_level: i32,
        repo_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ExistingMemoryRow>> {
        let mut stmt = conn
            .prepare(
                "SELECT id, content, memory_type, scope_level, scope_path
                   FROM memories
                  WHERE superseded_by IS NULL
                    AND scope_level <= ?1
                    AND scope_level < ?2
                    AND (repo_id IS NULL OR ?3 IS NULL OR repo_id = ?3)
                  ORDER BY importance DESC, id DESC
                  LIMIT ?4",
            )
            .context("preparing existing-memories query")?;
        let rows = stmt
            .query_map(
                rusqlite::params![
                    scope_level,
                    crate::memory::scope::SCOPE_RUN,
                    repo_id,
                    limit as i64
                ],
                |r| {
                    Ok(ExistingMemoryRow {
                        id: r.get(0)?,
                        content: r.get(1)?,
                        memory_type: r.get(2)?,
                        scope_label: scope_label(r.get(3)?, r.get::<_, String>(4)?),
                    })
                },
            )
            .context("querying existing memories")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading existing memories")?;
        Ok(rows)
    }

    /// Rank the eligible set by cosine similarity to the session.
    ///
    /// Scored in Rust rather than through `vec_memories_scoped`, because
    /// the eligible set spans three scope levels and that virtual table
    /// is partitioned by one. `model_id` is matched against the active
    /// embedder for the same reason `vec_search_at_level` does it: a row
    /// embedded by a different model has a different dimensionality and
    /// would produce a meaningless distance.
    fn existing_by_relevance(
        conn: &rusqlite::Connection,
        scope_level: i32,
        repo_id: Option<&str>,
        limit: usize,
        query_vec: &[f32],
        model_id: &str,
    ) -> Result<Vec<ExistingMemoryRow>> {
        let mut stmt = conn
            .prepare(
                "SELECT id, content, memory_type, scope_level, scope_path, embedding
                   FROM memories
                  WHERE superseded_by IS NULL
                    AND scope_level <= ?1
                    AND scope_level < ?2
                    AND (repo_id IS NULL OR ?3 IS NULL OR repo_id = ?3)
                    AND embedding IS NOT NULL
                    AND model_id = ?4
                  ORDER BY id DESC
                  LIMIT ?5",
            )
            .context("preparing ranked existing-memories query")?;
        let mut scored: Vec<(f32, ExistingMemoryRow)> = stmt
            .query_map(
                rusqlite::params![
                    scope_level,
                    crate::memory::scope::SCOPE_RUN,
                    repo_id,
                    model_id,
                    Self::RELEVANCE_SCAN_CAP as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(5)?,
                        ExistingMemoryRow {
                            id: r.get(0)?,
                            content: r.get(1)?,
                            memory_type: r.get(2)?,
                            scope_label: scope_label(r.get(3)?, r.get::<_, String>(4)?),
                        },
                    ))
                },
            )
            .context("querying ranked existing memories")?
            .filter_map(|r| r.ok())
            .filter_map(|(blob, row)| {
                let v = super::blob_to_embedding(&blob)?;
                if v.len() != query_vec.len() {
                    return None;
                }
                Some((super::cosine_similarity(query_vec, &v), row))
            })
            .collect();

        // Descending similarity; ties broken by id so the order is
        // deterministic for a given store.
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.id.cmp(&a.1.id))
        });
        Ok(scored.into_iter().take(limit).map(|(_, row)| row).collect())
    }

    /// Clear a row's `superseded_by` edge, putting it back in play.
    ///
    /// The inverse of [`Self::supersede_memory`]; rollback uses it to
    /// reinstate the memory a `SUPERSEDE` retired.
    pub async fn clear_supersede(&self, memory_id: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let n = conn
            .execute(
                "UPDATE memories SET superseded_by = NULL, updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![memory_id],
            )
            .context("clearing superseded_by")?;
        Ok(n)
    }
}

/// A memory the consolidator is allowed to reference by id.
#[derive(Debug, Clone)]
pub struct ExistingMemoryRow {
    pub id: i64,
    pub content: String,
    pub memory_type: String,
    pub scope_label: String,
}

/// Human-readable scope, e.g. `repo` or `module:crates/foo`.
fn scope_label(scope_level: i32, scope_path: String) -> String {
    let name = match scope_level {
        crate::memory::scope::SCOPE_GLOBAL => "global",
        crate::memory::scope::SCOPE_WORKSPACE => "workspace",
        crate::memory::scope::SCOPE_REPO => "repo",
        crate::memory::scope::SCOPE_MODULE => "module",
        _ => "run",
    };
    if scope_path.is_empty() {
        name.to_string()
    } else {
        format!("{name}:{scope_path}")
    }
}

/// One consolidation run, summarised for `/consolidate history`.
#[derive(Debug, Clone)]
pub struct ConsolidationRun {
    /// Identifies the invocation. This is what `/consolidate rollback`
    /// takes. For pre-v16 rows it falls back to the session's `run_id`.
    pub batch_id: String,
    /// The session the invocation belonged to.
    pub run_id: String,
    /// Operations attempted in the run.
    pub ops: usize,
    /// How many of them landed.
    pub applied: usize,
    pub started_at: String,
    pub prompt_version: Option<String>,
    pub scope: Option<String>,
    /// Whether `/consolidate rollback` has already reversed it.
    pub rolled_back: bool,
}

/// A consolidation audit row as read back.
#[derive(Debug, Clone)]
pub struct ConsolidationAuditRecord {
    pub id: i64,
    /// The session the operation belonged to. Carried so a rollback can
    /// attribute its own audit rows without a second lookup.
    pub run_id: String,
    pub kind: String,
    pub memory_id: Option<i64>,
    pub related_id: Option<i64>,
    pub op_json: String,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub applied: bool,
    pub error: Option<String>,
    pub prompt_version: Option<String>,
    pub scope: Option<String>,
    /// What the consolidator predicted, when it said anything.
    pub expected_outcome: Option<String>,
    pub created_at: String,
}
