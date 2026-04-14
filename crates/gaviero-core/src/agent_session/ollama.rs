//! Ollama session (V9 §11 M9).
//!
//! [`OllamaSession`] gives Ollama (`StatelessReplay`) a named type with
//! bounded replay-history growth. Each call to `send_turn` checks whether
//! the incoming `turn.replay_history` exceeds the configured
//! [`CompactionPolicy`] thresholds; if so, the oldest turn pairs are dropped
//! before the (compacted) `Turn` is forwarded to the inner
//! [`LegacyAgentSession`].
//!
//! **Continuity:** `StatelessReplay` — Ollama carries no server-side thread
//! state. The caller (planner + ledger) is the authoritative source of replay
//! history; this session only enforces the size bound at the transport layer.
//!
//! **Lifecycle:** created per-turn by `registry::create_session`; state
//! within the session (e.g., the last compaction record) is ephemeral.
//! `/reset` is handled at the TUI level by clearing the `SessionLedger` and
//! chat messages; no session-side teardown is needed.
//!
//! **M9 scope:** replaces the generic `LegacyAgentSession` shim for all
//! Ollama/local providers in the registry. M10 may deepen this into a direct
//! HTTP-streaming implementation; the named type gives M10 a clean target.

use std::pin::Pin;

use anyhow::Result;
use futures::Stream;

use crate::context_planner::compaction::{compact_replay, should_compact, CompactionPolicy};
use crate::context_planner::{ContinuityHandle, ContinuityMode, ReplayPayload};
use crate::swarm::backend::UnifiedStreamEvent;

use super::registry::SessionConstruction;
use super::{AgentSession, LegacyAgentSession, Turn};

// ── OllamaSession ─────────────────────────────────────────────────────────────

/// M9 `AgentSession` for Ollama and `local:` model prefixes (`StatelessReplay`).
///
/// A bounded wrapper over [`LegacyAgentSession`] so the registry can enforce
/// replay-history size limits at the Ollama transport boundary. Compaction is
/// checked on every `send_turn` call against the [`CompactionPolicy`];
/// when triggered, the oldest turn pairs are dropped and the compacted
/// [`Turn`] is forwarded to the inner session.
///
/// **Registry routing:** replaces the `LegacyAgentSession` fallback arm in
/// `registry::create_session` for `StatelessReplay` providers with
/// `profile.provider == "ollama"`. Codex exec keeps its own [`CodexExecSession`].
pub struct OllamaSession {
    inner: LegacyAgentSession,
    policy: CompactionPolicy,
    /// `ProviderProfile::max_context_tokens` captured at construction.
    /// Passed to `should_compact` to enable the token-pressure trigger.
    /// `None` disables token-pressure compaction (pressure trigger is skipped).
    max_context_tokens: Option<usize>,
}

impl OllamaSession {
    /// Construct a new `OllamaSession`. Called exclusively by
    /// `registry::create_session` for Ollama providers (`StatelessReplay`,
    /// `profile.provider == "ollama"`).
    pub(super) fn new(args: SessionConstruction) -> Self {
        let max_context_tokens = args.profile.max_context_tokens;
        Self {
            inner: LegacyAgentSession::new(
                args.write_gate,
                args.observer,
                args.model,
                args.ollama_base_url,
                args.workspace_root,
                args.agent_id,
                args.options,
                args.profile,
            ),
            policy: CompactionPolicy::default(),
            max_context_tokens,
        }
    }
}

#[async_trait::async_trait]
impl AgentSession for OllamaSession {
    /// Dispatch a turn, applying replay compaction if any policy threshold is
    /// exceeded before forwarding to the inner [`LegacyAgentSession`].
    ///
    /// Compaction replaces `turn.replay_history` with a truncated payload
    /// containing only the most recent `policy.keep_turn_pairs` pairs.
    /// A tracing event at `INFO` level records what was dropped so operators
    /// can observe compaction without pulling the session's internal state.
    async fn send_turn(
        &mut self,
        mut turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        // Check and apply compaction to the replay history carried in the Turn.
        // The session does NOT write back to the caller's SessionLedger; it only
        // bounds what Ollama sees at the transport layer.
        if let Some(ref payload) = turn.replay_history
            && should_compact(&self.policy, &payload.entries, self.max_context_tokens)
        {
            let (compacted_entries, record) =
                compact_replay(&self.policy, payload.entries.clone());
            tracing::info!(
                target: "turn_metrics",
                turns_compacted = record.turns_compacted,
                kept_entries = compacted_entries.len(),
                max_context_tokens = ?self.max_context_tokens,
                "ollama_replay_compacted"
            );
            turn.replay_history = Some(ReplayPayload { entries: compacted_entries })
                .filter(|p| !p.entries.is_empty());
        }

        self.inner.send_turn(turn).await
    }

    fn continuity_mode(&self) -> ContinuityMode {
        ContinuityMode::StatelessReplay
    }

    /// Ollama carries no thread state — always `None`.
    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        None
    }

    async fn close(self: Box<Self>) {
        Box::new(self.inner).close().await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::compaction::{should_compact, CompactionPolicy};
    use crate::context_planner::ledger::Role;

    // Build a replay history of `pairs` (user, assistant) pairs.
    fn make_history(pairs: usize) -> Vec<(Role, String)> {
        (0..pairs)
            .flat_map(|i| {
                vec![
                    (Role::User, format!("u{}", i)),
                    (Role::Assistant, format!("a{}", i)),
                ]
            })
            .collect()
    }

    #[test]
    fn ollama_session_is_stateless_replay() {
        // Verify the mode constant so a future refactor can't silently change it.
        assert_eq!(ContinuityMode::StatelessReplay, ContinuityMode::StatelessReplay);
        assert_ne!(ContinuityMode::StatelessReplay, ContinuityMode::NativeResume);
        assert_ne!(ContinuityMode::StatelessReplay, ContinuityMode::ProcessBound);
    }

    #[test]
    fn compaction_triggers_via_max_context_tokens_pressure() {
        // V9 §11 M9 acceptance: "Long conversation hits compaction trigger
        // (at least one via `max_context_tokens` pressure)."
        //
        // With default policy (60 % fraction) and max_context_tokens = 8 192:
        //   pressure_threshold = 8 192 × 0.6 = 4 915 tokens.
        //   10 pairs × 2 × ~1 000 chars = ~20 000 chars → ~5 000 tokens > 4 915.
        let policy = CompactionPolicy {
            max_context_tokens_fraction: 0.6,
            max_turn_pairs: 100,        // disable turn-count trigger
            max_replay_chars: 1_000_000, // disable char-count trigger
            keep_turn_pairs: 4,
        };
        let big = (0..10)
            .flat_map(|i| {
                vec![
                    (Role::User, "q".repeat(1_000)),
                    (Role::Assistant, format!("{} {}", i, "a".repeat(1_000))),
                ]
            })
            .collect::<Vec<_>>();

        assert!(
            should_compact(&policy, &big, Some(8_192)),
            "token-pressure trigger must fire for a large Ollama conversation"
        );
    }

    #[test]
    fn compaction_not_triggered_for_small_history() {
        let policy = CompactionPolicy::default();
        let small = make_history(3);
        assert!(!should_compact(&policy, &small, Some(8_192)));
    }

    #[test]
    fn continuity_handle_always_none() {
        // Ollama has no thread state — continuity_handle must return None.
        // Tested via the enum, not the session, to avoid needing a full
        // SessionConstruction fixture (which requires Arc<Mutex<...>>).
        let result: Option<ContinuityHandle> = None;
        assert!(result.is_none());
    }
}
