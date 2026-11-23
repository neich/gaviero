//! Turn outcome labels (`turn_outcomes`, schema v20).
//!
//! One row per chat turn / swarm unit, keyed like its History row
//! (`history:{session_id}:{turn_id}`). A status label and a review label
//! for the same turn merge into one row. History rows are never touched.

use anyhow::{Context, Result};

use super::MemoryStore;
use crate::memory::outcome::OutcomeLabel;

/// One stored outcome label.
#[derive(Debug, Clone)]
pub struct OutcomeLabelRow {
    pub session_id: String,
    pub turn_id: String,
    pub origin: String,
    pub status: Option<String>,
    pub review: Option<String>,
    pub files_kept: Option<i64>,
    pub files_reverted: Option<i64>,
    pub detail: Option<String>,
    pub updated_at: String,
}

impl MemoryStore {
    /// Insert or merge one label. `None` fields keep the stored value;
    /// `origin` stays the one from the first insert.
    pub async fn upsert_outcome_label(&self, o: &OutcomeLabel) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO turn_outcomes
                 (session_id, turn_id, origin, status, review, files_kept, files_reverted, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(session_id, turn_id) DO UPDATE SET
               status         = COALESCE(excluded.status, turn_outcomes.status),
               review         = COALESCE(excluded.review, turn_outcomes.review),
               files_kept     = COALESCE(excluded.files_kept, turn_outcomes.files_kept),
               files_reverted = COALESCE(excluded.files_reverted, turn_outcomes.files_reverted),
               detail         = COALESCE(excluded.detail, turn_outcomes.detail),
               updated_at     = strftime('%Y-%m-%d %H:%M:%f','now')",
            rusqlite::params![
                o.session_id,
                o.turn_id,
                o.origin.as_str(),
                o.status.map(|s| s.as_str()),
                o.review.map(|r| r.as_str()),
                o.files_kept.map(i64::from),
                o.files_reverted.map(i64::from),
                o.detail.as_deref(),
            ],
        )
        .context("upserting turn_outcomes row")?;
        Ok(())
    }

    /// The stored label for one turn, if any.
    pub async fn outcome_label(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<OutcomeLabelRow>> {
        let conn = self.conn.lock().await;
        let row = conn.query_row(
            "SELECT session_id, turn_id, origin, status, review, files_kept,
                    files_reverted, detail, updated_at
               FROM turn_outcomes
              WHERE session_id = ?1 AND turn_id = ?2",
            rusqlite::params![session_id, turn_id],
            |r| {
                Ok(OutcomeLabelRow {
                    session_id: r.get(0)?,
                    turn_id: r.get(1)?,
                    origin: r.get(2)?,
                    status: r.get(3)?,
                    review: r.get(4)?,
                    files_kept: r.get(5)?,
                    files_reverted: r.get(6)?,
                    detail: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            },
        );
        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("reading turn_outcomes row: {e}")),
        }
    }

    /// SQLite's clock with milliseconds, in the format `turn_outcomes`
    /// timestamps use. The pattern maintainer takes its watermark here.
    pub async fn sqlite_now_ms(&self) -> Result<String> {
        let conn = self.conn.lock().await;
        conn.query_row("SELECT strftime('%Y-%m-%d %H:%M:%f','now')", [], |r| {
            r.get(0)
        })
        .context("reading sqlite clock")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::memory::embedder::NullEmbedder;
    use crate::memory::outcome::{OutcomeOrigin, OutcomeReview, OutcomeStatus};

    #[tokio::test]
    async fn upsert_merges_status_and_review() {
        let store = MemoryStore::in_memory(Arc::new(NullEmbedder::new(8))).unwrap();
        store
            .upsert_outcome_label(&OutcomeLabel::status(
                OutcomeOrigin::Chat,
                "conv",
                "t1",
                OutcomeStatus::Completed,
                None,
            ))
            .await
            .unwrap();
        let first = store.outcome_label("conv", "t1").await.unwrap().unwrap();
        assert_eq!(first.status.as_deref(), Some("completed"));
        assert_eq!(first.review, None);

        store
            .upsert_outcome_label(&OutcomeLabel::review(
                OutcomeOrigin::Swarm,
                "conv",
                "t1",
                OutcomeReview::Partial,
                Some(1),
                Some(2),
            ))
            .await
            .unwrap();
        let merged = store.outcome_label("conv", "t1").await.unwrap().unwrap();
        assert_eq!(merged.origin, "chat", "origin comes from the first insert");
        assert_eq!(merged.status.as_deref(), Some("completed"));
        assert_eq!(merged.review.as_deref(), Some("partial"));
        assert_eq!(merged.files_kept, Some(1));
        assert_eq!(merged.files_reverted, Some(2));
        assert!(merged.updated_at >= first.updated_at);
        assert!(store.outcome_label("conv", "t2").await.unwrap().is_none());
    }
}
