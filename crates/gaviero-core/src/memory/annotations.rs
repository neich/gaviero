//! `<turn_annotations>` sidecar parsing and stripping (Tier A / A1).
//!
//! The LLM is taught (via a system-prompt convention) to end every chat
//! response with a JSON block of the shape:
//!
//! ```text
//! <turn_annotations>
//! {
//!   "v": 1,
//!   "flags": [
//!     { "type": "decision", "importance": 0.8, "scope": "repo",
//!       "text": "...", "refs": ["src/foo.rs:L42"] }
//!   ],
//!   "session_thread": "investigating worktree cleanup races",
//!   "open_questions": ["what if a swarm agent panics mid-worktree?"]
//! }
//! </turn_annotations>
//! ```
//!
//! This module's job is to:
//! * **Find** the block by its delimiter pair — tolerating prose before it.
//! * **Parse** the JSON envelope into a typed `TurnAnnotations`, dropping
//!   gracefully to `None` on malformed payloads (never error the turn).
//! * **Strip** the block from the response text so it's never shown to
//!   the user.
//!
//! The writer task (S2) consumes the parsed annotations when handling
//! `WriterMessage::TurnComplete` and produces `LlmAnnotated` memories.

use serde::{Deserialize, Serialize};

/// Current schema version. Bump when the JSON shape changes.
pub const CURRENT_VERSION: u32 = 1;

/// Hard cap on `flags` per turn — matches extractor's own cap so the
/// combined signal channel can't flood the store. Excess are dropped
/// with a warning log upstream.
pub const MAX_FLAGS_PER_TURN: usize = 5;

/// Minimum importance for a flag to survive parse. Matches
/// `extractor::MIN_IMPORTANCE` — the rubric is shared.
pub const MIN_IMPORTANCE: f32 = 0.3;

/// Importance cap applied to flags from very short turns (<30s elapsed
/// from user send to LLM reply). Risk-mitigation against importance
/// inflation — a 10-second Q&A can't claim architectural-level 0.95.
pub const SHORT_TURN_IMPORTANCE_CAP: f32 = 0.7;
pub const SHORT_TURN_THRESHOLD_MS: u64 = 30_000;

/// One LLM-self-flagged memory candidate. Same shape family as
/// [`crate::memory::extractor::ExtractedMemory`] so the writer task can
/// treat them symmetrically modulo `source_kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationFlag {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub importance: f32,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub refs: Vec<String>,
}

fn default_scope() -> String {
    "repo".to_string()
}

/// Parsed `<turn_annotations>` envelope. `v` is the schema version —
/// parsers ignore blocks whose version is newer than they understand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnAnnotations {
    #[serde(default = "default_version")]
    pub v: u32,
    #[serde(default)]
    pub flags: Vec<AnnotationFlag>,
    #[serde(default)]
    pub session_thread: Option<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

/// Outcome of scanning an assistant response. `stripped` is the text
/// that should flow to the user (with the annotations block removed).
/// `annotations` is `Some` when a block was successfully parsed.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub stripped: String,
    pub annotations: Option<TurnAnnotations>,
    /// Present-but-malformed: the block delimiters were found but the
    /// JSON inside failed to parse. Callers should log a warning and
    /// emit a metric — prompt may need iteration.
    pub parse_error: Option<String>,
}

/// Scan `response` for a `<turn_annotations>...</turn_annotations>`
/// block, parse it, and return a `ParsedResponse` with the block
/// stripped from the text.
///
/// Robustness: scans for the **last** occurrence of the delimiter pair
/// so the convention's place-at-end nature survives mid-response
/// explanations of the tag. Lines starting with ``` are skipped on the
/// outer pass so fenced-code examples don't match.
pub fn parse_and_strip(response: &str) -> ParsedResponse {
    let (start, end) = match find_annotation_span(response) {
        Some(span) => span,
        None => {
            return ParsedResponse {
                stripped: response.to_string(),
                annotations: None,
                parse_error: None,
            };
        }
    };

    let open_tag = "<turn_annotations>";
    let close_tag = "</turn_annotations>";
    let inner_start = start + open_tag.len();
    let inner_end = end;
    let inner = response[inner_start..inner_end].trim();

    // Strip the block (and a preceding newline if present) from the
    // response text so it never surfaces to the user.
    let mut body_end = start;
    // Trim one trailing newline before the block to avoid a gap.
    if body_end > 0 && response.as_bytes()[body_end - 1] == b'\n' {
        body_end -= 1;
    }
    let after = end + close_tag.len();
    let mut stripped = String::with_capacity(response.len() - (after - start));
    stripped.push_str(&response[..body_end]);
    if after < response.len() {
        // Preserve any trailing content after the block (rare, but the
        // LLM might emit one final period).
        stripped.push_str(response[after..].trim_end());
    }

    // Parse the inner JSON. Malformed → annotations=None with a
    // parse_error so the caller can record a metric.
    match serde_json::from_str::<TurnAnnotations>(inner) {
        Ok(mut parsed) => {
            // Version gate: ignore future-version blocks until we
            // update the parser.
            if parsed.v > CURRENT_VERSION {
                return ParsedResponse {
                    stripped,
                    annotations: None,
                    parse_error: Some(format!(
                        "unknown annotations version {}, expected <= {}",
                        parsed.v, CURRENT_VERSION
                    )),
                };
            }
            // Filter + cap flags; also drop any without non-empty text.
            parsed
                .flags
                .retain(|f| f.importance >= MIN_IMPORTANCE && !f.text.trim().is_empty());
            parsed.flags.truncate(MAX_FLAGS_PER_TURN);
            ParsedResponse {
                stripped,
                annotations: Some(parsed),
                parse_error: None,
            }
        }
        Err(e) => ParsedResponse {
            stripped,
            annotations: None,
            parse_error: Some(e.to_string()),
        },
    }
}

/// Apply the short-turn importance cap in place. Writer task calls this
/// with the elapsed turn duration; unit-tested here.
pub fn apply_short_turn_cap(flags: &mut [AnnotationFlag], elapsed_ms: u64) {
    if elapsed_ms >= SHORT_TURN_THRESHOLD_MS {
        return;
    }
    for f in flags {
        if f.importance > SHORT_TURN_IMPORTANCE_CAP {
            f.importance = SHORT_TURN_IMPORTANCE_CAP;
        }
    }
}

/// Find the last `<turn_annotations>...</turn_annotations>` span that
/// is **not** inside a fenced-code block. Returns
/// `(offset_of_open_tag, offset_of_close_tag)` — the close tag offset
/// points at the `<` of `</turn_annotations>`.
///
/// Strategy: pick the last close tag, walk back to the nearest open tag
/// before it, then count triple-backtick fences before the open tag; if
/// odd (we're inside a fence), walk to the previous close tag and
/// retry. LLM responses are small, so quadratic worst-case is cheap.
fn find_annotation_span(response: &str) -> Option<(usize, usize)> {
    let open_tag = "<turn_annotations>";
    let close_tag = "</turn_annotations>";

    let mut search_end = response.len();
    loop {
        let close_off = response[..search_end].rfind(close_tag)?;
        // Find the nearest open tag before this close.
        let open_off = response[..close_off].rfind(open_tag)?;
        // Count `` ``` `` line-openers before open_off to detect fenced
        // state. A line begins after a newline (or at start).
        let mut fences = 0usize;
        let mut i = 0usize;
        while i < open_off {
            let line_start = i;
            let nl = response[i..open_off].find('\n').map(|n| i + n);
            let line_end = nl.unwrap_or(open_off);
            let line = &response[line_start..line_end];
            if line.trim_start().starts_with("```") {
                fences += 1;
            }
            match nl {
                Some(pos) => i = pos + 1,
                None => break,
            }
        }
        if fences % 2 == 0 {
            return Some((open_off, close_off));
        }
        // Inside a fence — this match is the convention-example, not
        // the real sidecar. Retry from before this close tag.
        search_end = close_off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_strip_happy_path() {
        let resp = "Here's your answer.\n\n<turn_annotations>\n\
                    {\"v\":1,\"flags\":[{\"type\":\"decision\",\"text\":\"use git2\",\
                    \"importance\":0.9,\"scope\":\"repo\"}],\
                    \"session_thread\":\"git2 vs shell git\",\
                    \"open_questions\":[\"what about rebasing?\"]}\n\
                    </turn_annotations>";
        let p = parse_and_strip(resp);
        assert!(p.parse_error.is_none());
        let ann = p.annotations.expect("annotations parsed");
        assert_eq!(ann.flags.len(), 1);
        assert_eq!(ann.session_thread.as_deref(), Some("git2 vs shell git"));
        assert_eq!(ann.open_questions.len(), 1);
        assert!(!p.stripped.contains("<turn_annotations>"));
        assert!(p.stripped.contains("Here's your answer."));
    }

    #[test]
    fn parse_and_strip_missing_block_returns_original() {
        let resp = "plain response, no annotations";
        let p = parse_and_strip(resp);
        assert_eq!(p.stripped, resp);
        assert!(p.annotations.is_none());
        assert!(p.parse_error.is_none());
    }

    #[test]
    fn parse_and_strip_malformed_json_is_reported_not_fatal() {
        let resp = "reply\n<turn_annotations>\nnot even json\n</turn_annotations>";
        let p = parse_and_strip(resp);
        assert!(p.annotations.is_none());
        assert!(p.parse_error.is_some(), "should record parse_error");
        assert!(!p.stripped.contains("<turn_annotations>"));
    }

    #[test]
    fn parse_and_strip_drops_low_importance_flags() {
        let resp = "reply\n<turn_annotations>\n\
                    {\"flags\":[\
                    {\"type\":\"lesson\",\"text\":\"low\",\"importance\":0.1,\"scope\":\"run\"},\
                    {\"type\":\"decision\",\"text\":\"keep\",\"importance\":0.8,\"scope\":\"repo\"}\
                    ]}\n</turn_annotations>";
        let ann = parse_and_strip(resp).annotations.unwrap();
        assert_eq!(ann.flags.len(), 1);
        assert_eq!(ann.flags[0].text, "keep");
    }

    #[test]
    fn parse_and_strip_caps_at_max_flags() {
        let mut flags = String::new();
        for _ in 0..10 {
            flags.push_str(r#"{"type":"decision","text":"x","importance":0.9,"scope":"repo"},"#);
        }
        flags.pop();
        let resp =
            format!("reply\n<turn_annotations>\n{{\"flags\":[{flags}]}}\n</turn_annotations>");
        let ann = parse_and_strip(&resp).annotations.unwrap();
        assert_eq!(ann.flags.len(), MAX_FLAGS_PER_TURN);
    }

    #[test]
    fn parse_and_strip_ignores_example_inside_code_fence() {
        // The LLM sometimes explains the convention inside a fenced
        // code block. That occurrence must NOT be parsed as the real
        // annotations — only the trailing unfenced block counts.
        let resp = "Here's how the tag looks:\n\
                    ```\n\
                    <turn_annotations>\n\
                    {\"flags\":[]}\n\
                    </turn_annotations>\n\
                    ```\n\
                    Now my actual reply, followed by real flags:\n\
                    <turn_annotations>\n\
                    {\"flags\":[{\"type\":\"decision\",\"text\":\"real\",\
                    \"importance\":0.9,\"scope\":\"repo\"}]}\n\
                    </turn_annotations>";
        let p = parse_and_strip(resp);
        let ann = p.annotations.unwrap();
        assert_eq!(ann.flags.len(), 1);
        assert_eq!(ann.flags[0].text, "real");
        // The fenced example block survives (it's inside ``` so not the
        // parsed block). The real block below is stripped.
        assert!(p.stripped.contains("```"));
        assert!(
            !p.stripped.contains("\"real\""),
            "real block must be stripped"
        );
    }

    #[test]
    fn parse_and_strip_rejects_future_version() {
        let resp = "<turn_annotations>\n{\"v\":999,\"flags\":[]}\n</turn_annotations>";
        let p = parse_and_strip(resp);
        assert!(p.annotations.is_none());
        assert!(p.parse_error.is_some());
    }

    #[test]
    fn short_turn_cap_is_applied() {
        let mut flags = vec![AnnotationFlag {
            kind: "decision".to_string(),
            text: "x".to_string(),
            importance: 0.95,
            scope: "repo".to_string(),
            refs: vec![],
        }];
        apply_short_turn_cap(&mut flags, 10_000);
        assert!((flags[0].importance - SHORT_TURN_IMPORTANCE_CAP).abs() < 1e-6);

        // Long turns unaffected.
        let mut flags2 = vec![AnnotationFlag {
            kind: "decision".to_string(),
            text: "x".to_string(),
            importance: 0.95,
            scope: "repo".to_string(),
            refs: vec![],
        }];
        apply_short_turn_cap(&mut flags2, 60_000);
        assert!((flags2[0].importance - 0.95).abs() < 1e-6);
    }
}
