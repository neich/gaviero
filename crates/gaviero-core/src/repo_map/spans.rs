//! Which symbol a reference sits in.
//!
//! A reference edge is sourced from the innermost stored symbol whose line
//! range contains the reference (`src/app.rs::run` for a call inside `run`),
//! or from the file itself for a top-level reference (an import). That is
//! what lets the graph answer "which functions call X" instead of only
//! "which files mention X".
//!
//! The ranges come from the `nodes` table of the source file, not from a
//! fresh parse, so a full build and a targeted update
//! ([`super::graph_builder::update_paths`]) attribute every edge the same
//! way. Same-named symbols in one file share one node (the graph keys nodes
//! by `file::name`), so they keep a single range.

use super::store::{GraphStore, SymbolSpan};

/// The symbol line ranges of one file ([`GraphStore::symbol_spans`]).
#[derive(Debug, Clone, Default)]
pub struct SymbolSpans {
    spans: Vec<SymbolSpan>,
}

impl SymbolSpans {
    /// Load `file`'s ranges from the store.
    pub fn load(store: &GraphStore, file: &str) -> anyhow::Result<Self> {
        Ok(Self {
            spans: store.symbol_spans(file)?,
        })
    }

    /// Ranges given directly (tests, callers that already hold them).
    pub fn from_spans(spans: Vec<SymbolSpan>) -> Self {
        Self { spans }
    }

    /// Innermost span containing 0-based `line`: greatest `line_start`,
    /// then smallest `line_end`, then `qualified_name` ascending.
    pub fn enclosing(&self, line: i64) -> Option<&SymbolSpan> {
        self.spans
            .iter()
            .filter(|s| s.line_start <= line && line <= s.line_end)
            .min_by(|a, b| {
                b.line_start
                    .cmp(&a.line_start)
                    .then(a.line_end.cmp(&b.line_end))
                    .then(a.qualified_name.cmp(&b.qualified_name))
            })
    }

    /// `(source_qn, source_is_test_symbol)` for a reference on `line` of
    /// `file`: the enclosing span's qualified name, or `file` itself (the
    /// File node's qualified name); the flag is `kind == "Test"` of that
    /// span (false for the file).
    pub fn source_for(&self, file: &str, line: i64) -> (String, bool) {
        match self.enclosing(line) {
            Some(span) => (span.qualified_name.clone(), span.kind == "Test"),
            None => (file.to_string(), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(qn: &str, kind: &str, start: i64, end: i64) -> SymbolSpan {
        SymbolSpan {
            qualified_name: qn.into(),
            kind: kind.into(),
            line_start: start,
            line_end: end,
        }
    }

    #[test]
    fn the_innermost_span_wins() {
        let spans = SymbolSpans::from_spans(vec![
            span("f.rs::outer", "Function", 0, 10),
            span("f.rs::inner", "Test", 3, 5),
        ]);
        assert_eq!(spans.enclosing(4).unwrap().qualified_name, "f.rs::inner");
        assert_eq!(spans.enclosing(7).unwrap().qualified_name, "f.rs::outer");
        assert!(spans.enclosing(20).is_none());
        assert_eq!(spans.source_for("f.rs", 20), ("f.rs".to_string(), false));
        assert_eq!(spans.source_for("f.rs", 4), ("f.rs::inner".to_string(), true));
        assert_eq!(spans.source_for("f.rs", 0), ("f.rs::outer".to_string(), false));
    }

    #[test]
    fn a_start_line_tie_picks_the_shorter_span() {
        let spans = SymbolSpans::from_spans(vec![
            span("f.rs::long", "Function", 2, 9),
            span("f.rs::short", "Function", 2, 4),
        ]);
        assert_eq!(spans.enclosing(3).unwrap().qualified_name, "f.rs::short");
        assert_eq!(spans.enclosing(6).unwrap().qualified_name, "f.rs::long");
    }
}
