//! Custom query predicate evaluation for indent queries.
//!
//! Helix's `indents.scm` files use custom predicates beyond what tree-sitter
//! supports natively. These are evaluated after `QueryCursor::matches()`
//! returns, filtering out non-matching patterns.

/// Node position info extracted for predicate evaluation.
#[derive(Debug, Clone)]
pub struct CaptureNodeInfo {
    pub start_line: usize,
    pub end_line: usize,
    pub kind: String,
}

/// Evaluate a single predicate.
///
/// `predicate_name` is e.g. "not-kind-eq?", `args` are the capture indices
/// and string literals from the query.
pub fn evaluate_single_predicate(
    name: &str,
    args: &[PredicateArg],
    capture_nodes: &[Option<CaptureNodeInfo>],
) -> bool {
    match name {
        "not-kind-eq?" => {
            // #not-kind-eq? @capture "kind_string"
            // The captured node's kind must NOT equal the string.
            if let (Some(PredicateArg::Capture(idx)), Some(PredicateArg::String(kind))) =
                (args.first(), args.get(1))
            {
                if let Some(Some(node)) = capture_nodes.get(*idx as usize) {
                    return node.kind != *kind;
                }
            }
            true // if we can't evaluate, pass through
        }
        "same-line?" => {
            // #same-line? @a @b — both captures must start on the same line.
            if let (Some(PredicateArg::Capture(a)), Some(PredicateArg::Capture(b))) =
                (args.first(), args.get(1))
            {
                if let (Some(Some(node_a)), Some(Some(node_b))) =
                    (capture_nodes.get(*a as usize), capture_nodes.get(*b as usize))
                {
                    return node_a.start_line == node_b.start_line;
                }
            }
            true
        }
        "not-same-line?" => {
            if let (Some(PredicateArg::Capture(a)), Some(PredicateArg::Capture(b))) =
                (args.first(), args.get(1))
            {
                if let (Some(Some(node_a)), Some(Some(node_b))) =
                    (capture_nodes.get(*a as usize), capture_nodes.get(*b as usize))
                {
                    return node_a.start_line != node_b.start_line;
                }
            }
            true
        }
        "one-line?" => {
            // #one-line? @a — the captured node must span exactly one line.
            if let Some(PredicateArg::Capture(idx)) = args.first() {
                if let Some(Some(node)) = capture_nodes.get(*idx as usize) {
                    return node.start_line == node.end_line;
                }
            }
            true
        }
        "not-one-line?" => {
            if let Some(PredicateArg::Capture(idx)) = args.first() {
                if let Some(Some(node)) = capture_nodes.get(*idx as usize) {
                    return node.start_line != node.end_line;
                }
            }
            true
        }
        _ => true, // Unknown predicate — pass through
    }
}

/// Argument to a predicate: either a capture index or a string literal.
#[derive(Debug, Clone)]
pub enum PredicateArg {
    Capture(u32),
    String(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(start_line: usize, end_line: usize, kind: &str) -> Option<CaptureNodeInfo> {
        Some(CaptureNodeInfo {
            start_line,
            end_line,
            kind: kind.to_string(),
        })
    }

    #[test]
    fn test_not_kind_eq_match() {
        let nodes = vec![node(0, 5, "function_item")];
        let args = vec![
            PredicateArg::Capture(0),
            PredicateArg::String("comment".to_string()),
        ];
        assert!(evaluate_single_predicate("not-kind-eq?", &args, &nodes));
    }

    #[test]
    fn test_not_kind_eq_reject() {
        let nodes = vec![node(0, 5, "comment")];
        let args = vec![
            PredicateArg::Capture(0),
            PredicateArg::String("comment".to_string()),
        ];
        assert!(!evaluate_single_predicate("not-kind-eq?", &args, &nodes));
    }

    #[test]
    fn test_same_line() {
        let nodes = vec![node(3, 3, "a"), node(3, 5, "b")];
        let args = vec![PredicateArg::Capture(0), PredicateArg::Capture(1)];
        assert!(evaluate_single_predicate("same-line?", &args, &nodes));
    }

    #[test]
    fn test_not_same_line() {
        let nodes = vec![node(3, 3, "a"), node(5, 7, "b")];
        let args = vec![PredicateArg::Capture(0), PredicateArg::Capture(1)];
        assert!(evaluate_single_predicate("not-same-line?", &args, &nodes));
    }

    #[test]
    fn test_one_line() {
        let nodes = vec![node(3, 3, "x")];
        let args = vec![PredicateArg::Capture(0)];
        assert!(evaluate_single_predicate("one-line?", &args, &nodes));
    }

    #[test]
    fn test_not_one_line() {
        let nodes = vec![node(3, 7, "x")];
        let args = vec![PredicateArg::Capture(0)];
        assert!(evaluate_single_predicate("not-one-line?", &args, &nodes));
    }

    #[test]
    fn test_unknown_predicate_passes() {
        assert!(evaluate_single_predicate("unknown?", &[], &[]));
    }
}
