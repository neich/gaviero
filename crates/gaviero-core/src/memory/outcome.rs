//! How a chat turn or swarm unit ended. Stored in `turn_outcomes`, keyed
//! like its History row (`history:{session_id}:{turn_id}`). A label may
//! exist without a History row (cancelled turn, extractor off, failed
//! swarm unit); such labels are kept but never sampled.
//!
//! The names deliberately differ from `turn_capture::TurnOutcome` and
//! `history::TurnStatus`, which the TUI and CLI already import.

/// Which surface produced the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeOrigin {
    Chat,
    Swarm,
}

impl OutcomeOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Swarm => "swarm",
        }
    }
}

/// How the turn itself ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
    Completed,
    Cancelled,
    Error,
    Failed,
}

impl OutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
            Self::Failed => "failed",
        }
    }
}

/// What happened to the turn's changes afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeReview {
    Kept,
    Partial,
    Reverted,
    NoChanges,
    Undone,
    Merged,
    MergeConflict,
}

impl OutcomeReview {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Partial => "partial",
            Self::Reverted => "reverted",
            Self::NoChanges => "no_changes",
            Self::Undone => "undone",
            Self::Merged => "merged",
            Self::MergeConflict => "merge_conflict",
        }
    }

    /// Classify a finished TURN REVIEW from its kept / reverted file counts.
    pub fn from_counts(kept: u32, reverted: u32) -> Self {
        match (kept, reverted) {
            (0, 0) => Self::NoChanges,
            (_, 0) => Self::Kept,
            (0, _) => Self::Reverted,
            _ => Self::Partial,
        }
    }
}

/// Review values that mark a turn as a negative example.
pub const NEGATIVE_REVIEWS: [&str; 4] = ["reverted", "partial", "undone", "merge_conflict"];
/// Status values that mark a turn as a negative example.
pub const NEGATIVE_STATUSES: [&str; 2] = ["error", "failed"];
/// Review values that mark a turn as a positive example.
pub const POSITIVE_REVIEWS: [&str; 2] = ["kept", "merged"];
/// `detail` is cut to this many chars.
pub const MAX_DETAIL_CHARS: usize = 500;

/// One outcome label. `None` fields leave the stored value unchanged
/// (`upsert_outcome_label` merges a status label and a review label for
/// the same turn).
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeLabel {
    pub session_id: String,
    pub turn_id: String,
    pub origin: OutcomeOrigin,
    pub status: Option<OutcomeStatus>,
    pub review: Option<OutcomeReview>,
    pub files_kept: Option<u32>,
    pub files_reverted: Option<u32>,
    pub detail: Option<String>,
}

impl OutcomeLabel {
    /// How the turn ended; `detail` (e.g. the error) is cut to
    /// `MAX_DETAIL_CHARS` chars.
    pub fn status(
        origin: OutcomeOrigin,
        session_id: &str,
        turn_id: &str,
        status: OutcomeStatus,
        detail: Option<String>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            origin,
            status: Some(status),
            review: None,
            files_kept: None,
            files_reverted: None,
            detail: detail.map(|d| d.chars().take(MAX_DETAIL_CHARS).collect()),
        }
    }

    /// What happened to the turn's changes.
    pub fn review(
        origin: OutcomeOrigin,
        session_id: &str,
        turn_id: &str,
        review: OutcomeReview,
        kept: Option<u32>,
        reverted: Option<u32>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            origin,
            status: None,
            review: Some(review),
            files_kept: kept,
            files_reverted: reverted,
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_counts_classifies() {
        assert_eq!(OutcomeReview::from_counts(0, 0), OutcomeReview::NoChanges);
        assert_eq!(OutcomeReview::from_counts(3, 0), OutcomeReview::Kept);
        assert_eq!(OutcomeReview::from_counts(0, 2), OutcomeReview::Reverted);
        assert_eq!(OutcomeReview::from_counts(1, 1), OutcomeReview::Partial);
    }

    #[test]
    fn status_detail_is_truncated() {
        let long = "é".repeat(MAX_DETAIL_CHARS + 50);
        let label = OutcomeLabel::status(
            OutcomeOrigin::Swarm,
            "run",
            "swarm:run:u",
            OutcomeStatus::Failed,
            Some(long),
        );
        let detail = label.detail.expect("detail kept");
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS);
        assert_eq!(label.status, Some(OutcomeStatus::Failed));
        assert_eq!(label.review, None);
    }
}
