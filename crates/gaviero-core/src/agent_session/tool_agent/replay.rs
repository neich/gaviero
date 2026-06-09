//! Cross-turn replay for the in-process tool-agent (DeepSeek plan Unit 14).
//!
//! Builds the OpenAI-compatible `messages` array from prior turns
//! (`Turn.replay_history`) plus the current user prompt. Compaction is applied
//! at the session boundary before messages are assembled (mirrors
//! [`super::super::ollama::OllamaSession`]).

use serde_json::{Value, json};

use crate::context_planner::compaction::{
    CompactionPolicy, compact_replay, should_compact,
};
use crate::context_planner::ledger::Role;
use crate::context_planner::ReplayPayload;

use crate::agent_session::Turn;

/// Apply replay compaction to `turn` when any policy threshold is exceeded.
pub(crate) fn apply_replay_compaction(
    turn: &mut Turn,
    policy: &CompactionPolicy,
    max_context_tokens: Option<usize>,
) {
    let Some(ref payload) = turn.replay_history else {
        return;
    };
    if !should_compact(policy, &payload.entries, max_context_tokens) {
        return;
    }
    let (compacted_entries, record) = compact_replay(policy, payload.entries.clone());
    tracing::info!(
        target: "turn_metrics",
        provider = "deepseek",
        turns_compacted = record.turns_compacted,
        kept_entries = compacted_entries.len(),
        max_context_tokens = ?max_context_tokens,
        "tool_agent_replay_compacted"
    );
    turn.replay_history = Some(ReplayPayload {
        entries: compacted_entries,
    })
    .filter(|p| !p.entries.is_empty());
}

/// Assemble the initial message array for one API turn.
pub(crate) fn build_messages(
    system: &str,
    replay: Option<&ReplayPayload>,
    user_prompt: &str,
) -> Vec<Value> {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    if let Some(payload) = replay {
        for (role, content) in &payload.entries {
            let role_str = match role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };
            messages.push(json!({ "role": role_str, "content": content }));
        }
    }
    messages.push(json!({ "role": "user", "content": user_prompt }));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_planner::compaction::CompactionPolicy;

    #[test]
    fn build_messages_interleaves_replay_before_current_user() {
        let replay = ReplayPayload {
            entries: vec![
                (Role::User, "old q".into()),
                (Role::Assistant, "old a".into()),
            ],
        };
        let msgs = build_messages("sys", Some(&replay), "new q");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["content"], "old q");
        assert_eq!(msgs[2]["content"], "old a");
        assert_eq!(msgs[3]["content"], "new q");
    }

    #[test]
    fn compaction_drops_oldest_pairs_under_pressure() {
        let policy = CompactionPolicy {
            max_context_tokens_fraction: 0.6,
            max_turn_pairs: 100,
            max_replay_chars: 1_000_000,
            keep_turn_pairs: 2,
        };
        let big: Vec<(Role, String)> = (0..10)
            .flat_map(|i| {
                vec![
                    (Role::User, "q".repeat(1_000)),
                    (Role::Assistant, format!("{i} {}", "a".repeat(1_000))),
                ]
            })
            .collect();
        let mut turn = Turn {
            user_message: "now".into(),
            memory_selections: vec![],
            graph_selections: vec![],
            file_refs: vec![],
            skill_selections: vec![],
            replay_history: Some(ReplayPayload { entries: big }),
            effort: None,
            auto_approve: false,
            metadata: Default::default(),
        };
        apply_replay_compaction(&mut turn, &policy, Some(8_192));
        let kept = turn.replay_history.as_ref().unwrap().entries.len();
        assert_eq!(kept, 4, "keep_turn_pairs=2 → 4 entries");
    }
}
