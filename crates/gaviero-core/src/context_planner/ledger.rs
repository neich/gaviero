//! Session ledger and fingerprint (M1 of PROVIDER_PLAN_V9).
//!
//! `SessionLedger` is the *single source of truth* for client-side replay
//! history, injected memory ids, attached/outlined files, and continuity
//! handles. V9 §0 rule 4: do not duplicate `replay_history` anywhere.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::types::{ContinuityHandle, ContinuityMode, ProviderProfile};

/// Conversational role used by replay history.
///
/// Defined planner-side; M5 transports reference it via `Turn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Content digest used to detect file changes and re-attach.
///
/// Opaque newtype around a hex-encoded hash. M3 will populate; M1 doesn't
/// write to this field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentDigest(pub String);

// V9 §4 `GraphDecision` is owned by `crate::repo_map` (see module-cycle
// rationale on `MemoryCandidate`). Re-exported here so ledger consumers
// see it at its V9-spec address.
pub use crate::repo_map::GraphDecision;

/// Placeholder for compaction summary record.
///
/// V9 §4 — populated in M9 when compaction lands. M1 keeps the field on the
/// ledger so M9 only adds population logic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactionRecord {
    pub turns_compacted: u32,
    pub summary: String,
    pub created_at: Option<SystemTime>,
}

/// Inputs to a [`PlannerFingerprint`]. Any mismatch invalidates the ledger
/// (V9 §4 invalidation triggers list).
///
/// Reviewers: when adding a new field, also update the M4 invalidation
/// logic. Until M4 lands, [`SessionLedger::invalidate_if_fingerprint_changed`]
/// is a stub that always returns `false` — so fingerprint plumbing here is
/// load-bearing for *future* milestones, not M1 behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerFingerprint {
    pub provider: String,
    pub model: String,
    pub system_prompt_digest: String,
    pub toolset_digest: String,
    pub workspace_root_digest: String,
    pub branch_name: Option<String>,
}

impl PlannerFingerprint {
    /// Minimal fingerprint from a [`ProviderProfile`]. Other fields default
    /// empty; M4 wires real digests of system prompt / toolset / workspace.
    pub fn from_profile(profile: &ProviderProfile) -> Self {
        Self {
            provider: profile.provider.clone(),
            model: profile.model.clone(),
            system_prompt_digest: String::new(),
            toolset_digest: String::new(),
            workspace_root_digest: String::new(),
            branch_name: None,
        }
    }
}

/// Per-conversation (or per-swarm-attempt) state ledger.
///
/// V9 §4 type, copied verbatim including doc-comments.
#[derive(Debug, Clone)]
pub struct SessionLedger {
    pub continuity_mode: ContinuityMode,
    pub continuity_handle: Option<ContinuityHandle>,

    pub injected_memory_ids: HashSet<i64>,

    /// Files whose full content was attached. Digest lets us detect
    /// modification and re-attach when the file changed.
    pub injected_file_digests: HashMap<PathBuf, ContentDigest>,

    /// Files seen by the model as outline/signature only. Kept distinct
    /// from attached so the planner can upgrade outline -> FullAttach
    /// without re-injecting the outline.
    pub injected_outline_paths: HashSet<PathBuf>,

    pub injected_graph_decisions: HashMap<PathBuf, GraphDecision>,

    /// THE single source of truth for client-side replay history.
    /// Used by StatelessReplay providers. Never mirrored elsewhere.
    /// Specifically: do not add a `conversation_history` field on
    /// `PlannerInput` — read from `self.replay_history` instead.
    pub replay_history: Vec<(Role, String)>,

    pub compacted_summary: Option<CompactionRecord>,

    pub turn_count: u32,
    pub last_successful_resume: Option<SystemTime>,

    pub fingerprint: PlannerFingerprint,
}

impl SessionLedger {
    /// New ledger seeded from a provider profile.
    pub fn new(profile: &ProviderProfile, fingerprint: PlannerFingerprint) -> Self {
        Self {
            continuity_mode: profile.continuity_mode,
            continuity_handle: None,
            injected_memory_ids: HashSet::new(),
            injected_file_digests: HashMap::new(),
            injected_outline_paths: HashSet::new(),
            injected_graph_decisions: HashMap::new(),
            replay_history: Vec::new(),
            compacted_summary: None,
            turn_count: 0,
            last_successful_resume: None,
            fingerprint,
        }
    }

    /// Replaces the ad-hoc `is_first_turn = resume_session_id.is_none()`
    /// check at side_panel.rs:764. Equivalent for M1; M4 makes this
    /// load-bearing across restarts.
    pub fn is_first_turn(&self) -> bool {
        self.turn_count == 0
    }

    pub fn record_turn_dispatched(&mut self) {
        self.turn_count = self.turn_count.saturating_add(1);
    }

    pub fn record_replay(&mut self, role: Role, content: String) {
        self.replay_history.push((role, content));
    }

    pub fn record_continuity_handle(&mut self, handle: ContinuityHandle) {
        self.continuity_handle = Some(handle);
    }

    /// Stub for M4 invalidation logic. Returns `false` (no invalidation)
    /// until M4 wires the real comparison. Existence here keeps the API
    /// surface stable so M4 doesn't leak new types into call sites.
    pub fn invalidate_if_fingerprint_changed(&mut self, _current: &PlannerFingerprint) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::types::{build_provider_profile, ModelSpec, RuntimeConfig};

    fn fixture_ledger() -> SessionLedger {
        let profile = build_provider_profile(
            &ModelSpec::parse("claude-code:sonnet"),
            &RuntimeConfig::default(),
        );
        let fp = PlannerFingerprint::from_profile(&profile);
        SessionLedger::new(&profile, fp)
    }

    #[test]
    fn new_ledger_is_first_turn() {
        let l = fixture_ledger();
        assert!(l.is_first_turn());
        assert_eq!(l.turn_count, 0);
        assert!(l.continuity_handle.is_none());
        assert!(l.replay_history.is_empty());
    }

    #[test]
    fn record_turn_clears_first_turn() {
        let mut l = fixture_ledger();
        l.record_turn_dispatched();
        assert!(!l.is_first_turn());
        assert_eq!(l.turn_count, 1);
    }

    #[test]
    fn record_replay_appends() {
        let mut l = fixture_ledger();
        l.record_replay(Role::User, "hi".to_string());
        l.record_replay(Role::Assistant, "hello".to_string());
        assert_eq!(l.replay_history.len(), 2);
        assert_eq!(l.replay_history[0].0, Role::User);
    }

    #[test]
    fn record_continuity_handle_sets_handle() {
        let mut l = fixture_ledger();
        l.record_continuity_handle(ContinuityHandle::ClaudeSessionId("abc".into()));
        match l.continuity_handle {
            Some(ContinuityHandle::ClaudeSessionId(id)) => assert_eq!(id, "abc"),
            other => panic!("unexpected handle: {:?}", other),
        }
    }

    #[test]
    fn invalidation_stub_returns_false() {
        // M1 stub. M4 will replace the body and update this test.
        let mut l = fixture_ledger();
        let fp = l.fingerprint.clone();
        assert!(!l.invalidate_if_fingerprint_changed(&fp));
    }
}
