//! Replay-history compaction (V9 §11 M9).
//!
//! Provides trigger detection and inline compaction for `StatelessReplay`
//! providers (Ollama in M9, any future provider of the same mode). Compaction
//! is applied by the provider session (`OllamaSession`) before forwarding a
//! `Turn` to the backend, bounding what the model sees without requiring
//! the caller to trim the `SessionLedger` directly.
//!
//! **Design constraints (V9 §0 rule 5):** no prompt strings are produced
//! here. `compact_replay` drops entries and returns a `CompactionRecord`
//! with a summary *description*; the provider session decides how (and
//! whether) to surface that description to the model.
//!
//! **Trigger precedence:** any trigger fires compaction; callers may pass
//! `None` for `max_context_tokens` to disable the token-pressure trigger
//! (e.g., for providers where context size is unknown).

use crate::context_planner::ledger::{CompactionRecord, Role};

// ── CompactionPolicy ─────────────────────────────────────────────────────────

/// Thresholds that control when and how inline replay compaction fires.
///
/// Applied per turn by `OllamaSession` (M9) and any future `StatelessReplay`
/// provider that accumulates unbounded history. Values are intentionally
/// conservative defaults; callers may construct a custom policy.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    /// Maximum number of `(user, assistant)` turn pairs to keep before
    /// compaction fires. Checked against `replay.len() / 2`.
    pub max_turn_pairs: u32,
    /// Maximum total characters across all replay entries.
    /// Guards against very large individual messages.
    pub max_replay_chars: usize,
    /// Fraction of `ProviderProfile::max_context_tokens` at which the
    /// token-pressure trigger fires. `0.0` disables the trigger;
    /// `1.0` triggers only at the hard limit. V9 §11 M9 acceptance
    /// requires at least one test that exercises this trigger.
    pub max_context_tokens_fraction: f32,
    /// Number of recent `(user, assistant)` pairs to retain after compaction.
    /// Must be ≤ `max_turn_pairs`; the session logs a warning if violated.
    pub keep_turn_pairs: u32,
}

impl Default for CompactionPolicy {
    /// Conservative defaults suitable for typical Ollama deployments.
    ///
    /// * 20 turn-pair ceiling (enough for most conversations)
    /// * 80 000 char ceiling (≈ 20 000 tokens at 4 chars/token average)
    /// * Compact at 60 % of the provider's declared context window
    /// * Keep the 8 most recent turn pairs after compaction
    fn default() -> Self {
        Self {
            max_turn_pairs: 20,
            max_replay_chars: 80_000,
            max_context_tokens_fraction: 0.6,
            keep_turn_pairs: 8,
        }
    }
}

impl CompactionPolicy {
    /// Convenience constructor for tests or custom sessions.
    pub fn custom(max_turn_pairs: u32, max_replay_chars: usize, keep_turn_pairs: u32) -> Self {
        Self {
            max_turn_pairs,
            max_replay_chars,
            keep_turn_pairs,
            ..Default::default()
        }
    }
}

// ── Trigger detection ────────────────────────────────────────────────────────

/// Returns `true` iff any compaction trigger fires for the given replay slice.
///
/// Three independent triggers (V9 §11 M9 required outputs):
/// 1. **Turn-pair count** — `replay.len() / 2 >= policy.max_turn_pairs`.
/// 2. **Total-char count** — sum of all entry lengths >= `policy.max_replay_chars`.
/// 3. **Token-pressure** — estimated token count >= fraction × `max_context_tokens`
///    (V9 §11 M9 acceptance: "at least one trigger via `max_context_tokens` pressure").
///    Token estimate: 4 chars per token (BPE average). Disabled when
///    `max_context_tokens` is `None`.
pub fn should_compact(
    policy: &CompactionPolicy,
    replay: &[(Role, String)],
    max_context_tokens: Option<usize>,
) -> bool {
    // Trigger 1: turn-pair count.
    if replay.len() / 2 >= policy.max_turn_pairs as usize {
        tracing::debug!(
            target: "compaction",
            pair_count = replay.len() / 2,
            threshold = policy.max_turn_pairs,
            "compaction trigger: turn_pair_count"
        );
        return true;
    }

    // Trigger 2: total chars.
    let total_chars: usize = replay.iter().map(|(_, c)| c.len()).sum();
    if total_chars >= policy.max_replay_chars {
        tracing::debug!(
            target: "compaction",
            total_chars,
            threshold = policy.max_replay_chars,
            "compaction trigger: max_replay_chars"
        );
        return true;
    }

    // Trigger 3: max_context_tokens pressure.
    if let Some(max_tok) =
        max_context_tokens.filter(|&t| t > 0 && policy.max_context_tokens_fraction > 0.0)
    {
        // Rough BPE token estimate: 4 chars per token.
        let estimated_tokens = total_chars / 4;
        let pressure_threshold = (max_tok as f32 * policy.max_context_tokens_fraction) as usize;
        if estimated_tokens >= pressure_threshold {
            tracing::debug!(
                target: "compaction",
                estimated_tokens,
                pressure_threshold,
                max_tok,
                "compaction trigger: max_context_tokens_pressure"
            );
            return true;
        }
    }

    false
}

// ── Apply compaction ─────────────────────────────────────────────────────────

/// Drop old replay entries, retaining the `policy.keep_turn_pairs` most recent
/// `(user, assistant)` pairs.
///
/// Returns `(compacted_replay, record)` where `record` describes what was
/// dropped. The caller may store `record` in `SessionLedger::compacted_summary`
/// (M9 wires this on the TUI side via event).
///
/// **Invariant:** if `replay.len() <= keep_turn_pairs * 2`, no entries are
/// dropped and `record.turns_compacted == 0`.
pub fn compact_replay(
    policy: &CompactionPolicy,
    replay: Vec<(Role, String)>,
) -> (Vec<(Role, String)>, CompactionRecord) {
    let keep = (policy.keep_turn_pairs as usize) * 2;
    let total = replay.len();
    let dropped_entries = total.saturating_sub(keep);
    let dropped_pairs = dropped_entries / 2;

    let summary = if dropped_pairs > 0 {
        format!(
            "[{} older turn{} compacted to fit context limit]",
            dropped_pairs,
            if dropped_pairs == 1 { "" } else { "s" },
        )
    } else {
        "[context compacted — no entries dropped]".to_string()
    };

    let remaining: Vec<(Role, String)> = if total > keep {
        replay[total - keep..].to_vec()
    } else {
        replay
    };

    let record = CompactionRecord {
        turns_compacted: dropped_pairs as u32,
        summary,
        created_at: Some(std::time::SystemTime::now()),
    };
    (remaining, record)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_history(pairs: usize) -> Vec<(Role, String)> {
        (0..pairs)
            .flat_map(|i| {
                vec![
                    (Role::User, format!("question {}", i)),
                    (Role::Assistant, format!("answer {}", i)),
                ]
            })
            .collect()
    }

    // ── should_compact ────────────────────────────────────────────────────────

    #[test]
    fn trigger_turn_pair_count_at_threshold() {
        let policy = CompactionPolicy {
            max_turn_pairs: 5,
            ..Default::default()
        };
        let history = make_history(5); // exactly at threshold
        assert!(should_compact(&policy, &history, None));
    }

    #[test]
    fn no_trigger_below_all_thresholds() {
        let policy = CompactionPolicy::default();
        let history = make_history(3);
        assert!(!should_compact(&policy, &history, None));
    }

    #[test]
    fn trigger_total_chars() {
        let policy = CompactionPolicy {
            max_replay_chars: 50,
            ..Default::default()
        };
        // Two entries of 30 chars each → 60 total > 50.
        let history = vec![
            (Role::User, "a".repeat(30)),
            (Role::Assistant, "b".repeat(30)),
        ];
        assert!(should_compact(&policy, &history, None));
    }

    #[test]
    fn trigger_max_context_tokens_pressure() {
        // V9 §11 M9 acceptance: "at least one trigger via max_context_tokens
        // pressure". This is the canonical test that exercises trigger 3.
        let policy = CompactionPolicy {
            max_context_tokens_fraction: 0.6,
            max_turn_pairs: 100, // disable other triggers
            max_replay_chars: 1_000_000,
            ..Default::default()
        };

        // 10 pairs × 2 × ~1 000 chars = ~20 000 chars.
        // Estimated tokens: 20 000 / 4 = 5 000.
        // Pressure threshold: 8 192 × 0.6 = 4 915.
        // 5 000 ≥ 4 915 → fires.
        let big: Vec<(Role, String)> = (0..10)
            .flat_map(|i| {
                vec![
                    (Role::User, "q".repeat(1_000)),
                    (Role::Assistant, format!("{} {}", i, "a".repeat(1_000))),
                ]
            })
            .collect();
        assert!(
            should_compact(&policy, &big, Some(8_192)),
            "expected token-pressure trigger to fire"
        );
    }

    #[test]
    fn no_pressure_trigger_when_max_context_tokens_is_none() {
        let policy = CompactionPolicy {
            max_turn_pairs: 100,
            max_replay_chars: 1_000_000,
            ..Default::default()
        };
        // Same big history, but no max_context_tokens → no trigger.
        let big: Vec<(Role, String)> = (0..10)
            .flat_map(|_| {
                vec![
                    (Role::User, "q".repeat(1_000)),
                    (Role::Assistant, "a".repeat(1_000)),
                ]
            })
            .collect();
        assert!(!should_compact(&policy, &big, None));
    }

    // ── compact_replay ────────────────────────────────────────────────────────

    #[test]
    fn compact_keeps_recent_pairs() {
        let policy = CompactionPolicy {
            keep_turn_pairs: 2,
            ..Default::default()
        };
        let history = make_history(5);
        // 5 pairs × 2 = 10 entries. keep = 2 × 2 = 4.
        // Retained: indices 6..10 → pairs 3 and 4.
        let (compacted, record) = compact_replay(&policy, history);
        assert_eq!(compacted.len(), 4, "should retain 2 turn pairs = 4 entries");
        assert_eq!(record.turns_compacted, 3, "dropped 3 oldest pairs");
        assert_eq!(compacted[0], (Role::User, "question 3".to_string()));
        assert_eq!(compacted[1], (Role::Assistant, "answer 3".to_string()));
        assert_eq!(compacted[2], (Role::User, "question 4".to_string()));
        assert_eq!(compacted[3], (Role::Assistant, "answer 4".to_string()));
    }

    #[test]
    fn compact_no_drop_when_under_keep() {
        let policy = CompactionPolicy {
            keep_turn_pairs: 10,
            ..Default::default()
        };
        let history = make_history(3);
        let len = history.len();
        let (compacted, record) = compact_replay(&policy, history);
        assert_eq!(compacted.len(), len, "nothing should be dropped");
        assert_eq!(record.turns_compacted, 0);
    }

    #[test]
    fn compact_record_has_created_at() {
        let policy = CompactionPolicy {
            keep_turn_pairs: 1,
            ..Default::default()
        };
        let history = make_history(3);
        let (_, record) = compact_replay(&policy, history);
        assert!(
            record.created_at.is_some(),
            "CompactionRecord must have a timestamp"
        );
    }

    #[test]
    fn compact_summary_text_pluralises_correctly() {
        let policy = CompactionPolicy {
            keep_turn_pairs: 2,
            ..Default::default()
        };
        // 3 pairs in, 2 kept → 1 dropped → "1 older turn"
        let (_, record) = compact_replay(&policy, make_history(3));
        assert!(
            record.summary.contains("1 older turn compacted"),
            "singular form expected: {}",
            record.summary
        );

        // 5 pairs in, 2 kept → 3 dropped → "3 older turns"
        let (_, record2) = compact_replay(&policy, make_history(5));
        assert!(
            record2.summary.contains("3 older turns compacted"),
            "plural form expected: {}",
            record2.summary
        );
    }
}
