//! Tier B / B5: per-session consolidator.
//!
//! Runs at session close (or 90-second idle, or explicit
//! `/consolidate-session`). Pulls the session's transcript + recent
//! `TurnComplete`-extracted memories, asks a medium-tier LLM to emit
//! ADD / MERGE / SUPERSEDE / DROP operations + a session summary,
//! parses the response, and applies the operations through the
//! Tier S2 writer task. **The LLM proposes; the writer applies.**
//!
//! The prompt is version-pinned (see `PROMPT_V5`); future revisions
//! bump the version so the audit trail can identify which rubric
//! produced a given operation. Every applied operation records its
//! prompt version, so a revision can be judged against the op quality
//! it actually caused.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Pinned consolidator prompt. Revisions bump the version suffix and
/// must keep the JSON output schema stable; downstream parsers key on
/// the field names below.
///
/// `session_v3` added the verbatim-ADD rule. The first live run of
/// `session_v2` emitted six ADDs of which five stored raw `USER:` /
/// `ASSISTANT:` transcript blobs into the durable repo record at
/// importance 1.0 — the model read "ADD it as-is" as an instruction to
/// keep whatever it was shown. The candidate pool no longer contains
/// transcript rows at all (see `prepare_session_consolidate`), and the
/// rule is the second line of defence for any other bulky candidate.
///
/// `session_v4` replaces "Prefer DROP over MERGE if uncertain" with an
/// operation-selection rule keyed on what the candidate *does* to
/// existing knowledge. The old wording routed reconfirmations —
/// candidates restating a memory already held — to DROP, which discards
/// the very signal `MERGE` exists to record (it raises the surviving
/// row's trust to `MERGE_TRUST`). Across five live runs the consolidator
/// emitted zero MERGE and zero SUPERSEDE while repeatedly seeing
/// candidates identical to stored rows.
///
/// `session_v5` strips `promotions` from the prompt. The parser still
/// accepts a leftover `promotions` array (`#[serde(default)]`); apply
/// records each as `kind=promotion` `applied=0` and writes nothing.
/// Real promotion stays on `Consolidator::consolidate_run` /
/// `promote_frequent_cross_scope`.
pub const PROMPT_VERSION: &str = "session_v5";

/// Verbatim consolidator prompt. Kept as a single &'static so the
/// prompt and its version travel together.
pub const PROMPT_V5: &str = r#"
You are Gaviero's session consolidator. Read the chat transcript and the
list of CANDIDATE memories that were extracted from it. For each candidate,
decide whether to ADD it as-is, MERGE it into a similar existing memory,
SUPERSEDE an obsolete memory with it, or DROP it (not worth storing).

Also produce one SHORT session summary (≤400 tokens) capturing the thread
of the conversation. The summary is stored as a long-lived memory at the
session's working scope.

Reply with ONE JSON object:

{
  "session_summary": "...",
  "operations": [
    {"op": "ADD", "candidate_index": <int>, "expected_outcome": "..."},
    {"op": "MERGE", "candidate_index": <int>, "into_memory_id": <int>, "expected_outcome": "..."},
    {"op": "SUPERSEDE", "candidate_index": <int>, "supersedes_memory_id": <int>, "expected_outcome": "..."},
    {"op": "DROP", "candidate_index": <int>, "reason": "...", "expected_outcome": "..."}
  ]
}

Rules:
- `candidate_index` is the 0-based index into the CANDIDATES list below.
- Never invent memory ids. `into_memory_id` and `supersedes_memory_id`
  MUST be ids listed in EXISTING_MEMORIES below. An operation naming any
  other id is rejected and recorded as a failure.
- ADD stores the candidate's text **verbatim** as a long-lived memory —
  it does not rewrite or shorten it. So only ADD a candidate that already
  reads as a standalone durable fact. DROP anything that reads like raw
  conversation, a narrated work log, or a restatement of the transcript:
  the session summary already covers that ground, and a verbatim
  conversation excerpt in the durable store is worse than nothing.
- Choose the operation by what the candidate does to what is already
  known, not by how confident you feel:
  * It says the same thing as a memory in EXISTING_MEMORIES → MERGE into
    that id. This is a *reconfirmation*: the fact came up again and still
    holds, and merging raises that memory's trust. Do not DROP it — that
    silently discards the evidence.
  * It contradicts, corrects, or supersedes a memory in
    EXISTING_MEMORIES → SUPERSEDE that id.
  * It is new and worth keeping → ADD.
  * It is genuinely low-value, or a restatement of something already in
    this same batch → DROP.
- DROP is for candidates not worth storing at all. It is not the safe
  default for "already known" — MERGE is.
- `expected_outcome` is one short sentence saying what should be true of
  the memory store after the operation lands, e.g. "the older tokio
  decision is replaced by the mutex one". It is optional but strongly
  preferred: it is recorded next to the operation and read back to you
  in CONSOLIDATION_HISTORY on later runs, so it is how you find out
  whether your own reasoning held up.
- If CONSOLIDATION_HISTORY shows a previous run was rolled back, treat
  the operations in it as rejected by the user and do not repeat them.
"#;

/// One discrete operation emitted by the consolidator. `candidate_index`
/// always refers to the CANDIDATES list passed to the prompt; the
/// writer task resolves indices to concrete writes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

impl ConsolidationOp {
    /// Short, stable name for the audit row's `kind` column.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Merge { .. } => "merge",
            Self::Supersede { .. } => "supersede",
            Self::Drop { .. } => "drop",
        }
    }

    /// The candidate this operation refers to.
    pub fn candidate_index(&self) -> usize {
        match self {
            Self::Add { candidate_index }
            | Self::Merge {
                candidate_index, ..
            }
            | Self::Supersede {
                candidate_index, ..
            }
            | Self::Drop {
                candidate_index, ..
            } => *candidate_index,
        }
    }
}

/// Cross-scope promotion request from the consolidator.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PromotionRequest {
    pub memory_id: i64,
    pub to_scope: String, // "module" | "repo" | "workspace" | "global"
}

/// One operation plus the consolidator's own prediction about it.
///
/// `expected_outcome` is metadata *about* the operation rather than
/// part of it, so it rides alongside rather than inside
/// [`ConsolidationOp`] — which keeps the op enum, and every match on
/// it, unchanged. It is optional and defaults to `None`, so a model
/// that ignores the field still produces a valid response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnnotatedOp {
    #[serde(flatten)]
    pub op: ConsolidationOp,
    #[serde(default)]
    pub expected_outcome: Option<String>,
}

/// Parsed consolidator response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidatorResponse {
    #[serde(default)]
    pub session_summary: String,
    #[serde(default)]
    pub operations: Vec<AnnotatedOp>,
    #[serde(default)]
    pub promotions: Vec<PromotionRequest>,
}

/// One past consolidation run, as replayed into the prompt.
///
/// The point is self-correction: a model that can see its previous
/// batch failed — or that the user reversed it outright — has a reason
/// to change its behaviour, which a stateless prompt never gives it.
#[derive(Debug, Clone)]
pub struct ConsolidationRunBrief {
    pub run_id: String,
    /// One line per operation, e.g. `merge -> 42: ok` or
    /// `merge -> 999: rejected (does not exist)`.
    pub ops: Vec<String>,
    /// Whether the user reversed the whole run afterwards.
    pub rolled_back: bool,
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

/// Most existing memories the prompt will list (H-OD7).
///
/// Twenty rows at a typical memory length lands comfortably under the
/// 1500-token budget the plan allows, and the rows are ordered by
/// relevance to this session's candidates so the cap keeps the ones
/// most worth reconciling.
pub const MAX_EXISTING_MEMORIES: usize = 20;

/// Character budget for the existing-memories section (H-OD7).
///
/// ~1500 tokens at the usual ~4 chars/token. A char budget rather than a
/// token count because the consolidator has no tokenizer to hand, and
/// erring small here only costs the model the least important rows.
pub const MAX_EXISTING_MEMORIES_CHARS: usize = 6000;

/// Character budget for the history section (≈600 tokens).
pub const MAX_HISTORY_CHARS: usize = 2400;

/// How many previous runs to replay.
pub const MAX_HISTORY_RUNS: usize = 5;

/// Character budget for the transcript section.
///
/// The prompt has always announced "truncated to last N turns", but
/// nothing truncated: `/consolidate-session` joins *every* message in
/// the active conversation. A 98-message session produced a 187 KB
/// transcript (~47k tokens) in one consolidation call, which is most of
/// why a single run took 3m41s. ~24 000 chars is ~6k tokens — enough
/// tail for the summary to be faithful, bounded enough that the call
/// stays predictable however long a conversation runs.
///
/// A byte budget rather than a token count, for the same reason
/// [`MAX_EXISTING_MEMORIES_CHARS`] is: the consolidator has no
/// tokenizer to hand.
pub const MAX_TRANSCRIPT_CHARS: usize = 24_000;

/// Marker prefixed to a transcript that lost its head.
pub const TRANSCRIPT_ELISION: &str = "[… earlier turns elided …]";

/// Keep the tail of `transcript` within [`MAX_TRANSCRIPT_CHARS`].
///
/// The *tail*, not the head: the most recent turns are the ones the
/// candidates were extracted from, and the ones a session summary is
/// most likely to be asked about. Cuts on a `char` boundary and then
/// forward to the next line break, so the excerpt starts on a whole
/// line instead of mid-word.
pub fn truncate_transcript(transcript: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if transcript.len() <= MAX_TRANSCRIPT_CHARS {
        return Cow::Borrowed(transcript);
    }
    let mut cut = transcript.len() - MAX_TRANSCRIPT_CHARS;
    while !transcript.is_char_boundary(cut) {
        cut += 1;
    }
    let tail = &transcript[cut..];
    let tail = match tail.find('\n') {
        Some(nl) => &tail[nl + 1..],
        None => tail,
    };
    Cow::Owned(format!("{TRANSCRIPT_ELISION}\n{tail}"))
}

/// Build the prompt body for one session.
///
/// Assembles the template, the transcript, this session's candidates,
/// the existing memories an operation is allowed to reference, and what
/// became of the last few runs.
pub fn build_prompt(
    transcript: &str,
    candidates: &[CandidateBrief],
    existing: &[ExistingBrief],
    history: &[ConsolidationRunBrief],
) -> String {
    // The header used to promise truncation that never happened. Do the
    // truncation here rather than at the call site so every caller gets
    // the bound, and so the promise and the mechanism cannot drift apart
    // again.
    let transcript = truncate_transcript(transcript);
    let mut body = String::with_capacity(transcript.len() + 2048);
    body.push_str(PROMPT_V5.trim_start());
    body.push_str("\n\nTRANSCRIPT (most recent turns; earlier ones may be elided):\n");
    body.push_str(&transcript);
    body.push_str("\n\nCANDIDATES (extracted this session):\n");
    for (i, c) in candidates.iter().enumerate() {
        body.push_str(&format!(
            "[{i}] type={} importance={:.2} | {}\n",
            c.kind, c.importance, c.text
        ));
    }

    body.push_str("\nEXISTING_MEMORIES (the only ids you may reference):\n");
    if existing.is_empty() {
        body.push_str(
            "(none — do not emit MERGE or SUPERSEDE operations, as there is \
             nothing valid to name)\n",
        );
    } else {
        let mut used = 0usize;
        for (i, e) in existing.iter().take(MAX_EXISTING_MEMORIES).enumerate() {
            let line = format!(
                "id={} scope={} type={} | {}\n",
                e.id, e.scope_label, e.kind, e.text
            );
            // The budget governs how many *more* rows to add. One row
            // that busts it on its own still goes in: overshooting the
            // token estimate slightly is better than an empty section,
            // which leaves MERGE and SUPERSEDE with no legal target.
            if i > 0 && used + line.len() > MAX_EXISTING_MEMORIES_CHARS {
                break;
            }
            used += line.len();
            body.push_str(&line);
        }
    }

    if !history.is_empty() {
        body.push_str(&render_history(history));
    }
    body
}

/// Render the history section, newest first, dropping the oldest runs
/// once the budget is spent.
fn render_history(history: &[ConsolidationRunBrief]) -> String {
    let mut out = String::from("\nCONSOLIDATION_HISTORY (your previous runs):\n");
    let mut used = 0usize;
    for run in history.iter().take(MAX_HISTORY_RUNS) {
        let mut block = format!(
            "run {}{}\n",
            run.run_id,
            if run.rolled_back {
                "  [ROLLED BACK BY THE USER]"
            } else {
                ""
            }
        );
        for op in &run.ops {
            block.push_str(&format!("  {op}\n"));
        }
        // Oldest runs fall off the end because the loop is newest-first
        // and stops as soon as the budget is gone.
        if used + block.len() > MAX_HISTORY_CHARS {
            break;
        }
        used += block.len();
        out.push_str(&block);
    }
    out
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
        assert!(matches!(
            parsed.operations[0].op,
            ConsolidationOp::Add { .. }
        ));
        assert!(matches!(
            parsed.operations[1].op,
            ConsolidationOp::Merge {
                into_memory_id: 42,
                ..
            }
        ));
        assert!(matches!(
            parsed.operations[2].op,
            ConsolidationOp::Supersede {
                supersedes_memory_id: 17,
                ..
            }
        ));
        assert!(matches!(
            parsed.operations[3].op,
            ConsolidationOp::Drop { .. }
        ));
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
            &[],
        );
        assert!(body.contains("[0] type=decision"));
        assert!(body.contains("id=7 scope=repo"));
        assert!(body.contains("session_summary"));
    }

    // ── H1 / PR-7: session_v2 ────────────────────────────────────────

    fn existing(id: i64) -> ExistingBrief {
        ExistingBrief {
            id,
            text: format!("an existing memory number {id}"),
            kind: "decision".into(),
            scope_label: "repo".into(),
        }
    }

    fn run(run_id: &str, rolled_back: bool, ops: &[&str]) -> ConsolidationRunBrief {
        ConsolidationRunBrief {
            run_id: run_id.into(),
            ops: ops.iter().map(|s| s.to_string()).collect(),
            rolled_back,
        }
    }

    #[test]
    fn the_prompt_version_is_v5() {
        assert_eq!(PROMPT_VERSION, "session_v5");
        assert!(PROMPT_V5.contains("expected_outcome"));
        assert!(
            !PROMPT_V5.to_ascii_lowercase().contains("promotions"),
            "session_v5 must not ask the model for promotions"
        );
    }

    /// The v4 rubric change. A candidate restating a known memory is a
    /// reconfirmation, and `MERGE` is what records reconfirmation —
    /// `set_trust_score(target, MERGE_TRUST)`. The old wording
    /// ("Prefer DROP over MERGE if uncertain") threw that away.
    #[test]
    fn the_prompt_routes_reconfirmation_to_merge_not_drop() {
        assert!(
            !PROMPT_V5.contains("Prefer DROP over MERGE"),
            "the rule that suppressed every MERGE is back"
        );
        assert!(PROMPT_V5.contains("reconfirmation"));
        assert!(PROMPT_V5.contains("It is not the safe"));
    }

    /// The rubric change that earned the v3 bump. Pinned so a future
    /// prompt edit that drops it has to bump the version deliberately.
    #[test]
    fn the_prompt_forbids_adding_raw_conversation() {
        assert!(PROMPT_V5.contains("verbatim"));
        assert!(PROMPT_V5.contains("narrated work log"));
    }

    #[test]
    fn a_response_that_still_contains_promotions_parses() {
        let raw = r#"{
            "session_summary": "ok",
            "operations": [],
            "promotions": [{"memory_id": 8, "to_scope": "repo"}]
        }"#;
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.promotions.len(), 1);
        assert_eq!(parsed.promotions[0].memory_id, 8);
    }

    // ── Transcript truncation ────────────────────────────────────────

    #[test]
    fn a_short_transcript_is_passed_through_untouched() {
        let t = "USER: hi\nASSISTANT: hello\n";
        let got = truncate_transcript(t);
        assert_eq!(got, t);
        assert!(
            matches!(got, std::borrow::Cow::Borrowed(_)),
            "no allocation for a transcript already under budget"
        );
    }

    #[test]
    fn a_long_transcript_keeps_its_tail_within_budget() {
        let mut t = String::new();
        for i in 0..4000 {
            t.push_str(&format!("USER: turn number {i}\n"));
        }
        assert!(t.len() > MAX_TRANSCRIPT_CHARS, "fixture must overflow");

        let got = truncate_transcript(&t);
        assert!(
            got.len() <= MAX_TRANSCRIPT_CHARS + TRANSCRIPT_ELISION.len() + 1,
            "truncated transcript still over budget: {}",
            got.len()
        );
        assert!(got.starts_with(TRANSCRIPT_ELISION), "elision not marked");
        assert!(
            got.contains("turn number 3999"),
            "kept the head instead of the tail"
        );
        assert!(
            !got.contains("turn number 0\n"),
            "oldest turn survived truncation"
        );
    }

    /// The cut lands at an arbitrary byte offset, so it must not split a
    /// multi-byte character.
    #[test]
    fn truncation_never_splits_a_multibyte_char() {
        let t = "é".repeat(MAX_TRANSCRIPT_CHARS);
        let got = truncate_transcript(&t);
        assert!(got.len() <= MAX_TRANSCRIPT_CHARS + TRANSCRIPT_ELISION.len() + 1);
    }

    /// C-fix: the header promised truncation that never happened. Prove
    /// the promise is now backed by the mechanism, at the choke point
    /// every caller shares.
    #[test]
    fn build_prompt_bounds_an_oversized_transcript() {
        let huge = "USER: filler line that goes on\n".repeat(4000);
        assert!(huge.len() > MAX_TRANSCRIPT_CHARS);

        let body = build_prompt(&huge, &[], &[], &[]);
        assert!(body.contains(TRANSCRIPT_ELISION));
        assert!(
            body.len() < huge.len(),
            "prompt embedded the whole transcript"
        );
    }

    #[test]
    fn an_op_may_carry_an_expected_outcome() {
        let raw = r#"{"operations": [
            {"op": "MERGE", "candidate_index": 0, "into_memory_id": 5,
             "expected_outcome": "the tokio decision absorbs the newer note"}
        ]}"#;
        let parsed = parse_response(raw).unwrap();

        assert!(matches!(
            parsed.operations[0].op,
            ConsolidationOp::Merge {
                into_memory_id: 5,
                ..
            }
        ));
        assert_eq!(
            parsed.operations[0].expected_outcome.as_deref(),
            Some("the tokio decision absorbs the newer note")
        );
    }

    #[test]
    fn an_op_without_an_expected_outcome_still_parses() {
        // Forward-lenient: a model that ignores the new field must not
        // break the whole batch.
        let raw = r#"{"operations": [{"op": "ADD", "candidate_index": 0}]}"#;
        let parsed = parse_response(raw).unwrap();

        assert_eq!(parsed.operations.len(), 1);
        assert!(parsed.operations[0].expected_outcome.is_none());
    }

    #[test]
    fn existing_memories_are_capped_by_row_count() {
        let rows: Vec<ExistingBrief> = (1..=60).map(existing).collect();
        let body = build_prompt("t", &[], &rows, &[]);

        assert!(body.contains("id=1 "), "the most important rows survive");
        let listed = body.matches("\nid=").count();
        assert!(
            listed <= MAX_EXISTING_MEMORIES,
            "listed {listed} rows, cap is {MAX_EXISTING_MEMORIES}"
        );
    }

    #[test]
    fn existing_memories_are_capped_by_size() {
        let fat = ExistingBrief {
            id: 1,
            text: "x".repeat(MAX_EXISTING_MEMORIES_CHARS),
            kind: "decision".into(),
            scope_label: "repo".into(),
        };
        let rows = vec![fat, existing(2), existing(3)];
        let body = build_prompt("t", &[], &rows, &[]);

        assert!(body.contains("id=1 "));
        assert!(
            !body.contains("id=2 "),
            "the budget should have been spent by the first row"
        );
    }

    #[test]
    fn an_empty_existing_set_tells_the_model_not_to_reference_ids() {
        // The honest instruction when there is nothing valid to name —
        // otherwise every MERGE it emits is an invented id.
        let body = build_prompt("t", &[], &[], &[]);
        assert!(body.contains("do not emit MERGE or SUPERSEDE"));
    }

    #[test]
    fn history_renders_runs_and_marks_rollbacks() {
        let body = build_prompt(
            "t",
            &[],
            &[],
            &[
                run("run-2", true, &["merge -> 42: ok"]),
                run("run-1", false, &["add: ok", "merge -> 9: rejected (gone)"]),
            ],
        );

        assert!(body.contains("CONSOLIDATION_HISTORY"));
        assert!(body.contains("run run-2"));
        assert!(body.contains("[ROLLED BACK BY THE USER]"));
        assert!(body.contains("merge -> 9: rejected (gone)"));
        // run-1 was not reversed, so it carries no marker.
        let run1 = body.split("run run-1").nth(1).unwrap();
        assert!(!run1.starts_with("  [ROLLED BACK"));
    }

    #[test]
    fn history_is_capped_and_drops_the_oldest_runs() {
        // Newest first, so the budget is spent on the most recent.
        let fat: Vec<ConsolidationRunBrief> = (0..MAX_HISTORY_RUNS + 3)
            .map(|i| ConsolidationRunBrief {
                run_id: format!("run-{i}"),
                ops: vec!["y".repeat(MAX_HISTORY_CHARS / 2)],
                rolled_back: false,
            })
            .collect();
        let body = build_prompt("t", &[], &[], &fat);

        assert!(body.contains("run run-0"), "the newest run must survive");
        assert!(
            !body.contains("run run-7"),
            "the oldest runs fall off the budget"
        );
    }

    #[test]
    fn no_history_means_no_history_section() {
        // The rules in the prompt template mention CONSOLIDATION_HISTORY
        // regardless, so look for the section header itself.
        let body = build_prompt("t", &[], &[existing(1)], &[]);
        assert!(!body.contains("CONSOLIDATION_HISTORY (your previous runs)"));
    }
}
