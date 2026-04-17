//! Syntax-aware auto-indentation engine.
//!
//! Uses tree-sitter indent queries (`indents.scm`) when available,
//! with a bracket-counting fallback for languages without queries.

pub mod bracket;
pub mod captures;
pub mod config;
pub mod heuristic;
pub mod predicates;
pub mod treesitter;
pub mod utils;

/// Result of an indent computation.
#[derive(Debug, Clone)]
pub struct IndentResult {
    /// The whitespace string to insert (spaces, tabs, or alignment prefix).
    pub whitespace: String,
    /// Net indent level relative to zero (for debugging/testing).
    pub level: i32,
}

/// Which heuristic to use for indent computation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndentHeuristic {
    /// Compute absolute indent from tree structure alone.
    TreeSitter,
    /// Compute relative delta applied to actual indent of a nearby line (default).
    Hybrid,
}

/// Compute indentation for a new line.
///
/// The editor resolves which injection layer the cursor is in and passes
/// that layer's tree and query. The indent engine has no injection knowledge.
///
/// If `tree` or `indent_query` is None, falls back to bracket counting.
pub fn compute_indent(
    doc: &ropey::Rope,
    tree: Option<&tree_sitter::Tree>,
    indent_query: Option<&tree_sitter::Query>,
    cursor_byte: usize,
    new_line_below: bool,
    tab_width: u8,
    indent_unit: &str,
    indent_heuristic: IndentHeuristic,
) -> IndentResult {
    // Use tree-sitter when both tree and indent query are available
    if let (Some(tree), Some(query)) = (tree, indent_query) {
        return match indent_heuristic {
            IndentHeuristic::Hybrid => heuristic::compute_hybrid_indent(
                doc,
                tree,
                query,
                cursor_byte,
                tab_width,
                indent_unit,
            ),
            IndentHeuristic::TreeSitter => treesitter::compute_treesitter_indent(
                doc,
                tree,
                query,
                cursor_byte,
                new_line_below,
                tab_width,
                indent_unit,
            ),
        };
    }

    // Fallback: bracket counting
    bracket::compute_bracket_indent(doc, cursor_byte, tab_width, indent_unit)
}
