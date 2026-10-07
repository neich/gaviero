//! Cross-turn replay for the in-process tool-agent (DeepSeek plan Unit 14).
//!
//! Builds the OpenAI-compatible `messages` array from prior turns
//! (`Turn.replay_history`) plus the current user prompt. Compaction is applied
//! at the session boundary before messages are assembled (mirrors
//! [`super::super::ollama::OllamaSession`]).

use serde_json::{Value, json};

use crate::context_planner::ReplayPayload;
use crate::context_planner::compaction::CompactionPolicy;
use crate::context_planner::ledger::Role;

use crate::agent_session::Turn;
use crate::agent_session::replay_compaction::compact_turn_replay;

/// Apply replay compaction to `turn` when any policy threshold is exceeded.
pub(crate) fn apply_replay_compaction(
    turn: &mut Turn,
    policy: &CompactionPolicy,
    max_context_tokens: Option<usize>,
) {
    compact_turn_replay(turn, policy, max_context_tokens, "deepseek");
}

/// Assemble the initial message array for one API turn.
///
/// Replayed assistant turns carry `"reasoning_content": ""`. The ledger keeps
/// text only, so the real chain of thought of an earlier turn is gone — but
/// DeepSeek's thinking-mode guide requires the field on the assistant turns of
/// *every* tools-bearing request, "even for turns where the model did not
/// perform a tool call", and answers its absence with a 400. An empty string is
/// the documented `string | null` shape, and is what rig's DeepSeek dialect
/// sends for the same reason.
///
/// `user_content` is the current user message's `content`: a string, or the
/// `[text, image_url…]` part array [`super::attachments::user_content`] builds.
/// Images only ever ride the current user message — replay stays text.
pub(crate) fn build_messages(
    system: &str,
    replay: Option<&ReplayPayload>,
    user_content: impl Into<Value>,
) -> Vec<Value> {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    if let Some(payload) = replay {
        for (role, content) in &payload.entries {
            messages.push(match role {
                Role::User => json!({ "role": "user", "content": content }),
                Role::Assistant => json!({
                    "role": "assistant",
                    "content": content,
                    "reasoning_content": "",
                }),
                Role::System => json!({ "role": "system", "content": content }),
            });
        }
    }
    messages.push(json!({ "role": "user", "content": user_content.into() }));
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
    fn replayed_assistant_turns_carry_reasoning_content() {
        let replay = ReplayPayload {
            entries: vec![
                (Role::User, "old q".into()),
                (Role::Assistant, "old a".into()),
            ],
        };
        let msgs = build_messages("sys", Some(&replay), "new q");
        assert_eq!(msgs[2].get("reasoning_content"), Some(&json!("")));
        assert!(msgs[1].get("reasoning_content").is_none());
        assert!(msgs[3].get("reasoning_content").is_none());
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
