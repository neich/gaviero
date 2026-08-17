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

use anyhow::{Context, Result};

use super::MemoryStore;

/// One attempted consolidator operation, as recorded.
///
/// Borrowed rather than owned because every field is already in hand at
/// the call site and the row is written immediately.
pub struct ConsolidationAuditRow<'a> {
    /// The consolidation run this operation belonged to. Ties the rows
    /// of one batch together for `/consolidate history` and rollback.
    pub run_id: &'a str,
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
    /// State after it, so a rollback can tell whether the row it is
    /// about to reverse still looks the way this operation left it.
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
                 (run_id, kind, memory_id, related_id, payload, dry_run,
                  origin, before_json, after_json, applied, error,
                  prompt_version, scope, expected_outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, 0,
                     'consolidation', ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                row.run_id,
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

    /// Every consolidation audit row for `run_id`, oldest first.
    ///
    /// Ordered by insertion so a rollback can walk them in reverse and
    /// undo the batch in the opposite order to how it was applied.
    pub async fn consolidation_audit_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<ConsolidationAuditRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, memory_id, related_id, payload, before_json,
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
                    kind: r.get(1)?,
                    memory_id: r.get(2)?,
                    related_id: r.get(3)?,
                    op_json: r.get(4)?,
                    before_json: r.get(5)?,
                    after_json: r.get(6)?,
                    applied: r.get::<_, i64>(7)? != 0,
                    error: r.get(8)?,
                    prompt_version: r.get(9)?,
                    scope: r.get(10)?,
                    expected_outcome: r.get(11)?,
                    created_at: r.get(12)?,
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
    /// The run being undone. Rollback rows reuse it, so the pair
    /// (origin, run_id) is enough to find them.
    pub run_id: &'a str,
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
    /// Rollback rows reuse the original `run_id` and are distinguished
    /// by `origin`. That is deliberate: "has this run been rolled back"
    /// then answers itself with one indexed lookup, and no extra column
    /// is needed to point back at the run.
    pub async fn log_consolidation_rollback(&self, row: RollbackAuditRow<'_>) -> Result<()> {
        let RollbackAuditRow {
            run_id,
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
                 (run_id, kind, memory_id, related_id, payload, dry_run,
                  origin, applied, error)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
            rusqlite::params![
                run_id,
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

    /// Has `run_id` already been rolled back?
    ///
    /// Guards against a second rollback re-deleting rows the first one
    /// already reversed — the inverse operations are not idempotent.
    pub async fn consolidation_run_was_rolled_back(&self, run_id: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sleeptime_audit
                  WHERE origin = ?1 AND run_id = ?2",
                rusqlite::params![ROLLBACK_ORIGIN, run_id],
                |r| r.get(0),
            )
            .context("checking for a prior rollback")?;
        Ok(n > 0)
    }

    /// The most recent consolidation runs, newest first.
    ///
    /// Backs `/consolidate history`, and is the input PR-7 folds into
    /// the prompt so the consolidator can see how its previous batches
    /// fared.
    pub async fn recent_consolidation_runs(&self, limit: usize) -> Result<Vec<ConsolidationRun>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT run_id,
                        COUNT(*)                                  AS ops,
                        SUM(CASE WHEN applied = 1 THEN 1 ELSE 0 END) AS applied,
                        MIN(created_at)                           AS started_at,
                        MAX(prompt_version)                       AS prompt_version,
                        MAX(scope)                                AS scope,
                        EXISTS (SELECT 1 FROM sleeptime_audit rb
                                 WHERE rb.origin = ?1
                                   AND rb.run_id = a.run_id)      AS rolled_back
                   FROM sleeptime_audit a
                  WHERE a.origin = 'consolidation'
                  GROUP BY run_id
                  ORDER BY MIN(created_at) DESC, run_id DESC
                  LIMIT ?2",
            )
            .context("preparing consolidation history query")?;
        let rows = stmt
            .query_map(rusqlite::params![ROLLBACK_ORIGIN, limit as i64], |r| {
                Ok(ConsolidationRun {
                    run_id: r.get(0)?,
                    ops: r.get::<_, i64>(1)? as usize,
                    applied: r.get::<_, i64>(2)? as usize,
                    started_at: r.get(3)?,
                    prompt_version: r.get(4)?,
                    scope: r.get(5)?,
                    rolled_back: r.get::<_, i64>(6)? != 0,
                })
            })
            .context("querying consolidation history")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading consolidation history")?;
        Ok(rows)
    }

    /// Memories the consolidator may legitimately `MERGE` into or
    /// `SUPERSEDE`, for the scope its summary will land in.
    ///
    /// Everything at or broader than `scope_level` is fair game — a
    /// module-scoped session can still supersede a stale repo-wide
    /// belief — but nothing narrower, and nothing already superseded or
    /// run-local. Ordered by importance so a tight cap keeps the rows
    /// most worth reconciling.
    ///
    /// Before this existed the prompt was handed an empty list, so
    /// every id the model produced was necessarily invented.
    pub async fn consolidation_existing_memories(
        &self,
        scope_level: i32,
        repo_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ExistingMemoryRow>> {
        let conn = self.conn.lock().await;
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
