//! Tree-sitter based indent computation.
//!
//! Algorithm: find deepest node at cursor → walk ancestors to root →
//! at each ancestor, check if it has indent/outdent captures → accumulate.

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, QueryPredicateArg, Tree};

use super::IndentResult;
use super::captures::{IndentCapture, IndentCaptureType, IndentScope, LineAccumulator};
use super::predicates::{CaptureNodeInfo, PredicateArg, evaluate_single_predicate};

/// Map of node ID → list of (capture type, scope, column).
pub type CaptureMap =
    std::collections::HashMap<usize, Vec<(IndentCaptureType, IndentScope, usize)>>;

/// Build the capture map for the entire tree. Call once and pass to
/// `indent_for_cursor` for each line to avoid re-running the query.
pub fn build_document_capture_map(tree: &Tree, indent_query: &Query, source: &[u8]) -> CaptureMap {
    build_capture_map(indent_query, &tree.root_node(), source)
}

/// Compute indentation using tree-sitter indent queries.
pub fn compute_treesitter_indent(
    doc: &ropey::Rope,
    tree: &Tree,
    indent_query: &Query,
    cursor_byte: usize,
    _new_line_below: bool,
    _tab_width: u8,
    indent_unit: &str,
) -> IndentResult {
    let source = doc.to_string();
    let source_bytes = source.as_bytes();
    let capture_map = build_capture_map(indent_query, &tree.root_node(), source_bytes);
    indent_for_cursor(
        doc,
        tree,
        &capture_map,
        cursor_byte,
        _tab_width,
        indent_unit,
    )
}

/// Compute indentation at a cursor position using a pre-built capture map.
pub fn indent_for_cursor(
    doc: &ropey::Rope,
    tree: &Tree,
    capture_map: &CaptureMap,
    cursor_byte: usize,
    _tab_width: u8,
    indent_unit: &str,
) -> IndentResult {
    let source = doc.to_string();
    let root = tree.root_node();

    let cursor_byte = cursor_byte.min(source.len().saturating_sub(1));
    let cursor_line = doc.byte_to_line(cursor_byte.min(doc.len_bytes().saturating_sub(1)));
    let new_line = cursor_line + 1;

    let Some(deepest) = root.descendant_for_byte_range(cursor_byte, cursor_byte) else {
        return super::bracket::compute_bracket_indent(doc, cursor_byte, _tab_width, indent_unit);
    };

    let mut accumulator = LineAccumulator::new();
    let mut node = deepest;

    loop {
        let node_id = node.id();
        let node_start_line = node.start_position().row;
        let node_end_line = node.end_position().row;

        if let Some(captures) = capture_map.get(&node_id) {
            for (capture_type, scope, column) in captures {
                if new_line < node_start_line || new_line > node_end_line {
                    continue;
                }

                let indent_capture = IndentCapture {
                    capture_type: *capture_type,
                    scope: *scope,
                    effective_line: node_start_line,
                    node_first_line: node_start_line,
                    column: *column,
                };

                accumulator.add(indent_capture, new_line);
            }
        }

        if let Some(parent) = node.parent() {
            node = parent;
        } else {
            break;
        }
    }

    let level = accumulator.compute_level();

    if let Some(align_col) = accumulator.find_alignment() {
        return IndentResult {
            whitespace: " ".repeat(align_col),
            level,
        };
    }

    let effective_level = level.max(0) as usize;
    IndentResult {
        whitespace: indent_unit.repeat(effective_level),
        level,
    }
}

/// Pre-compute which tree nodes have indent captures.
///
/// Runs the indent query once on the full tree and maps each captured
/// node ID to its capture type(s) and scope.
fn build_capture_map(
    query: &Query,
    root: &Node,
    source: &[u8],
) -> std::collections::HashMap<usize, Vec<(IndentCaptureType, IndentScope, usize)>> {
    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut map: std::collections::HashMap<usize, Vec<(IndentCaptureType, IndentScope, usize)>> =
        std::collections::HashMap::new();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, *root, source);

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        let pattern_idx = m.pattern_index;
        let scope_override = read_scope_property(query, pattern_idx);

        // Evaluate predicates
        let predicates = query.general_predicates(pattern_idx);
        if !predicates.is_empty() {
            let max_cap = m.captures.iter().map(|c| c.index).max().unwrap_or(0) as usize;
            let mut nodes: Vec<Option<CaptureNodeInfo>> = vec![None; max_cap + 1];
            for cap in m.captures {
                nodes[cap.index as usize] = Some(CaptureNodeInfo {
                    start_line: cap.node.start_position().row,
                    end_line: cap.node.end_position().row,
                    kind: cap.node.kind().to_string(),
                });
            }
            let mut pass = true;
            for pred in predicates {
                let args: Vec<PredicateArg> = pred
                    .args
                    .iter()
                    .map(|a| match a {
                        QueryPredicateArg::Capture(i) => PredicateArg::Capture(*i),
                        QueryPredicateArg::String(s) => PredicateArg::String(s.to_string()),
                    })
                    .collect();
                if !evaluate_single_predicate(&pred.operator, &args, &nodes) {
                    pass = false;
                    break;
                }
            }
            if !pass {
                continue;
            }
        }

        for cap in m.captures {
            let name = &capture_names[cap.index as usize];
            let Some(ct) = IndentCaptureType::from_name(name) else {
                continue;
            };
            let scope = scope_override.unwrap_or_else(|| ct.default_scope());
            let col = cap.node.start_position().column;
            map.entry(cap.node.id()).or_default().push((ct, scope, col));
        }
    }

    map
}

fn read_scope_property(query: &Query, pattern_index: usize) -> Option<IndentScope> {
    for prop in query.property_settings(pattern_index) {
        if &*prop.key == "scope" {
            if let Some(ref val) = prop.value {
                return match &**val {
                    "all" => Some(IndentScope::All),
                    "tail" => Some(IndentScope::Tail),
                    _ => None,
                };
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indent_at(source: &str, cursor_line: usize, ext: &str) -> IndentResult {
        let lang = crate::tree_sitter::language_for_extension(ext).unwrap();
        let lang_name = crate::tree_sitter::language_name_for_extension(ext).unwrap();

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let mut cache = super::super::config::IndentQueryCache::new();
        let query = cache.get_or_load(lang_name, &lang).unwrap();

        let rope = ropey::Rope::from_str(source);
        let line_text: String = rope.line(cursor_line).into();
        let trimmed_len = line_text.trim_end_matches('\n').len();
        let cursor_byte = rope.line_to_byte(cursor_line) + trimmed_len;

        compute_treesitter_indent(&rope, &tree, &query, cursor_byte, true, 4, "    ")
    }

    // ── Rust ────────────────────────────────────────────────────

    #[test]
    fn test_rust_after_fn_brace() {
        let src = "fn main() {\n    let x = 1;\n}\n";
        assert_eq!(indent_at(src, 0, "rs").level, 1);
    }

    #[test]
    fn test_rust_inside_fn() {
        let src = "fn main() {\n    let x = 1;\n}\n";
        assert_eq!(indent_at(src, 1, "rs").level, 1);
    }

    #[test]
    fn test_rust_closing_brace() {
        let src = "fn main() {\n    let x = 1;\n}\n";
        assert_eq!(indent_at(src, 2, "rs").level, 0);
    }

    #[test]
    fn test_rust_nested() {
        let src = "fn main() {\n    if true {\n        let x = 1;\n    }\n}\n";
        assert_eq!(indent_at(src, 1, "rs").level, 2);
    }

    #[test]
    fn test_rust_top_level() {
        let src = "use std::io;\n\nfn main() {\n}\n";
        assert_eq!(indent_at(src, 0, "rs").level, 0);
    }

    #[test]
    fn test_rust_match_block() {
        let src = "fn f() {\n    match x {\n        1 => {},\n    }\n}\n";
        assert_eq!(indent_at(src, 1, "rs").level, 2);
    }

    // ── JSON ────────────────────────────────────────────────────

    #[test]
    fn test_json_object() {
        let src = "{\n    \"key\": \"value\"\n}\n";
        assert_eq!(indent_at(src, 0, "json").level, 1);
    }

    #[test]
    fn test_json_nested() {
        let src = "{\n    \"obj\": {\n        \"inner\": 1\n    }\n}\n";
        assert_eq!(indent_at(src, 1, "json").level, 2);
    }

    // ── C ───────────────────────────────────────────────────────

    #[test]
    fn test_c_function() {
        let src = "int main() {\n    return 0;\n}\n";
        assert_eq!(indent_at(src, 0, "c").level, 1);
    }
}
