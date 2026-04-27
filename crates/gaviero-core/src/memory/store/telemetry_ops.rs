//! Tier B / B6: retrieval-use telemetry persistence + aggregation.
//!
//! Carved out of `store/mod.rs` (Phase 4.2). Owns the per-row
//! `record_retrieval_use` write and the `memory_utilization` /
//! `top_utilization_in_scope` aggregates that the sleeptime
//! trust-rescore step and the panel utilisation report consume. The
//! `sleeptime_prune_telemetry` retention step lives here too because
//! it operates against the `retrieval_use` table — it's a sleeptime
//! op by trigger but a telemetry op by data.

use anyhow::{Context, Result};

use super::{MemoryStore, stmt_or_err};

/// B6: aggregated retrieval-use stats for one memory.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryUtilization {
    pub memory_id: i64,
    pub times_injected: u32,
    pub times_used: u32,
    pub times_partial: u32,
    pub times_unused: u32,
    pub utilization_rate: f32,
    pub last_used_at: Option<String>,
}

impl MemoryStore {
    /// B5 / B6: prune `retrieval_use` rows older than `cutoff_days`.
    /// Aggregates feeding the sleeptime trust-rescore step are
    /// applied at sleeptime time; raw rows are no longer load-bearing.
    pub async fn sleeptime_prune_telemetry(&self, cutoff_days: u32) -> Result<usize> {
        let conn = self.conn.lock().await;
        let cutoff = format!("-{cutoff_days} days");
        let n = conn
            .execute(
                "DELETE FROM retrieval_use WHERE created_at < datetime('now', ?1)",
                rusqlite::params![cutoff],
            )
            .context("pruning retrieval_use")?;
        Ok(n)
    }

    /// B6: persist one classification row from the telemetry pass.
    pub async fn record_retrieval_use(
        &self,
        memory_id: i64,
        turn_id: &str,
        session_id: Option<&str>,
        injected_rank: i32,
        classification: &str,
        cosine_to_response: f32,
        substring_hit: bool,
    ) -> Result<i64> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO retrieval_use
                 (memory_id, turn_id, session_id, injected_rank,
                  classification, cosine_to_response, substring_hit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                memory_id,
                turn_id,
                session_id,
                injected_rank,
                classification,
                cosine_to_response,
                substring_hit as i64,
            ],
        )
        .context("inserting retrieval_use row")?;
        Ok(conn.last_insert_rowid())
    }

    /// B6: per-memory utilization aggregate. Computed on demand
    /// (cheap query against `retrieval_use`); no materialised view
    /// needed at this scale.
    pub async fn memory_utilization(&self, memory_ids: &[i64]) -> Result<Vec<MemoryUtilization>> {
        if memory_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().await;
        let placeholders = memory_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT memory_id,
                    COUNT(*) AS injected,
                    SUM(CASE WHEN classification = 'used' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN classification = 'partial' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN classification = 'unused' THEN 1 ELSE 0 END),
                    MAX(created_at)
             FROM retrieval_use
             WHERE memory_id IN ({placeholders})
             GROUP BY memory_id"
        );
        let mut stmt = conn.prepare(&sql).context("preparing memory_utilization")?;
        let params: Vec<&dyn rusqlite::ToSql> = memory_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params.iter()))
            .context("running memory_utilization")?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().context("utilization row")? {
            let memory_id: i64 = r.get(0)?;
            let injected: i64 = r.get(1)?;
            let used: i64 = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let partial: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let unused: i64 = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let last: Option<String> = r.get(5)?;
            out.push(MemoryUtilization {
                memory_id,
                times_injected: injected as u32,
                times_used: used as u32,
                times_partial: partial as u32,
                times_unused: unused as u32,
                utilization_rate: if injected > 0 {
                    used as f32 / injected as f32
                } else {
                    0.0
                },
                last_used_at: last,
            });
        }
        Ok(out)
    }

    /// B6: top-N rows by utilization rate within a scope. Used by the
    /// `gaviero-cli memory utilization` command.
    pub async fn top_utilization_in_scope(
        &self,
        scope_level: i32,
        ascending: bool,
        limit: usize,
    ) -> Result<Vec<(i64, MemoryUtilization)>> {
        let conn = self.conn.lock().await;
        let order = if ascending { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT m.id, COUNT(ru.id) AS injected,
                    SUM(CASE WHEN ru.classification = 'used' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN ru.classification = 'partial' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN ru.classification = 'unused' THEN 1 ELSE 0 END),
                    MAX(ru.created_at)
             FROM memories m
             LEFT JOIN retrieval_use ru ON ru.memory_id = m.id
             WHERE m.scope_level = ?1 AND m.superseded_by IS NULL
             GROUP BY m.id
             HAVING injected > 0
             ORDER BY (CAST(SUM(CASE WHEN ru.classification = 'used' THEN 1 ELSE 0 END) AS REAL)
                       / MAX(1.0, CAST(injected AS REAL))) {order}, injected DESC
             LIMIT ?2"
        );
        let mut stmt = stmt_or_err(&conn, &sql)?;
        let mut q = stmt
            .query(rusqlite::params![scope_level, limit as i64])
            .context("running top_utilization_in_scope")?;
        let mut out = Vec::new();
        while let Some(r) = q.next().context("top_utilization row")? {
            let id: i64 = r.get(0)?;
            let injected: i64 = r.get(1)?;
            let used: i64 = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let partial: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let unused: i64 = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let last: Option<String> = r.get(5)?;
            let util = MemoryUtilization {
                memory_id: id,
                times_injected: injected as u32,
                times_used: used as u32,
                times_partial: partial as u32,
                times_unused: unused as u32,
                utilization_rate: if injected > 0 {
                    used as f32 / injected as f32
                } else {
                    0.0
                },
                last_used_at: last,
            };
            out.push((id, util));
        }
        Ok(out)
    }
}
