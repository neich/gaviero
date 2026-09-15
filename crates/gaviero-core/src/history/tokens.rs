//! Token estimation — the single estimator implementation in the tree.
//!
//! Gaviero has no tokenizer and (by decision, see the plan's q5) will not
//! grow one. Every per-item number the history log or the HISTORY panel
//! prints is therefore an *estimate*, and every estimate is labelled with
//! the estimator that produced it so the number can be audited later.
//!
//! Two heuristics exist, and they were previously duplicated:
//!
//! * `words × 13 / 10` — natural-language text (prompt, assistant output,
//!   memory block). Lived in `gaviero-tui`'s `panels/agent_chat.rs` as
//!   `words_to_tokens`; that function now delegates here.
//! * `chars / 4` — serialized JSON (tool args, tool results, MCP in/out).
//!   Also used by the context planner's bootstrap accounting.
//!
//! Exact totals are never estimated: they come from the provider
//! (`TokenUsage` on the Claude `result` event) and are labelled
//! `source: "provider"` on the record.

use serde::{Deserialize, Serialize};

/// Estimator identifier persisted on every record so a number can be
/// audited after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Estimator {
    /// `words × 13 / 10` over Unicode-whitespace-delimited words.
    WordsX13,
    /// `chars / 4` over the serialized JSON text.
    CharsDiv4,
}

impl Estimator {
    /// Stable wire / record identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WordsX13 => "words_x13",
            Self::CharsDiv4 => "chars_div4",
        }
    }

    /// Human label used by the panel footer legend. Kept here so the
    /// panel and the CLI agree on the spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::WordsX13 => "words×1.3",
            Self::CharsDiv4 => "chars÷4",
        }
    }
}

/// Count words by splitting on Unicode whitespace. Empty/whitespace-only
/// input yields 0.
pub fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Convert a word count to an approximate token count using the rule of
/// thumb that an LLM token is ~30% smaller than an English word:
/// `tokens ≈ words × 1.3`. Integer math: `(words × 13) / 10`.
pub fn words_to_tokens(words: usize) -> usize {
    words.saturating_mul(13) / 10
}

/// Estimate the token count of natural-language text (`words × 13 / 10`).
///
/// Used for the prompt, the assistant's output, and the rendered
/// `<project_memory>` block.
pub fn estimate_text_tokens(text: &str) -> usize {
    words_to_tokens(count_words(text))
}

/// Estimate the token count of a JSON value (`chars / 4` over its compact
/// serialization). Used for tool arguments, tool results, and MCP
/// request/response payloads.
pub fn estimate_json_tokens(v: &serde_json::Value) -> usize {
    estimate_json_text_tokens(&v.to_string())
}

/// `chars / 4` over an already-serialized JSON string. Avoids a second
/// serialization when the caller already holds the text.
pub fn estimate_json_text_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_estimate_matches_the_legacy_words_rule() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("   "), 0);
        assert_eq!(estimate_text_tokens("hello"), 1); // 1 word → 13/10 = 1
        assert_eq!(estimate_text_tokens("hello world"), 2); // 2 → 26/10 = 2
        // 7 words → 91/10 = 9
        assert_eq!(estimate_text_tokens("a b c d e f g"), 9);
    }

    #[test]
    fn words_helper_is_the_multiplication_only() {
        assert_eq!(words_to_tokens(0), 0);
        assert_eq!(words_to_tokens(10), 13);
        assert_eq!(words_to_tokens(100), 130);
        assert_eq!(words_to_tokens(7), 9);
    }

    #[test]
    fn json_estimate_is_chars_over_four() {
        // `{"a":1}` = 7 chars → ceil(7/4) = 2
        assert_eq!(estimate_json_tokens(&serde_json::json!({"a": 1})), 2);
        assert_eq!(estimate_json_tokens(&serde_json::json!(null)), 1);
        assert_eq!(estimate_json_text_tokens(""), 0);
        assert_eq!(estimate_json_text_tokens("abcd"), 1);
        assert_eq!(estimate_json_text_tokens("abcde"), 2);
    }

    #[test]
    fn estimator_identifiers_are_stable() {
        assert_eq!(Estimator::WordsX13.as_str(), "words_x13");
        assert_eq!(Estimator::CharsDiv4.as_str(), "chars_div4");
        assert_eq!(
            serde_json::to_string(&Estimator::WordsX13).unwrap(),
            "\"words_x13\""
        );
    }
}
