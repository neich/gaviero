//! Heuristic prompt-section classifier.
//!
//! Splits an assembled prompt into a [`PromptDigest`] keyed by
//! [`SectionKind`]. Detection is structural string matching against
//! the XML section tags the prompt assembler emits — no regex.
//!
//! Tag sources (kept in sync; if the renderer changes its tags, the
//! tests in this module must be updated):
//!
//! - `<user_message>...</user_message>` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::render_swarm_prompt`
//!   and `crates/gaviero-core/src/agent_session/claude.rs::run_claude_turn`
//! - `<project_memory>...</project_memory>` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::render_memory_block`
//!   (structured swarm path) and
//!   `crates/gaviero-core/src/memory/retrieval.rs::render_block`
//!   (chat-injection path)
//! - `<repo_outline>...</repo_outline>` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::render_graph_block`
//! - `<prev_conv>...</prev_conv>` and `<file_refs>...</file_refs>` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::build_enriched_prompt`
//! - `<file_scope>...</file_scope>` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::render_swarm_prompt`
//!
//! ## Fallback for untagged user messages
//!
//! When no `<user_message>` tag is present (test harnesses that
//! pre-build prompts manually), leading text before the first tagged
//! section is classified as `UserMessage`.
//!
//! ## On `Other`
//!
//! Anything outside the recognised tags lands in `Other`. A non-trivial
//! `Other` bucket is itself diagnostic — it means the assembler emitted
//! content the heuristic doesn't recognise. Callers should treat large
//! `Other` ranges as a signal to extend this module.
//!
//! ## On determinism for snapshot tests (T1.6)
//!
//! Sections derived from Claude's text (notably `ReplayHistory` past the
//! first turn) are non-deterministic across runs. T1.6's snapshot
//! serializer should keep `bytes`/`tokens_approx` for those sections but
//! drop the SHA — kind/bytes/tokens are stable across the assistant
//! reply variance, the SHA is not.

use std::cmp::Ordering;

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SectionKind {
    UserMessage,
    MemorySelections,
    GraphSelections,
    FileRefs,
    ReplayHistory,
    FileScope,
    Wrapper,
    Other,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub kind: SectionKind,
    /// `(start, end)` byte offsets into the original `prompt` blob,
    /// half-open as is convention. `bytes == end - start`.
    pub byte_range: (usize, usize),
    pub bytes: usize,
    /// `bytes.div_ceil(4)` — matches `memory::retrieval` token budgeting.
    pub tokens_approx: usize,
    /// SHA-256 hex digest of the section body.
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct PromptDigest {
    pub turn_id: String,
    pub total_bytes: usize,
    pub total_tokens_approx: usize,
    pub sections: Vec<Section>,
}

const WRAPPER_PREFIX: &str = "Read the full prompt at @";
const WRAPPER_MAX_BYTES: usize = 256;

/// (kind, open tag, close tag). All section types are paired XML tags;
/// detection is a literal substring search for the open tag followed by
/// its matching close tag.
const TAG_TABLE: &[(SectionKind, &str, &str)] = &[
    (SectionKind::UserMessage, "<user_message>", "</user_message>"),
    (
        SectionKind::MemorySelections,
        "<project_memory>",
        "</project_memory>",
    ),
    (
        SectionKind::GraphSelections,
        "<repo_outline>",
        "</repo_outline>",
    ),
    (SectionKind::ReplayHistory, "<prev_conv>", "</prev_conv>"),
    (SectionKind::FileRefs, "<file_refs>", "</file_refs>"),
    (SectionKind::FileScope, "<file_scope>", "</file_scope>"),
];

pub fn classify(turn_id: &str, prompt: &str) -> PromptDigest {
    let total_bytes = prompt.len();
    let total_tokens_approx = total_bytes.div_ceil(4);

    if total_bytes < WRAPPER_MAX_BYTES && prompt.starts_with(WRAPPER_PREFIX) {
        let s = make_section(SectionKind::Wrapper, 0, total_bytes, prompt);
        return PromptDigest {
            turn_id: turn_id.to_string(),
            total_bytes,
            total_tokens_approx,
            sections: vec![s],
        };
    }

    // Find every (kind, start, end) tag pair. Each section's byte range
    // is closed-exact: start = first byte of opening tag, end = byte
    // past the closing tag. Multiple instances of the same tag pair are
    // all captured (memory + file_refs can repeat across paths).
    let mut hits: Vec<(SectionKind, usize, usize)> = Vec::new();
    for (kind, open, close) in TAG_TABLE.iter().copied() {
        let mut cursor = 0;
        while let Some(rel_open) = prompt[cursor..].find(open) {
            let open_at = cursor + rel_open;
            let body_start = open_at + open.len();
            if let Some(rel_close) = prompt[body_start..].find(close) {
                let end = body_start + rel_close + close.len();
                hits.push((kind, open_at, end));
                cursor = end;
            } else {
                break;
            }
        }
    }

    hits.sort_by(|a, b| a.1.cmp(&b.1));

    let mut sections: Vec<Section> = hits
        .into_iter()
        .map(|(kind, start, end)| make_section(kind, start, end, prompt))
        .collect();

    fill_gaps_with_user_or_other(prompt, &mut sections);

    sections.sort_by(|a, b| match a.byte_range.0.cmp(&b.byte_range.0) {
        Ordering::Equal => a.byte_range.1.cmp(&b.byte_range.1),
        other => other,
    });

    PromptDigest {
        turn_id: turn_id.to_string(),
        total_bytes,
        total_tokens_approx,
        sections,
    }
}

fn make_section(kind: SectionKind, start: usize, end: usize, prompt: &str) -> Section {
    let body = &prompt[start..end];
    let bytes = end - start;
    Section {
        kind,
        byte_range: (start, end),
        bytes,
        tokens_approx: bytes.div_ceil(4),
        sha256: sha_hex(body),
    }
}

fn sha_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// Walk the prompt, filling gaps between tagged sections.
///
/// When no `<user_message>` tag is present, the leading run before any
/// tagged section is reclassified as `UserMessage` so test harnesses
/// that pre-build prompts manually (without going through the production
/// renderer) still attribute their leading text correctly. Whitespace-only
/// gaps (the `\n\n` separators between tagged sections the renderer emits)
/// are skipped. Any other non-trivial gap becomes `Other` so the caller
/// can spot unrecognised ranges.
fn fill_gaps_with_user_or_other(prompt: &str, sections: &mut Vec<Section>) {
    sections.sort_by_key(|s| s.byte_range.0);

    let has_user_message = sections
        .iter()
        .any(|s| s.kind == SectionKind::UserMessage);

    let total = prompt.len();
    let mut cursor = 0usize;
    let mut leading_user_taken = has_user_message;
    let mut additions: Vec<Section> = Vec::new();
    for s in sections.iter() {
        if s.byte_range.0 > cursor {
            let gap = &prompt[cursor..s.byte_range.0];
            let is_whitespace_only = gap.chars().all(char::is_whitespace);
            if !is_whitespace_only {
                let kind = if !leading_user_taken {
                    leading_user_taken = true;
                    SectionKind::UserMessage
                } else {
                    SectionKind::Other
                };
                additions.push(make_section(kind, cursor, s.byte_range.0, prompt));
            }
        }
        cursor = s.byte_range.1;
    }
    if cursor < total {
        let trailing = &prompt[cursor..total];
        let is_whitespace_only = trailing.chars().all(char::is_whitespace);
        if !is_whitespace_only {
            let kind = if !leading_user_taken {
                // No tagged sections at all — entire prompt is user text.
                SectionKind::UserMessage
            } else {
                SectionKind::Other
            };
            additions.push(make_section(kind, cursor, total, prompt));
        }
    }
    sections.extend(additions);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(d: &PromptDigest) -> Vec<SectionKind> {
        d.sections.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn classify_empty_prompt_yields_no_sections() {
        let d = classify("t-empty", "");
        assert_eq!(d.total_bytes, 0);
        assert_eq!(d.total_tokens_approx, 0);
        assert!(d.sections.is_empty());
    }

    #[test]
    fn classify_pure_user_message_has_one_section() {
        let prompt = "Summarise the composite scoring formula.";
        let d = classify("t-user", prompt);
        assert_eq!(kinds(&d), vec![SectionKind::UserMessage]);
        assert_eq!(d.sections[0].byte_range, (0, prompt.len()));
        assert_eq!(d.sections[0].bytes, prompt.len());
        assert!(!d.sections[0].sha256.is_empty());
    }

    #[test]
    fn classify_wrapper_prompt_short_circuits() {
        let prompt = "Read the full prompt at @.gaviero/tmp/prompt-abc.md and follow its instructions.";
        let d = classify("t-wrapper", prompt);
        assert_eq!(kinds(&d), vec![SectionKind::Wrapper]);
        assert_eq!(d.total_bytes, prompt.len());
    }

    #[test]
    fn classify_wrapper_must_be_short() {
        // A long prompt that happens to start with the wrapper prefix
        // is NOT classified as a wrapper (only spill argv is short).
        let mut prompt = String::from("Read the full prompt at @x.md and follow its instructions.");
        prompt.push_str(&"x".repeat(WRAPPER_MAX_BYTES));
        let d = classify("t-long", &prompt);
        assert!(!matches!(d.sections.first().map(|s| s.kind), Some(SectionKind::Wrapper)));
    }

    #[test]
    fn classify_untagged_user_then_memory_block() {
        // Test-harness path: prompt assembled manually without a
        // <user_message> wrapper. Leading text falls back to UserMessage.
        let prompt =
            "User question?\n<project_memory>\n[repo] decision: x|s0.9\n</project_memory>";
        let d = classify("t-mem", prompt);
        assert_eq!(
            kinds(&d),
            vec![SectionKind::UserMessage, SectionKind::MemorySelections]
        );
        let mem = &d.sections[1];
        assert!(mem.bytes > 0);
        assert_eq!(
            &prompt[mem.byte_range.0..mem.byte_range.1],
            "<project_memory>\n[repo] decision: x|s0.9\n</project_memory>"
        );
    }

    #[test]
    fn classify_tagged_user_then_memory_block() {
        // Production path: <user_message> wraps the actual prompt and
        // takes precedence over the leading-text fallback.
        let prompt = "<user_message>\nUser question?\n</user_message>\n\n<project_memory>\n[repo] decision: x|s0.9\n</project_memory>";
        let d = classify("t-tagged", prompt);
        assert_eq!(
            kinds(&d),
            vec![SectionKind::UserMessage, SectionKind::MemorySelections]
        );
    }

    #[test]
    fn classify_prev_conv_and_file_refs_tags() {
        let prompt = "<user_message>\nask\n</user_message>\n\n<prev_conv>\nU: q\nA: a\n</prev_conv>\n\n<file_refs>\n@src/lib.rs\nfn x() {}\n/@src/lib.rs\n</file_refs>";
        let d = classify("t-hist-files", prompt);
        assert_eq!(
            kinds(&d),
            vec![
                SectionKind::UserMessage,
                SectionKind::ReplayHistory,
                SectionKind::FileRefs,
            ]
        );
    }

    #[test]
    fn classify_repo_outline_tag_recognised() {
        let prompt = "<user_message>\nask\n</user_message>\n\n<repo_outline>\n  crates/foo.rs (s1.00)\n</repo_outline>";
        let d = classify("t-repo", prompt);
        assert!(kinds(&d).contains(&SectionKind::GraphSelections));
    }

    #[test]
    fn classify_file_scope_tag_recognised() {
        let prompt = "<user_message>\nask\n</user_message>\n\n<file_scope>\n**Owned paths** (read/write):\n- `src/lib.rs`\n</file_scope>";
        let d = classify("t-scope", prompt);
        assert!(kinds(&d).contains(&SectionKind::FileScope));
    }

    #[test]
    fn classify_full_prompt_keeps_section_order() {
        let prompt = concat!(
            "<user_message>\nUser question?\n</user_message>\n\n",
            "<project_memory>\n[repo] decision: x|s0.9\n</project_memory>\n\n",
            "<repo_outline>\n  crates/foo.rs (s1.00)\n</repo_outline>\n\n",
            "<prev_conv>\nU: prev\nA: prev\n</prev_conv>\n\n",
            "<file_refs>\n@src/lib.rs\nfn x() {}\n/@src/lib.rs\n</file_refs>",
        );
        let d = classify("t-full", prompt);
        let order = kinds(&d);
        assert_eq!(order[0], SectionKind::UserMessage);
        assert_eq!(order[1], SectionKind::MemorySelections);
        let rest = &order[2..];
        assert!(rest.contains(&SectionKind::GraphSelections));
        assert!(rest.contains(&SectionKind::ReplayHistory));
        assert!(rest.contains(&SectionKind::FileRefs));
    }

    #[test]
    fn tokens_approx_uses_div_ceil_4() {
        let prompt = "x".repeat(13);
        let d = classify("t-tok", &prompt);
        assert_eq!(d.total_tokens_approx, 4); // 13.div_ceil(4) == 4
    }

    #[test]
    fn sha_changes_when_body_changes() {
        let a = classify("t", "<project_memory>\nA\n</project_memory>");
        let b = classify("t", "<project_memory>\nB\n</project_memory>");
        let mem_a = a.sections.iter().find(|s| s.kind == SectionKind::MemorySelections).unwrap();
        let mem_b = b.sections.iter().find(|s| s.kind == SectionKind::MemorySelections).unwrap();
        assert_ne!(mem_a.sha256, mem_b.sha256);
    }
}
