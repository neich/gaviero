//! Tier B / B5: per-session consolidator.
//!
//! Runs at session close (or 90-second idle, or explicit
//! `/consolidate-session`). Pulls the session's transcript + recent
//! `TurnComplete`-extracted memories, asks a medium-tier LLM to emit
//! ADD / MERGE / SUPERSEDE / DROP operations + a session summary,
//! parses the response, and applies the operations through the
//! Tier S2 writer task. **The LLM proposes; the writer applies.**
//!
//! The prompt is version-pinned (see `PROMPT_V1`); future revisions
//! bump the version so the audit trail can identify which rubric
//! produced a given operation.

use anyhow::Result;
use serde::Deserialize;

/// Pinned consolidator prompt. Revisions bump the version suffix and
/// must keep the JSON output schema stable; downstream parsers key on
/// the field names below.
pub const PROMPT_VERSION: &str = "session_v1";

/// Verbatim consolidator prompt. Kept as a single &'static so the
/// prompt and its version travel together.
pub const PROMPT_V1: &str = r#"
You are Gaviero's session consolidator. Read the chat transcript and the
list of CANDIDATE memories that were extracted from it. For each candidate,
decide whether to ADD it as-is, MERGE it into a similar existing memory,
SUPERSEDE an obsolete memory with it, or DROP it (low value / duplicate).

Also produce one SHORT session summary (≤400 tokens) capturing the thread
of the conversation. The summary is stored as a long-lived memory at the
session's working scope.

Reply with ONE JSON object:

{
  "session_summary": "...",
  "operations": [
    {"op": "ADD", "candidate_index": <int>},
    {"op": "MERGE", "candidate_index": <int>, "into_memory_id": <int>},
    {"op": "SUPERSEDE", "candidate_index": <int>, "supersedes_memory_id": <int>},
    {"op": "DROP", "candidate_index": <int>, "reason": "..."}
  ],
  "promotions": [
    {"memory_id": <int>, "to_scope": "module"|"repo"|"workspace"|"global"}
  ]
}

Rules:
- `candidate_index` is the 0-based index into the CANDIDATES list below.
- Never invent memory ids; only use ids you can see in EXISTING_MEMORIES.
- Prefer DROP over MERGE if uncertain.
- Promotions are optional; only include rows you actively want widened.
"#;

/// One discrete operation emitted by the consolidator. `candidate_index`
/// always refers to the CANDIDATES list passed to the prompt; the
/// writer task resolves indices to concrete writes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "UPPERCASE")]
pub enum ConsolidationOp {
    Add {
        candidate_index: usize,
    },
    Merge {
        candidate_index: usize,
        into_memory_id: i64,
    },
    Supersede {
        candidate_index: usize,
        supersedes_memory_id: i64,
    },
    Drop {
        candidate_index: usize,
        #[serde(default)]
        reason: String,
    },
}

/// Cross-scope promotion request from the consolidator.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PromotionRequest {
    pub memory_id: i64,
    pub to_scope: String, // "module" | "repo" | "workspace" | "global"
}

/// Parsed consolidator response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidatorResponse {
    #[serde(default)]
    pub session_summary: String,
    #[serde(default)]
    pub operations: Vec<ConsolidationOp>,
    #[serde(default)]
    pub promotions: Vec<PromotionRequest>,
}

/// Tolerantly extract a `{ ... }` JSON object from an LLM response that
/// may be wrapped in prose / fenced code blocks. Mirrors the strategy
/// used by [`super::extractor::parse_response`] so the consolidator
/// shares the same robustness model.
fn extract_json_object(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(raw[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a consolidator LLM response. Tolerates prose-wrapped /
/// fence-wrapped JSON. Empty or malformed responses parse as
/// `ConsolidatorResponse::default()` *plus* an `Err` so callers can
/// log the failure without losing user transcripts.
pub fn parse_response(raw: &str) -> Result<ConsolidatorResponse> {
    let body = extract_json_object(raw)
        .ok_or_else(|| anyhow::anyhow!("consolidator: no JSON object in response"))?;
    let parsed: ConsolidatorResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("consolidator: parse error: {e}"))?;
    Ok(parsed)
}

/// Build the prompt body for one session: prompt template + transcript
/// + candidate dump + (optionally) related-existing-memory dump.
pub fn build_prompt(
    transcript: &str,
    candidates: &[CandidateBrief],
    existing: &[ExistingBrief],
) -> String {
    let mut body = String::with_capacity(transcript.len() + 1024);
    body.push_str(PROMPT_V1.trim_start());
    body.push_str("\n\nTRANSCRIPT (truncated to last N turns):\n");
    body.push_str(transcript);
    body.push_str("\n\nCANDIDATES (extracted this session):\n");
    for (i, c) in candidates.iter().enumerate() {
        body.push_str(&format!(
            "[{i}] type={} importance={:.2} | {}\n",
            c.kind, c.importance, c.text
        ));
    }
    body.push_str("\nEXISTING_MEMORIES (similar at workspace/repo scope):\n");
    for e in existing {
        body.push_str(&format!(
            "id={} scope={} type={} | {}\n",
            e.id, e.scope_label, e.kind, e.text
        ));
    }
    body
}

/// Lightweight projection of an extracted memory for the prompt.
#[derive(Debug, Clone)]
pub struct CandidateBrief {
    pub text: String,
    pub kind: String,
    pub importance: f32,
}

/// Lightweight projection of an existing memory for the prompt.
#[derive(Debug, Clone)]
pub struct ExistingBrief {
    pub id: i64,
    pub text: String,
    pub kind: String,
    pub scope_label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_handles_well_formed_json() {
        let raw = r#"{
            "session_summary": "we picked tokio over std mutex",
            "operations": [
                {"op": "ADD", "candidate_index": 0},
                {"op": "MERGE", "candidate_index": 1, "into_memory_id": 42},
                {"op": "SUPERSEDE", "candidate_index": 2, "supersedes_memory_id": 17},
                {"op": "DROP", "candidate_index": 3, "reason": "duplicate"}
            ],
            "promotions": [{"memory_id": 8, "to_scope": "repo"}]
        }"#;
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.session_summary, "we picked tokio over std mutex");
        assert_eq!(parsed.operations.len(), 4);
        assert!(matches!(parsed.operations[0], ConsolidationOp::Add { .. }));
        assert!(matches!(
            parsed.operations[1],
            ConsolidationOp::Merge {
                into_memory_id: 42,
                ..
            }
        ));
        assert!(matches!(
            parsed.operations[2],
            ConsolidationOp::Supersede {
                supersedes_memory_id: 17,
                ..
            }
        ));
        assert!(matches!(parsed.operations[3], ConsolidationOp::Drop { .. }));
        assert_eq!(parsed.promotions.len(), 1);
    }

    #[test]
    fn parse_response_handles_prose_wrap_and_fence() {
        let raw = r#"Sure, here:
```json
{"session_summary": "ok", "operations": [], "promotions": []}
```
Hope that helps."#;
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.session_summary, "ok");
        assert!(parsed.operations.is_empty());
    }

    #[test]
    fn parse_response_errors_on_no_json() {
        let err = parse_response("totally not JSON").unwrap_err();
        assert!(err.to_string().contains("no JSON"));
    }

    #[test]
    fn parse_response_tolerates_missing_optional_fields() {
        let raw = r#"{"operations": [{"op": "ADD", "candidate_index": 0}]}"#;
        let parsed = parse_response(raw).unwrap();
        assert!(parsed.session_summary.is_empty());
        assert_eq!(parsed.operations.len(), 1);
        assert!(parsed.promotions.is_empty());
    }

    #[test]
    fn build_prompt_includes_candidate_indices() {
        let body = build_prompt(
            "user: hi\nassistant: hello",
            &[CandidateBrief {
                text: "use tokio".into(),
                kind: "decision".into(),
                importance: 0.9,
            }],
            &[ExistingBrief {
                id: 7,
                text: "use std::sync::Mutex".into(),
                kind: "decision".into(),
                scope_label: "repo".into(),
            }],
        );
        assert!(body.contains("[0] type=decision"));
        assert!(body.contains("id=7 scope=repo"));
        assert!(body.contains("session_summary"));
    }
}
