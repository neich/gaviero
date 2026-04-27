//! C2.1: typed `deleted_by` tag for the audit trail.
//!
//! Every row in the `deletions` table carries a `deleted_by` string
//! identifying which path produced the soft-delete. The set is
//! closed (no free-form values) so retention policies, restore
//! eligibility, and panel grouping can match on it cleanly.
//!
//! Restore eligibility:
//! - `UserCommand` / `Panel` / `SleeptimeMerge` / `SleeptimePrune` are
//!   soft-deletes. The `original_row_json` carries the full row body
//!   so `/restore` (C2.2) can reinstate them within the configured
//!   retention window.
//! - `UserRedaction` is **one-way** — see C2.4. The audit row exists
//!   for the audit trail (timestamp + SHA + actor) but the body in
//!   `original_row_json` is the post-redaction tombstone, not the
//!   original transcript. There is no path back.

use serde::{Deserialize, Serialize};

/// One row returned from `MemoryStore::recent_deletions` /
/// `get_deletion`. Mirrors the `deletions` table; `original_row_json`
/// is the verbatim serialized row for the restore path (C2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedRow {
    pub id: i64,
    pub memory_id: i64,
    pub memory_content_hash: Option<String>,
    pub memory_kind: String,
    pub memory_source: String,
    pub memory_trust: f32,
    pub deleted_at: String,
    pub deleted_by: String,
    pub reason: Option<String>,
    pub original_row_json: String,
}

impl DeletedRow {
    /// Typed accessor for [`Self::deleted_by`]. Returns `None` for
    /// rows persisted by a future code-path the current binary does
    /// not recognize (forward compat).
    pub fn deleted_by_typed(&self) -> Option<DeletedBy> {
        DeletedBy::parse_str(&self.deleted_by)
    }

    /// Convenience: is this audit row eligible for restore? See
    /// [`DeletedBy::is_restorable`].
    pub fn is_restorable(&self) -> bool {
        self.deleted_by_typed()
            .map(|d| d.is_restorable())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletedBy {
    /// CLI / TUI slash-command bulk delete: `/forget`, `/forget-scope`,
    /// `/forget-type`, `/forget-source`, `gaviero-cli memory forget`.
    UserCommand,
    /// TUI memory panel per-row `d` action.
    Panel,
    /// Sleeptime near-dup merge: the loser row is soft-deleted with
    /// `original_row_json.merged_into = <winner_id>` so a restore
    /// can carry the merge edge through the dedup pipeline.
    SleeptimeMerge,
    /// Sleeptime summary retention expiry (>365d) — short retention
    /// window (default 14d) before hard-delete.
    SleeptimePrune,
    /// C2.4 `/forget-history` tombstone path. **Not restorable** —
    /// `original_row_json` stores the post-redaction tombstone, not
    /// the original transcript.
    UserRedaction,
}

impl DeletedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserCommand => "user_command",
            Self::Panel => "panel",
            Self::SleeptimeMerge => "sleeptime_merge",
            Self::SleeptimePrune => "sleeptime_prune",
            Self::UserRedaction => "user_redaction",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "user_command" => Some(Self::UserCommand),
            "panel" => Some(Self::Panel),
            "sleeptime_merge" => Some(Self::SleeptimeMerge),
            "sleeptime_prune" => Some(Self::SleeptimePrune),
            "user_redaction" => Some(Self::UserRedaction),
            _ => None,
        }
    }

    /// Restore eligibility: only `UserRedaction` is one-way.
    /// All other tags allow restore within the retention window.
    pub fn is_restorable(self) -> bool {
        !matches!(self, Self::UserRedaction)
    }

    /// Default retention window (in days) before the audit row is
    /// hard-deleted. Sleeptime prune expirations get a shorter window
    /// (14d) than user-driven deletions (30d) per the plan; redactions
    /// share the user-driven window so the actor + reason persist long
    /// enough to be auditable.
    pub fn default_retention_days(self) -> u32 {
        match self {
            Self::SleeptimePrune => 14,
            _ => 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_str() {
        for v in [
            DeletedBy::UserCommand,
            DeletedBy::Panel,
            DeletedBy::SleeptimeMerge,
            DeletedBy::SleeptimePrune,
            DeletedBy::UserRedaction,
        ] {
            assert_eq!(DeletedBy::parse_str(v.as_str()), Some(v));
        }
        assert_eq!(DeletedBy::parse_str("nonsense"), None);
    }

    #[test]
    fn only_redaction_is_one_way() {
        assert!(DeletedBy::UserCommand.is_restorable());
        assert!(DeletedBy::Panel.is_restorable());
        assert!(DeletedBy::SleeptimeMerge.is_restorable());
        assert!(DeletedBy::SleeptimePrune.is_restorable());
        assert!(!DeletedBy::UserRedaction.is_restorable());
    }

    #[test]
    fn retention_defaults_match_plan() {
        assert_eq!(DeletedBy::UserCommand.default_retention_days(), 30);
        assert_eq!(DeletedBy::Panel.default_retention_days(), 30);
        assert_eq!(DeletedBy::SleeptimeMerge.default_retention_days(), 30);
        assert_eq!(DeletedBy::SleeptimePrune.default_retention_days(), 14);
        assert_eq!(DeletedBy::UserRedaction.default_retention_days(), 30);
    }

    #[test]
    fn serde_lowercase() {
        let s = serde_json::to_string(&DeletedBy::SleeptimeMerge).unwrap();
        assert_eq!(s, "\"sleeptime_merge\"");
        let v: DeletedBy = serde_json::from_str("\"user_redaction\"").unwrap();
        assert_eq!(v, DeletedBy::UserRedaction);
    }
}
