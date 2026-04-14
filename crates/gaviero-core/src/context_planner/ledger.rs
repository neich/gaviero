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

/// Serializable projection of [`SessionLedger`] for on-disk persistence
/// (V9 §11 M4). Fields are a subset of the in-memory ledger suitable for
/// cross-restart round-trip:
///
/// * `continuity_handle` — the whole point of persistence.
/// * `fingerprint` — used to invalidate the handle on model/tool changes.
/// * `turn_count` — so `is_first_turn()` returns the right value post-restart.
/// * `replay_history` — needed for `StatelessReplay` providers (Ollama M9).
/// * `last_successful_resume` — flat `u64` (unix seconds) to avoid serde-of-SystemTime.
///
/// `injected_*` fields are intentionally excluded: the planner recomputes
/// them per turn; they're first-turn-scoped and don't survive restart.
/// M9 will add `compacted_summary` here when compaction lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedLedger {
    pub continuity_handle: Option<ContinuityHandle>,
    pub fingerprint: PlannerFingerprint,
    pub turn_count: u32,
    #[serde(default)]
    pub replay_history: Vec<(Role, String)>,
    #[serde(default)]
    pub last_successful_resume_unix: Option<u64>,
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

    /// V9 §11 M4: compare the ledger's fingerprint to the one a caller
    /// computed from the current runtime and invalidate if they differ.
    /// Invalidation wipes continuity state (handle, replay history,
    /// injected_* sets, turn_count) so the next send bootstraps fresh.
    ///
    /// Returns `true` iff invalidation happened.
    pub fn invalidate_if_fingerprint_changed(&mut self, current: &PlannerFingerprint) -> bool {
        if &self.fingerprint == current {
            return false;
        }
        tracing::info!(
            target: "turn_metrics",
            reason = "fingerprint_mismatch",
            was_provider = %self.fingerprint.provider,
            was_model = %self.fingerprint.model,
            now_provider = %current.provider,
            now_model = %current.model,
            "ledger_invalidated"
        );
        self.continuity_handle = None;
        self.injected_memory_ids.clear();
        self.injected_file_digests.clear();
        self.injected_outline_paths.clear();
        self.injected_graph_decisions.clear();
        self.replay_history.clear();
        self.turn_count = 0;
        self.last_successful_resume = None;
        self.fingerprint = current.clone();
        true
    }

    /// Serializable projection for on-disk persistence. M4.
    pub fn to_persisted(&self) -> PersistedLedger {
        PersistedLedger {
            continuity_handle: self.continuity_handle.clone(),
            fingerprint: self.fingerprint.clone(),
            turn_count: self.turn_count,
            replay_history: self.replay_history.clone(),
            last_successful_resume_unix: self.last_successful_resume.and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
            }),
        }
    }

    /// Reconstruct an in-memory ledger from persisted state.
    ///
    /// `continuity_mode` is *not* persisted — the caller supplies the
    /// current `ProviderProfile` so model changes force a fresh mode
    /// rather than trusting stale on-disk data. `injected_*` fields are
    /// empty after restore (recomputed per turn from the planner).
    pub fn from_persisted(persisted: PersistedLedger, profile: &ProviderProfile) -> Self {
        Self {
            continuity_mode: profile.continuity_mode,
            continuity_handle: persisted.continuity_handle,
            injected_memory_ids: HashSet::new(),
            injected_file_digests: HashMap::new(),
            injected_outline_paths: HashSet::new(),
            injected_graph_decisions: HashMap::new(),
            replay_history: persisted.replay_history,
            compacted_summary: None,
            turn_count: persisted.turn_count,
            last_successful_resume: persisted
                .last_successful_resume_unix
                .map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s)),
            fingerprint: persisted.fingerprint,
        }
    }

    /// Record that the provider confirmed resume succeeded.
    /// Called from the `SystemInit` handler when `resume_accepted=true`.
    pub fn record_resume_success(&mut self) {
        self.last_successful_resume = Some(std::time::SystemTime::now());
    }

    /// Record that the provider rejected the resume attempt.
    /// Clears the stale handle so the next send bootstraps fresh.
    pub fn record_resume_failure(&mut self) {
        tracing::warn!(
            target: "turn_metrics",
            "ledger resume failure — clearing continuity handle"
        );
        self.continuity_handle = None;
        self.turn_count = 0;
        self.last_successful_resume = None;
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
    fn m4_invalidation_noop_when_fingerprint_matches() {
        let mut l = fixture_ledger();
        l.record_turn_dispatched();
        l.record_continuity_handle(ContinuityHandle::ClaudeSessionId("abc".into()));
        let fp = l.fingerprint.clone();
        assert!(!l.invalidate_if_fingerprint_changed(&fp));
        // State preserved.
        assert_eq!(l.turn_count, 1);
        assert!(l.continuity_handle.is_some());
    }

    #[test]
    fn m4_invalidation_clears_state_on_mismatch() {
        // V9 §11 M4 acceptance: "Model change invalidates stored handle".
        let mut l = fixture_ledger();
        l.record_turn_dispatched();
        l.record_continuity_handle(ContinuityHandle::ClaudeSessionId("abc".into()));
        l.injected_memory_ids.insert(42);
        l.injected_outline_paths.insert("src/foo.rs".into());

        let mut different_fp = l.fingerprint.clone();
        different_fp.model = "opus".to_string(); // was sonnet → mismatch
        assert!(l.invalidate_if_fingerprint_changed(&different_fp));

        assert_eq!(l.turn_count, 0);
        assert!(l.continuity_handle.is_none());
        assert!(l.injected_memory_ids.is_empty());
        assert!(l.injected_outline_paths.is_empty());
        assert_eq!(l.fingerprint.model, "opus");
    }

    #[test]
    fn m4_persisted_ledger_round_trips() {
        // V9 §11 M4 acceptance: "Persisted ContinuityHandle round-trips
        // with correct variant".
        let mut l = fixture_ledger();
        l.record_turn_dispatched();
        l.record_turn_dispatched();
        l.record_continuity_handle(ContinuityHandle::ClaudeSessionId("abc-123".into()));
        l.record_replay(Role::User, "hi".into());
        l.record_resume_success();

        let persisted = l.to_persisted();
        let json = serde_json::to_string(&persisted).unwrap();
        // Explicit variant tag (forward-compat per V9 §4 ContinuityHandle doc).
        assert!(json.contains("ClaudeSessionId"));

        let back: PersistedLedger = serde_json::from_str(&json).unwrap();
        let profile =
            crate::context_planner::types::build_provider_profile(
                &crate::context_planner::types::ModelSpec::parse("claude-code:sonnet"),
                &crate::context_planner::types::RuntimeConfig::default(),
            );
        let restored = SessionLedger::from_persisted(back, &profile);

        assert_eq!(restored.turn_count, 2);
        match restored.continuity_handle {
            Some(ContinuityHandle::ClaudeSessionId(id)) => assert_eq!(id, "abc-123"),
            other => panic!("lost variant after round-trip: {:?}", other),
        }
        assert_eq!(restored.replay_history.len(), 1);
        assert!(restored.last_successful_resume.is_some());
    }

    #[test]
    fn m4_resume_failure_clears_handle() {
        let mut l = fixture_ledger();
        l.record_turn_dispatched();
        l.record_continuity_handle(ContinuityHandle::ClaudeSessionId("stale".into()));
        l.record_resume_failure();
        assert!(l.continuity_handle.is_none());
        assert_eq!(l.turn_count, 0);
        // is_first_turn() now returns true so bootstrap fires on next send.
        assert!(l.is_first_turn());
    }
}
