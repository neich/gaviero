//! Per-turn memory extractor (Tier S / S3).
//!
//! Invoked from inside the writer task on `WriterMessage::TurnComplete`.
//! Runs a medium-tier LLM against a strict prompt, parses 0–5 structured
//! `ExtractedMemory` candidates from JSON, runs SHA + cosine dedup, and
//! writes survivors through `MemoryStore::store_scoped`. On any failure
//! (LLM unavailable, JSON parse error) falls back to writing a single
//! Run-scope raw record — **never lose the turn**.
//!
//! The extractor is deliberately narrow and version-pinned: `PROMPT_V1`
//! is a `const &str`, and the version is stamped on every extracted
//! memory's metadata so future prompt revisions retain provenance.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::consolidation_llm::ConsolidationLlm;
use super::scope::{MemoryType, WriteMeta, WriteScope};
use super::trust_defaults::MemorySource;

/// Current extractor prompt version. Written into every extracted
/// memory's `metadata.prompt_version` so later migrations can
/// reason about provenance. Bump in lock-step with `PROMPT_V1`.
pub const PROMPT_VERSION: &str = "s3.v1";

/// Absolute cap on extractions per turn. Mirrors the rubric in
/// `PROMPT_V1`; the parser also enforces this so a misbehaving LLM can't
/// flood the store.
pub const MAX_EXTRACTIONS_PER_TURN: usize = 5;

/// Minimum importance an extraction must carry to survive. Values below
/// this are dropped during parse. Kept here (not the prompt) so we can
/// tune without reprompting, at the cost of prompt/parser drift — the
/// prompt rubric and this constant must agree.
pub const MIN_IMPORTANCE: f32 = 0.3;

/// Version-pinned extractor prompt (v1). Strict JSON output contract.
///
/// Changes to this string **must** bump `PROMPT_VERSION`. Regression
/// tests snapshot the LLM output format against fixed transcripts — when
/// the prompt changes, snapshots need a conscious re-record.
pub const PROMPT_V1: &str = r#"You are a memory extractor for a coding agent. Read the TRANSCRIPT below and emit durable, non-obvious memories as strict JSON.

RULES
- Output ONLY one JSON object: `{"extractions": [...]}`. No prose, no code fences.
- 0 to 5 extractions per turn. Prefer quality over quantity. `{"extractions": []}` is a valid answer.
- Each extraction has: `type`, `scope_hint`, `text` (≤280 chars), `importance` (0.0–1.0), and optional `refs` (array of file paths or symbols).
- `type` ∈ { "decision", "lesson", "error", "convention", "preference", "gotcha", "invariant" }
- `scope_hint` ∈ { "run", "module", "repo", "workspace", "global" }
- Importance rubric:
  * 0.9+ : architectural truth that shapes future design
  * 0.6–0.9 : module-level decision or convention
  * 0.3–0.6 : local gotcha, small fix, minor preference
  * < 0.3 : DO NOT EMIT
- DO NOT extract:
  * Generic programming knowledge
  * Facts derivable from grep / tree-sitter / file listing
  * Restatements of the user's request
  * Tentative plans or assistant intent — only outcomes
  * Pleasantries, acknowledgements, task progress narration

TRANSCRIPT
```
{{TRANSCRIPT}}
```

JSON:"#;

/// Structured memory candidate as the LLM emits it. Importance is
/// floated (not clamped) at parse so we can log the raw value; clamping
/// happens at insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    #[serde(rename = "type")]
    pub kind: String,
    pub scope_hint: String,
    pub text: String,
    pub importance: f32,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionEnvelope {
    #[serde(default)]
    extractions: Vec<ExtractedMemory>,
}

/// Build the full prompt string for a transcript.
pub fn build_prompt(transcript: &str) -> String {
    PROMPT_V1.replace("{{TRANSCRIPT}}", transcript)
}

/// Parse an LLM response into a capped, filtered list of extractions.
///
/// Tolerates common LLM shape drift: leading/trailing prose, fenced code
/// blocks, JSON prefixed by whitespace. Drops entries below
/// `MIN_IMPORTANCE`. Caps at `MAX_EXTRACTIONS_PER_TURN`.
pub fn parse_response(raw: &str) -> Result<Vec<ExtractedMemory>> {
    let json_str = extract_json_object(raw)
        .ok_or_else(|| anyhow::anyhow!("extractor response contained no JSON object"))?;
    let envelope: ExtractionEnvelope =
        serde_json::from_str(&json_str).context("parsing extractor JSON envelope")?;
    let mut items: Vec<ExtractedMemory> = envelope
        .extractions
        .into_iter()
        .filter(|m| m.importance >= MIN_IMPORTANCE && !m.text.trim().is_empty())
        .collect();
    items.truncate(MAX_EXTRACTIONS_PER_TURN);
    Ok(items)
}

/// Run the extractor and return parsed candidates. Pure function — does
/// not touch the store. Caller (the writer task) handles dedup + insert.
pub async fn extract(llm: &dyn ConsolidationLlm, transcript: &str) -> Result<Vec<ExtractedMemory>> {
    let prompt = build_prompt(transcript);
    let response = llm
        .complete(prompt)
        .await
        .context("extractor LLM completion failed")?;
    parse_response(&response)
}

/// Resolve an `ExtractedMemory.scope_hint` into a concrete `WriteScope`
/// given the session context. Unknown / invalid hints fall back to
/// `Run`, which dies with the session — safe default.
pub fn resolve_scope(
    hint: &str,
    repo_id: &str,
    module_path: Option<&str>,
    run_id: &str,
) -> WriteScope {
    match hint.to_ascii_lowercase().as_str() {
        "global" => WriteScope::Global,
        "workspace" => WriteScope::Workspace,
        "repo" => WriteScope::Repo {
            repo_id: repo_id.to_string(),
        },
        "module" => match module_path {
            Some(m) => WriteScope::Module {
                repo_id: repo_id.to_string(),
                module_path: m.to_string(),
            },
            // Fall back to Repo when the extractor asked for Module but
            // the session has no module context.
            None => WriteScope::Repo {
                repo_id: repo_id.to_string(),
            },
        },
        _ => WriteScope::Run {
            repo_id: repo_id.to_string(),
            run_id: run_id.to_string(),
        },
    }
}

/// Parse the extractor's `type` field into a `MemoryType`. Maps each
/// rubric label one-to-one onto the post-B4-expanded enum so the
/// decay-exempt list (`memory.scoring.decayExemptTypes` — defaults to
/// `decision/convention/invariant/preference`) actually applies to
/// extractor-authored rows. Unknown types fall back to `Factual` so a
/// taxonomy mismatch never drops an otherwise good extraction.
pub fn resolve_memory_type(kind: &str) -> MemoryType {
    match kind.to_ascii_lowercase().as_str() {
        "decision" => MemoryType::Decision,
        "gotcha" => MemoryType::Gotcha,
        "convention" => MemoryType::Convention,
        "invariant" => MemoryType::Invariant,
        "preference" => MemoryType::Preference,
        "lesson" => MemoryType::Lesson,
        "error" => MemoryType::Error,
        "pattern" => MemoryType::Pattern,
        "procedural" => MemoryType::Procedural,
        "factual" => MemoryType::Factual,
        _ => MemoryType::Factual,
    }
}

/// Build the `WriteMeta` for an extracted memory.
///
/// A3: `source_kind = MemorySource::LlmExtracted` drives `trust_score =
/// 0.6` via the default mapping — plan's provenance default for LLM-
/// authored writes. The `prompt_version` is stamped into the `tag` so
/// future migrations can identify memories authored under a given
/// extractor prompt revision.
pub fn build_write_meta(ext: &ExtractedMemory, prompt_version: &str) -> WriteMeta {
    WriteMeta::for_source(MemorySource::LlmExtracted)
        .with_type(resolve_memory_type(&ext.kind))
        .with_importance(ext.importance)
        .with_tag(format!(
            "llm_extracted:{}:{prompt_version}",
            ext.kind.to_ascii_lowercase()
        ))
}

/// Override `source_kind = MemorySource::LlmAnnotated` for Tier A1
/// `<turn_annotations>` flags. Trust defaults to 0.7 via the mapping.
pub fn build_annotated_write_meta(ext: &ExtractedMemory) -> WriteMeta {
    WriteMeta::for_source(MemorySource::LlmAnnotated)
        .with_type(resolve_memory_type(&ext.kind))
        .with_importance(ext.importance)
        .with_tag(format!("llm_annotated:{}", ext.kind.to_ascii_lowercase()))
}

/// Extract a single `{ ... }` JSON object from an LLM response that may
/// be wrapped in prose, fenced code blocks, or trailing commentary.
///
/// Strategy: scan for the first `{`, then return the substring that ends
/// at the matching `}` by brace-counting while respecting string
/// literals. Keeps the parser robust to chatty models without needing a
/// full JSON pre-tokeniser.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_happy_path() {
        let raw = r#"{
            "extractions": [
                {"type":"decision","scope_hint":"repo","text":"use git2, never shell git","importance":0.9,"refs":["src/git.rs"]},
                {"type":"gotcha","scope_hint":"module","text":"embedding must run outside the mutex","importance":0.7,"refs":[]}
            ]
        }"#;
        let items = parse_response(raw).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "decision");
    }

    #[test]
    fn parse_response_drops_low_importance() {
        let raw = r#"{"extractions":[
            {"type":"lesson","scope_hint":"run","text":"low","importance":0.1}
        ]}"#;
        assert!(parse_response(raw).unwrap().is_empty());
    }

    #[test]
    fn parse_response_caps_at_max() {
        let mut inner = String::new();
        for _ in 0..10 {
            inner.push_str(
                r#"{"type":"decision","scope_hint":"repo","text":"x","importance":0.9},"#,
            );
        }
        inner.pop();
        let raw = format!(r#"{{"extractions":[{inner}]}}"#);
        let items = parse_response(&raw).unwrap();
        assert_eq!(items.len(), MAX_EXTRACTIONS_PER_TURN);
    }

    #[test]
    fn parse_response_handles_prose_wrap() {
        let raw = r#"Sure! Here you go:
```json
{"extractions": [{"type":"decision","scope_hint":"repo","text":"foo","importance":0.9}]}
```
Hope that helps."#;
        let items = parse_response(raw).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn parse_response_empty_extractions() {
        let raw = r#"{"extractions": []}"#;
        assert!(parse_response(raw).unwrap().is_empty());
    }

    #[test]
    fn parse_response_errors_on_no_json() {
        let err = parse_response("no json here").unwrap_err();
        assert!(err.to_string().contains("no JSON"));
    }

    #[test]
    fn resolve_memory_type_maps_reference_kinds_to_exempt_variants() {
        // B4 acceptance: extractor-authored convention/invariant/preference
        // must round-trip through the new MemoryType variants so the
        // decay-exempt list applies. Pre-fix this collapsed to Pattern /
        // Procedural and silently decayed.
        assert_eq!(resolve_memory_type("convention"), MemoryType::Convention);
        assert_eq!(resolve_memory_type("invariant"), MemoryType::Invariant);
        assert_eq!(resolve_memory_type("preference"), MemoryType::Preference);
        assert_eq!(resolve_memory_type("lesson"), MemoryType::Lesson);
        assert_eq!(resolve_memory_type("error"), MemoryType::Error);
        assert_eq!(resolve_memory_type("decision"), MemoryType::Decision);
        assert_eq!(resolve_memory_type("gotcha"), MemoryType::Gotcha);
        assert_eq!(resolve_memory_type("totally-new-type"), MemoryType::Factual);
    }

    #[test]
    fn resolve_scope_falls_back_to_repo_when_module_missing() {
        let s = resolve_scope("module", "abc", None, "run1");
        assert!(matches!(s, WriteScope::Repo { .. }));
    }

    #[test]
    fn resolve_scope_unknown_hint_defaults_to_run() {
        let s = resolve_scope("wat", "abc", None, "run1");
        assert!(matches!(s, WriteScope::Run { .. }));
    }
}
