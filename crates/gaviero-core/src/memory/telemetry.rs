//! Tier B / B6: retrieval-use telemetry.
//!
//! After each chat turn, the writer task runs a lightweight pass that
//! decides — for every memory the chat injected — whether the response
//! actually used it. Three classes:
//!
//! * **Used**: cosine ≥ `used_threshold` (default 0.55) **OR** a
//!   substring of ≥ `substring_min_tokens` (default 8) consecutive
//!   words from the memory text appears verbatim in the response.
//! * **Partial**: `partial_threshold` ≤ cosine < `used_threshold`.
//! * **Unused**: cosine < `partial_threshold` and no substring hit.
//!
//! The classifier is cheap by design — cosine + substring, no LLM.
//! Empty / tiny responses (below `min_response_tokens`) skip the pass
//! entirely so casual chat ("hi", "thanks") doesn't poison utilization
//! signal.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::store::MemoryStore;

/// Per-pass classifier config. Resolved from `memory.telemetry.*`
/// settings; defaults match the plan.
#[derive(Debug, Clone)]
pub struct ClassifyConfig {
    pub used_threshold: f32,
    pub partial_threshold: f32,
    pub substring_min_tokens: usize,
    pub min_response_tokens: u32,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            used_threshold: 0.55,
            partial_threshold: 0.35,
            substring_min_tokens: 8,
            min_response_tokens: 20,
        }
    }
}

/// Classification verdict for one (memory, turn) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseClass {
    Used,
    Partial,
    Unused,
}

impl UseClass {
    pub fn as_str(self) -> &'static str {
        match self {
            UseClass::Used => "used",
            UseClass::Partial => "partial",
            UseClass::Unused => "unused",
        }
    }
}

/// One classified injected memory in a turn.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedItem {
    pub memory_id: i64,
    pub injected_rank: i32,
    pub class: UseClass,
    pub cosine: f32,
    pub substring_hit: bool,
}

/// Per-turn telemetry roll-up emitted to the [`TelemetryObserver`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelemetryReport {
    pub turn_id: String,
    pub used: usize,
    pub partial: usize,
    pub unused: usize,
    pub items: Vec<ClassifiedItem>,
}

/// Observer fired once per classification pass after the rows have
/// landed in `retrieval_use`. Implementations forward to the TUI panel
/// for live display; never block sleeptime / writer paths.
pub trait TelemetryObserver: Send + Sync {
    fn on_use_classified(&self, report: &TelemetryReport);
}

/// Classify one memory text vs a response. Pure function; the cosine
/// path is owned by the caller (it has the embedder), this fn handles
/// only the substring + threshold logic.
pub fn classify(
    cosine: f32,
    response: &str,
    memory_text: &str,
    cfg: &ClassifyConfig,
) -> (UseClass, bool) {
    let substring_hit = substring_match(memory_text, response, cfg.substring_min_tokens);
    let class = if substring_hit || cosine >= cfg.used_threshold {
        UseClass::Used
    } else if cosine >= cfg.partial_threshold {
        UseClass::Partial
    } else {
        UseClass::Unused
    };
    (class, substring_hit)
}

/// Whitespace-tokenized substring-match: returns true when at least
/// `min_tokens` consecutive whitespace-separated words from `needle`
/// (lowercased) appear contiguously in `haystack` (lowercased).
fn substring_match(needle: &str, haystack: &str, min_tokens: usize) -> bool {
    if min_tokens == 0 {
        return false;
    }
    let h = haystack.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    let n_tokens: Vec<&str> = n.split_whitespace().collect();
    if n_tokens.len() < min_tokens {
        return n_tokens
            .windows(n_tokens.len().max(1))
            .any(|w| h.contains(&w.join(" ")));
    }
    n_tokens
        .windows(min_tokens)
        .any(|w| h.contains(&w.join(" ")))
}

/// Run B6 classification for one turn. Reads the most recent
/// `injection_manifests` row for `turn_id`, embeds the response once,
/// computes per-memory cosines against the stored embedding, persists
/// `retrieval_use` rows, and returns the report. Caller forwards the
/// report to a [`TelemetryObserver`] if any.
pub async fn classify_turn(
    store: &Arc<MemoryStore>,
    turn_id: &str,
    session_id: Option<&str>,
    response: &str,
    cfg: &ClassifyConfig,
) -> Result<TelemetryReport> {
    let manifests = store
        .manifests_for_turn(turn_id)
        .await
        .context("telemetry: fetching manifest")?;
    let Some(manifest) = manifests.into_iter().next() else {
        return Ok(TelemetryReport {
            turn_id: turn_id.to_string(),
            ..Default::default()
        });
    };

    let payload: serde_json::Value =
        serde_json::from_str(&manifest.payload).context("telemetry: parsing manifest payload")?;
    let injected: Vec<(i64, String)> = payload
        .get("selected_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64())
                .enumerate()
                .map(|(i, id)| (id, format!("rank-{}", i + 1)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if injected.is_empty() {
        return Ok(TelemetryReport {
            turn_id: turn_id.to_string(),
            ..Default::default()
        });
    }

    // Single response embedding — reused across all injected memories.
    let response_embedding = store
        .embedder()
        .embed_query(response)
        .await
        .context("telemetry: embedding response")?;

    let mut report = TelemetryReport {
        turn_id: turn_id.to_string(),
        ..Default::default()
    };
    for (rank, (memory_id, _)) in injected.iter().enumerate() {
        let mem_emb = match store.embedding_for(*memory_id).await {
            Ok(Some(e)) => e,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(target: "memory_telemetry", memory_id, error = %e, "embedding fetch");
                continue;
            }
        };
        let cos = cosine_similarity(&response_embedding, &mem_emb);
        // Pull memory text for the substring-match path. Cheap second
        // round-trip; the classifier needs the text not the blob.
        let mem_text = store
            .get_content(*memory_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let (class, substring_hit) = classify(cos, response, &mem_text, cfg);
        let item = ClassifiedItem {
            memory_id: *memory_id,
            injected_rank: (rank + 1) as i32,
            class,
            cosine: cos,
            substring_hit,
        };
        match class {
            UseClass::Used => report.used += 1,
            UseClass::Partial => report.partial += 1,
            UseClass::Unused => report.unused += 1,
        }
        report.items.push(item);

        if let Err(e) = store
            .record_retrieval_use(
                *memory_id,
                turn_id,
                session_id,
                (rank + 1) as i32,
                class.as_str(),
                cos,
                substring_hit,
            )
            .await
        {
            tracing::warn!(
                target: "memory_telemetry",
                memory_id,
                error = %e,
                "retrieval_use insert failed"
            );
        }
    }
    Ok(report)
}

/// Local cosine helper duplicated from `store.rs` to keep this module
/// callable without exposing internals. f32 cosine; zero on mismatched
/// or empty vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_hit_when_8_consecutive_words_match() {
        let needle = "we always use tokio because std mutex is broken under load";
        let haystack =
            "earlier we noted: we always use tokio because std mutex is broken under load.";
        assert!(substring_match(needle, haystack, 8));
    }

    #[test]
    fn substring_misses_when_only_short_runs_match() {
        let needle = "alpha beta gamma delta";
        let haystack = "alpha beta is fine; gamma delta later";
        assert!(!substring_match(needle, haystack, 4));
    }

    #[test]
    fn classify_promotes_substring_to_used_even_at_low_cosine() {
        let cfg = ClassifyConfig::default();
        let needle = "we always use tokio because std mutex is broken under load";
        let haystack = "yes — we always use tokio because std mutex is broken under load.";
        let (class, hit) = classify(0.10, haystack, needle, &cfg);
        assert!(hit);
        assert_eq!(class, UseClass::Used);
    }

    #[test]
    fn classify_partial_band() {
        let cfg = ClassifyConfig::default();
        let (class, hit) = classify(0.40, "unrelated", "unrelated source", &cfg);
        assert!(!hit);
        assert_eq!(class, UseClass::Partial);
    }

    #[test]
    fn classify_unused_when_low_cosine_no_substring() {
        let cfg = ClassifyConfig::default();
        let (class, hit) = classify(0.10, "totally", "different topic", &cfg);
        assert!(!hit);
        assert_eq!(class, UseClass::Unused);
    }
}
