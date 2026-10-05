//! The record of what one turn changed, and the review decisions on it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed,
}

/// One side (before or after) of a changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    /// SHA-256 hex. `None` only past the first-baseline cap (stat-tracked).
    pub sha256: Option<String>,
    pub size: u64,
    /// Content is in the blob store (≤ [`super::MAX_BLOB_BYTES`]).
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Absolute path.
    pub path: PathBuf,
    /// Workspace folder root the path belongs to.
    pub root: PathBuf,
    /// Root-relative path, `/`-separated (display + history).
    pub rel: String,
    pub kind: ChangeKind,
    /// Pre-turn state; `None` = did not exist.
    pub before: Option<BlobRef>,
    /// Post-turn state; `None` = deleted.
    pub after: Option<BlobRef>,
    /// The pre-turn state can be restored (its content is stored, or the file
    /// did not exist).
    pub revertible: bool,
    /// Either side is not UTF-8 text: whole-file decisions only.
    #[serde(default)]
    pub binary: bool,
    /// Turn ids whose window overlapped this turn and changed the same path.
    #[serde(default)]
    pub overlap_with: Vec<String>,
}

impl FileChange {
    pub fn before_hash(&self) -> Option<&str> {
        self.before.as_ref().and_then(|b| b.sha256.as_deref())
    }

    pub fn after_hash(&self) -> Option<&str> {
        self.after.as_ref().and_then(|b| b.sha256.as_deref())
    }

    /// Key used for review decisions (absolute path, `/`-separated).
    pub fn key(&self) -> String {
        self.path.to_string_lossy().replace('\\', "/")
    }
}

/// Everything one turn changed inside the capture scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnChangeSet {
    pub turn_id: String,
    pub conv_id: Option<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub outcome: TurnOutcome,
    pub files: Vec<FileChange>,
    /// Human-readable notes (baseline capped, sensitive paths reverted, …).
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Sensitive paths restored automatically at turn end (root-relative).
    #[serde(default)]
    pub auto_reverted: Vec<String>,
}

impl TurnChangeSet {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// A reviewer's decision on one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision", content = "hunks")]
pub enum FileDecision {
    Keep,
    Revert,
    /// Revert only these hunk indices (from [`super::revert::file_hunks`]).
    RevertHunks(Vec<usize>),
}

/// A change set awaiting review, persisted so it survives restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingReview {
    pub set: TurnChangeSet,
    /// Decisions so far, keyed by [`FileChange::key`].
    #[serde(default)]
    pub decisions: BTreeMap<String, FileDecision>,
}

impl PendingReview {
    pub fn new(set: TurnChangeSet) -> Self {
        Self {
            set,
            decisions: BTreeMap::new(),
        }
    }
}

/// Outcome of applying one file's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result", content = "detail")]
pub enum ResolvedDecision {
    Kept,
    Reverted,
    RevertedHunks(usize),
    Failed(String),
}

/// The archived form of a reviewed turn (`sets/<turn_id>.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedTurn {
    pub set: TurnChangeSet,
    #[serde(default)]
    pub resolved: BTreeMap<String, ResolvedDecision>,
}
