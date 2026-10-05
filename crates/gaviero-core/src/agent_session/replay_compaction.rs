//! Bounded host replay for every session that replays the chat itself.
//!
//! Chat builds a fresh provider session per turn, and for `codex:`, `dsh:`,
//! `deepseek:` and `ollama:` the visible transcript is the only continuity:
//! the planner inlines all of it on every turn (`SessionLedger::turn_count`
//! only advances on a Claude/Cursor `system/init`). Without a bound a long
//! chat resends its whole history each turn. Each such session calls
//! [`compact_turn_replay`] before rendering, with the shared
//! [`CompactionPolicy`] — Ollama and DeepSeek did so already; Codex and dsh
//! did not.

use crate::context_planner::ReplayPayload;
use crate::context_planner::compaction::{CompactionPolicy, compact_replay, should_compact};
use crate::context_planner::ledger::Role;

use super::Turn;

/// Drop the oldest replay entries when any [`CompactionPolicy`] trigger
/// fires, keeping the most recent turn pairs. The first kept user entry is
/// prefixed with what was dropped, so the model knows the replay is not the
/// whole conversation (a note in that entry, not a `system` message, so it
/// renders on every provider — DeepSeek's message array included). Returns
/// how many turn pairs were dropped.
pub(crate) fn compact_turn_replay(
    turn: &mut Turn,
    policy: &CompactionPolicy,
    max_context_tokens: Option<usize>,
    provider: &str,
) -> u32 {
    let Some(payload) = turn.replay_history.as_ref() else {
        return 0;
    };
    if !should_compact(policy, &payload.entries, max_context_tokens) {
        return 0;
    }
    let (mut entries, record) = compact_replay(policy, payload.entries.clone());
    if record.turns_compacted > 0
        && let Some((_, first_user)) = entries.iter_mut().find(|(role, _)| *role == Role::User)
    {
        *first_user = format!("{}\n\n{first_user}", record.summary);
    }
    tracing::info!(
        target: "turn_metrics",
        provider,
        turns_compacted = record.turns_compacted,
        kept_entries = entries.len(),
        max_context_tokens = ?max_context_tokens,
        "replay_compacted"
    );
    turn.replay_history = Some(ReplayPayload { entries }).filter(|p| !p.entries.is_empty());
    record.turns_compacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::PlannerMetadata;

    fn turn_with_pairs(pairs: usize) -> Turn {
        let mut entries = Vec::new();
        for i in 0..pairs {
            entries.push((Role::User, format!("question {i}")));
            entries.push((Role::Assistant, format!("answer {i}")));
        }
        Turn {
            user_message: "next".into(),
            memory_selections: vec![],
            graph_selections: vec![],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: Some(ReplayPayload { entries }),
            effort: None,
            auto_approve: false,
            metadata: PlannerMetadata::default(),
        }
    }

    #[test]
    fn a_long_replay_keeps_the_recent_pairs_and_says_what_was_dropped() {
        let mut turn = turn_with_pairs(25);
        let dropped = compact_turn_replay(&mut turn, &CompactionPolicy::default(), None, "codex");
        assert_eq!(dropped, 17, "25 pairs, keep 8");
        let entries = &turn.replay_history.as_ref().unwrap().entries;
        assert_eq!(entries.len(), 16);
        assert_eq!(entries[0].0, Role::User);
        assert!(
            entries[0]
                .1
                .starts_with("[17 older turns compacted to fit context limit]\n\nquestion 17"),
            "{}",
            entries[0].1
        );
        assert_eq!(entries[15].1, "answer 24");
    }

    #[test]
    fn a_short_replay_is_left_alone() {
        let mut turn = turn_with_pairs(5);
        assert_eq!(
            compact_turn_replay(&mut turn, &CompactionPolicy::default(), None, "dsh"),
            0
        );
        assert_eq!(turn.replay_history.unwrap().entries.len(), 10);
    }

    #[test]
    fn token_pressure_alone_compacts() {
        // 10 pairs of ~2,000 chars ≈ 10,000 tokens, over 60% of an 8k window.
        let mut turn = turn_with_pairs(0);
        let big = "x".repeat(2_000);
        turn.replay_history = Some(ReplayPayload {
            entries: (0..20)
                .map(|i| {
                    let role = if i % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    };
                    (role, big.clone())
                })
                .collect(),
        });
        let dropped =
            compact_turn_replay(&mut turn, &CompactionPolicy::default(), Some(8_192), "dsh");
        assert_eq!(dropped, 2);
    }
}
