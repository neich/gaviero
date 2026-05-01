//! Default scripted sessions used by T1.5 (and reusable for T1.6).
//!
//! Three heterogeneous scripts on purpose — three copies of the same
//! script catch fewer bugs.
//!
//! Each script's prompts include a per-session marker so the test can
//! assert cross-session memory write isolation: a memory written by S0
//! (containing `EMBED_MARKER_S0_<uuid>`) must not surface for S1 / S2.

use std::time::Duration;

use super::orchestrator::{ScriptedSession, Step};

/// One barrier id reused by the default scripts. T1.5 places this
/// between turn 1 and turn 2 of every session to maximise concurrency
/// on the embedder.
pub const SHARED_BARRIER_AFTER_TURN_1: u32 = 1;

/// 6 turns including reset@4 with t5 == t1 verbatim.
pub fn refactor_session(id: &str, marker: &str) -> ScriptedSession {
    let t1 = format!(
        "Read crates/gaviero-core/src/memory/retrieval.rs and summarise the composite scoring formula. Tag your reply with marker {marker}."
    );
    ScriptedSession {
        id: id.to_string(),
        steps: vec![
            Step::User(t1.clone()),
            Step::Barrier(SHARED_BARRIER_AFTER_TURN_1),
            Step::User("List every callsite of retrieve_for_chat.".into()),
            Step::User(
                "Propose a refactor extracting the rerank blend into a helper.".into(),
            ),
            Step::Reset,
            Step::User(t1),
            Step::User("Summarise once more in two bullet points.".into()),
        ],
    }
}

/// 5 turns, reset@3, topic switch.
pub fn bugfix_session(id: &str, marker: &str) -> ScriptedSession {
    ScriptedSession {
        id: id.to_string(),
        steps: vec![
            Step::User(format!(
                "Inspect ARGV_THRESHOLD usage in crates/gaviero-core/src/acp/session.rs. Tag your reply with marker {marker}."
            )),
            Step::Barrier(SHARED_BARRIER_AFTER_TURN_1),
            Step::User(
                "What happens when prompt + system_prompt straddle the threshold?".into(),
            ),
            Step::Reset,
            Step::User(
                "Switching topics. Where does the writer task live and how is it shut down?"
                    .into(),
            ),
            Step::Sleep(Duration::from_millis(50)),
            Step::User("Briefly: how does TurnComplete enqueue propagate?".into()),
        ],
    }
}

/// 8 turns, no reset.
pub fn feature_session(id: &str, marker: &str) -> ScriptedSession {
    ScriptedSession {
        id: id.to_string(),
        steps: vec![
            Step::User(format!(
                "List the public modules of gaviero-core. Tag your reply with marker {marker}."
            )),
            Step::Barrier(SHARED_BARRIER_AFTER_TURN_1),
            Step::User("How many tree-sitter grammars does gaviero-core depend on?".into()),
            Step::User("What's the canonical model spec format?".into()),
            Step::User("Where is settings cascade documented?".into()),
            Step::User("Sketch a feature flag for an opt-in retrieval mode.".into()),
            Step::User("What would the test plan look like?".into()),
            Step::User("Finally: what would the rollback strategy be?".into()),
        ],
    }
}
