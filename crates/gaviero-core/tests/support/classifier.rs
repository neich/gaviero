//! Heuristic prompt-section classifier.
//!
//! Splits an assembled prompt into a [`PromptDigest`] keyed by
//! [`SectionKind`]. Detection is structural string matching against
//! the literal markers the prompt assembler already emits — no regex.
//!
//! Marker sources (kept in sync; if the renderer changes its tags, the
//! tests in this module must be updated):
//!
//! - `<project_memory>...</project_memory>` ←
//!   `crates/gaviero-core/src/memory/retrieval.rs::render_block`
//! - `\nPrevConv:\n` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::build_enriched_prompt`
//! - `\nFiles:\n` ← same source as `PrevConv:`
//! - `\nRepo:\n` ←
//!   `crates/gaviero-core/src/swarm/backend/shared.rs::render_graph_block`
//!   (caveman-style header used on the swarm path; chat-injection path
//!   does not render graph context)
//!
//! ## On `Other`
//!
//! Anything outside the recognised markers lands in `Other`. A non-trivial
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

const PROJECT_MEMORY_OPEN: &str = "<project_memory>";
const PROJECT_MEMORY_CLOSE: &str = "</project_memory>";
const PREV_CONV_MARKER: &str = "\nPrevConv:\n";
const FILES_MARKER: &str = "\nFiles:\n";
const REPO_MARKER: &str = "\nRepo:\n";
const WRAPPER_PREFIX: &str = "Read the full prompt at @";
const WRAPPER_MAX_BYTES: usize = 256;

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

    let mut hits: Vec<(SectionKind, usize, usize)> = Vec::new();

    if let Some(open) = prompt.find(PROJECT_MEMORY_OPEN) {
        let body_start = open;
        if let Some(close_rel) = prompt[body_start..].find(PROJECT_MEMORY_CLOSE) {
            let end = body_start + close_rel + PROJECT_MEMORY_CLOSE.len();
            hits.push((SectionKind::MemorySelections, open, end));
        }
    }

    let bare_markers: [(SectionKind, &str); 3] = [
        (SectionKind::ReplayHistory, PREV_CONV_MARKER),
        (SectionKind::FileRefs, FILES_MARKER),
        (SectionKind::GraphSelections, REPO_MARKER),
    ];
    for (kind, marker) in bare_markers {
        if let Some(pos) = prompt.find(marker) {
            // Section starts after the leading newline so the marker
            // itself is part of the body (preserved literally).
            let start = pos + 1;
            hits.push((kind, start, total_bytes));
        }
    }

    hits.sort_by(|a, b| a.1.cmp(&b.1));

    // Trim each marker-driven section to end at the next marker start
    // (or at `total_bytes` if it's the last). Memory's range is exact;
    // bare-line markers run to the next section header.
    let mut sections: Vec<Section> = Vec::new();
    for (i, (kind, start, end_hint)) in hits.iter().copied().enumerate() {
        let end = match kind {
            SectionKind::MemorySelections => end_hint,
            _ => hits
                .iter()
                .skip(i + 1)
                .map(|(_, s, _)| *s)
                .min()
                .unwrap_or(total_bytes)
                .max(start),
        };
        if end <= start {
            continue;
        }
        sections.push(make_section(kind, start, end, prompt));
    }

    fill_gaps_with_user_or_other(prompt, &mut sections, turn_id);

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

/// Walk the prompt, filling gaps between marker-driven sections.
///
/// The leading run before any marker section becomes `UserMessage`
/// (the user's question). Any other gap becomes `Other` so the
/// caller can spot unrecognised ranges.
fn fill_gaps_with_user_or_other(prompt: &str, sections: &mut Vec<Section>, _turn_id: &str) {
    sections.sort_by_key(|s| s.byte_range.0);

    let total = prompt.len();
    let mut cursor = 0usize;
    let mut leading_user_taken = false;
    let mut additions: Vec<Section> = Vec::new();
    for s in sections.iter() {
        if s.byte_range.0 > cursor {
            let kind = if !leading_user_taken {
                leading_user_taken = true;
                SectionKind::UserMessage
            } else {
                SectionKind::Other
            };
            additions.push(make_section(kind, cursor, s.byte_range.0, prompt));
        }
        cursor = s.byte_range.1;
    }
    if cursor < total {
        let kind = if !leading_user_taken {
            // No marker sections at all — entire prompt is user text.
            SectionKind::UserMessage
        } else {
            SectionKind::Other
        };
        additions.push(make_section(kind, cursor, total, prompt));
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
    fn classify_user_then_memory_block() {
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
    fn classify_history_and_files_markers_are_caveman_bare_lines() {
        let prompt = "ask\n\nPrevConv:\nU: q\nA: a\n\nFiles:\n@src/lib.rs\nfn x() {}\n/@src/lib.rs\n";
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
    fn classify_repo_graph_marker_recognised() {
        let prompt = "ask\n\nRepo:\n  crates/foo.rs (s1.00)\n";
        let d = classify("t-repo", prompt);
        assert!(kinds(&d).contains(&SectionKind::GraphSelections));
    }

    #[test]
    fn classify_full_prompt_keeps_section_order() {
        let prompt = concat!(
            "User question?\n",
            "<project_memory>\n[repo] decision: x|s0.9\n</project_memory>\n",
            "\n",
            "Repo:\n  crates/foo.rs (s1.00)\n",
            "\n",
            "PrevConv:\nU: prev\nA: prev\n",
            "\n",
            "Files:\n@src/lib.rs\nfn x() {}\n/@src/lib.rs\n",
        );
        let d = classify("t-full", prompt);
        let order = kinds(&d);
        assert_eq!(order[0], SectionKind::UserMessage);
        assert_eq!(order[1], SectionKind::MemorySelections);
        // Subsequent sections appear in marker order: Repo:, PrevConv:, Files:
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
