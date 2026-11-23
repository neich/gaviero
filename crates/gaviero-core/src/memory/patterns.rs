//! WikiSkill-style pattern pages: short, LLM-maintained markdown pages that
//! record a recurring failure mode (or a strategy that worked), distilled
//! from outcome-labelled History by the pattern maintainer
//! (`pattern_maintainer.rs`).
//!
//! This module holds the pure parts: the operation types the maintainer
//! emits, validation, the page edit engine, the FTS query sanitiser and the
//! text formatting shared by the TUI and the CLI. Persistence lives in
//! `store/pattern_ops.rs` (tables `pattern_pages`, `pattern_page_revisions`,
//! `pattern_pages_fts`, schema v21). Pages are never injected automatically.

use serde::{Deserialize, Serialize};

/// Live pages allowed at once; a create beyond this fails.
pub const MAX_LIVE_PAGES: usize = 40;
/// A page's content may span at most this many lines.
pub const MAX_PAGE_LINES: usize = 40;
/// `index_line` is one line of at most this many chars.
pub const MAX_INDEX_LINE_CHARS: usize = 200;
/// Page names are at most this many chars.
pub const MAX_PAGE_NAME_CHARS: usize = 64;
/// At most this many evidence tags are kept per operation.
pub const MAX_EVIDENCE: usize = 8;
/// Trust reported for pattern pages in MCP results.
pub const PATTERN_PAGE_TRUST: f32 = 0.6;
/// `sleeptime_audit.origin` of maintainer batches.
pub const ORIGIN: &str = "pattern_maintainer";
/// `sleeptime_audit.origin` of rollbacks.
pub const ROLLBACK_ORIGIN: &str = "pattern_rollback";
/// Revision rows written by a rollback carry `batch_id = "rb-{batch}"`.
pub const ROLLBACK_BATCH_PREFIX: &str = "rb-";

/// One edit inside a `patch` operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PageEdit {
    /// Append `content` as new lines at the end of the page.
    Append { content: String },
    /// Replace the first occurrence of `target` with `content`.
    Replace { target: String, content: String },
    /// Insert `content` right after the first occurrence of `target`.
    InsertAfter { target: String, content: String },
}

/// One operation the maintainer proposes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatternOp {
    Create {
        name: String,
        index_line: String,
        content: String,
        #[serde(default)]
        evidence: Vec<String>,
    },
    Patch {
        name: String,
        edits: Vec<PageEdit>,
        #[serde(default)]
        index_line: Option<String>,
        #[serde(default)]
        evidence: Vec<String>,
    },
    Retire {
        name: String,
        #[serde(default)]
        reason: String,
    },
}

impl PatternOp {
    pub fn name(&self) -> &str {
        match self {
            Self::Create { name, .. } | Self::Patch { name, .. } | Self::Retire { name, .. } => {
                name
            }
        }
    }

    /// The `sleeptime_audit.kind` of this operation.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Create { .. } => "pattern_create",
            Self::Patch { .. } => "pattern_patch",
            Self::Retire { .. } => "pattern_retire",
        }
    }
}

/// One stored page.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPage {
    pub id: i64,
    pub name: String,
    pub index_line: String,
    pub content: String,
    pub revision: i64,
    pub retired_at: Option<String>,
    pub updated_at: String,
}

/// Result of applying one maintainer batch.
#[derive(Debug, Clone, Default)]
pub struct PatternApplyOutcome {
    pub batch_id: String,
    pub created: usize,
    pub patched: usize,
    pub retired: usize,
    pub failed: Vec<String>,
}

/// Result of rolling one batch back.
#[derive(Debug, Clone, Default)]
pub struct PatternRollbackOutcome {
    pub batch_id: String,
    pub reverted: usize,
    pub failed: Vec<String>,
}

/// One maintainer batch as listed by `recent_pattern_batches`.
#[derive(Debug, Clone)]
pub struct PatternBatch {
    pub batch_id: String,
    pub ops: usize,
    pub applied: usize,
    pub started_at: String,
    pub rolled_back: bool,
    pub prompt_version: Option<String>,
}

/// One outcome-labelled turn offered to the maintainer.
#[derive(Debug, Clone)]
pub struct LabelledTurn {
    pub history_tag: String,
    pub transcript: String,
    pub status: Option<String>,
    pub review: Option<String>,
    pub updated_at: String,
}

/// 1..=64 chars of `[a-z0-9-]`, starting with `[a-z0-9]`.
pub fn validate_name(name: &str) -> Result<(), String> {
    let len = name.chars().count();
    if len == 0 || len > MAX_PAGE_NAME_CHARS {
        return Err(format!(
            "name must be 1..={MAX_PAGE_NAME_CHARS} characters"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("name may contain only lowercase letters, digits and hyphens".into());
    }
    if name.starts_with('-') {
        return Err("name must start with a letter or digit".into());
    }
    Ok(())
}

/// Trimmed, non-empty, one line, at most `MAX_INDEX_LINE_CHARS` chars.
pub fn validate_index_line(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("index_line is empty".into());
    }
    if trimmed.contains('\n') {
        return Err("index_line must be a single line".into());
    }
    if trimmed.chars().count() > MAX_INDEX_LINE_CHARS {
        return Err(format!(
            "index_line is longer than {MAX_INDEX_LINE_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

/// Non-blank and at most `MAX_PAGE_LINES` lines.
pub fn validate_content(s: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err("content is empty".into());
    }
    let lines = s.lines().count();
    if lines > MAX_PAGE_LINES {
        return Err(format!(
            "content has {lines} lines; at most {MAX_PAGE_LINES} allowed"
        ));
    }
    Ok(())
}

fn missing_target(target: &str) -> String {
    let head: String = target.chars().take(60).collect();
    format!("replace target not found: {head}")
}

/// Apply `edits` in order. A `replace` / `insert_after` whose target is
/// empty or absent fails the whole patch.
pub fn apply_edits(content: &str, edits: &[PageEdit]) -> Result<String, String> {
    let mut page = content.to_string();
    for edit in edits {
        match edit {
            PageEdit::Append { content } => {
                if !page.is_empty() && !page.ends_with('\n') {
                    page.push('\n');
                }
                page.push_str(content);
            }
            PageEdit::Replace { target, content } => {
                if target.is_empty() || !page.contains(target.as_str()) {
                    return Err(missing_target(target));
                }
                page = page.replacen(target.as_str(), content, 1);
            }
            PageEdit::InsertAfter { target, content } => {
                let at = if target.is_empty() {
                    None
                } else {
                    page.find(target.as_str())
                };
                let Some(at) = at else {
                    return Err(missing_target(target));
                };
                page.insert_str(at + target.len(), content);
            }
        }
    }
    Ok(page)
}

/// FTS5 query for free text: alphanumeric tokens of ≥ 2 chars, lowercased,
/// at most 16, each quoted, joined with `OR`. `None` when no token is left.
pub fn fts_or_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("\"{}\"", t.to_lowercase()))
        .take(16)
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

/// The page index: one line per live page.
pub fn format_index(pages: &[PatternPage]) -> String {
    let mut out = format!("Pattern pages ({} live):", pages.len());
    for p in pages {
        out.push_str(&format!(
            "\n  {} — {}  (rev {}, updated {})",
            p.name, p.index_line, p.revision, p.updated_at
        ));
    }
    out
}

/// One line per maintainer batch, newest first.
pub fn format_batches(batches: &[PatternBatch]) -> String {
    batches
        .iter()
        .map(|b| {
            format!(
                "  {}  {}/{} applied{}  {}",
                b.batch_id,
                b.applied,
                b.ops,
                if b.rolled_back { "  (rolled back)" } else { "" },
                b.started_at
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Summary of one applied batch, failures on the following lines.
pub fn format_apply_outcome(o: &PatternApplyOutcome) -> String {
    let mut out = format!(
        "Pattern batch {}: {} created, {} patched, {} retired",
        o.batch_id, o.created, o.patched, o.retired
    );
    for f in &o.failed {
        out.push_str(&format!("\n  failed: {f}"));
    }
    out
}

/// Summary of one rollback, failures on the following lines.
pub fn format_rollback_outcome(o: &PatternRollbackOutcome) -> String {
    let mut out = format!(
        "Rolled back pattern batch {}: {} page(s) reverted",
        o.batch_id, o.reverted
    );
    for f in &o.failed {
        out.push_str(&format!("\n  not reverted: {f}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edits_append_replace_insert() {
        let page = "## Problem\nslow\n## Fix\nnone";
        let out = apply_edits(
            page,
            &[
                PageEdit::Replace {
                    target: "slow".into(),
                    content: "very slow".into(),
                },
                PageEdit::InsertAfter {
                    target: "## Fix\n".into(),
                    content: "cache it\n".into(),
                },
                PageEdit::Append {
                    content: "## Evidence\nhistory:a:b".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            "## Problem\nvery slow\n## Fix\ncache it\nnone\n## Evidence\nhistory:a:b"
        );
    }

    #[test]
    fn apply_edits_missing_target_errors() {
        let err = apply_edits(
            "abc",
            &[PageEdit::Replace {
                target: "zzz".into(),
                content: "y".into(),
            }],
        )
        .unwrap_err();
        assert!(err.starts_with("replace target not found: zzz"), "{err}");
        assert!(apply_edits(
            "abc",
            &[PageEdit::InsertAfter {
                target: String::new(),
                content: "y".into(),
            }],
        )
        .is_err());
    }

    #[test]
    fn validate_name_rules() {
        assert!(validate_name("lock-across-await").is_ok());
        assert!(validate_name("a1").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("-leading").is_err());
        assert!(validate_name("Upper").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn pattern_op_json_round_trip() {
        let op: PatternOp = serde_json::from_str(
            r#"{"op":"patch","name":"x","edits":[{"op":"insert_after","target":"a","content":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(
            op,
            PatternOp::Patch {
                name: "x".into(),
                edits: vec![PageEdit::InsertAfter {
                    target: "a".into(),
                    content: "b".into(),
                }],
                index_line: None,
                evidence: vec![],
            }
        );
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(serde_json::from_str::<PatternOp>(&json).unwrap(), op);
        assert_eq!(op.kind_str(), "pattern_patch");
        assert_eq!(op.name(), "x");
    }

    #[test]
    fn fts_or_query_sanitises() {
        assert_eq!(
            fts_or_query("foo-bar? (baz)").as_deref(),
            Some("\"foo\" OR \"bar\" OR \"baz\"")
        );
        assert_eq!(fts_or_query("?!"), None);
        assert_eq!(fts_or_query("a b"), None);
    }
}
