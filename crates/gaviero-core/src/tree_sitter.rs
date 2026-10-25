use crate::types::{DiffHunk, HunkStatus, HunkType, NodeInfo, StructuralHunk};

// ── Language registry ──────────────────────────────────────────
//
// Single source of truth for extension → (name, grammar) mapping.
// Each entry: (extensions, canonical name, grammar constructor or None).

type GrammarFn = fn() -> tree_sitter::Language;

const LANGUAGE_REGISTRY: &[(&[&str], &str, Option<GrammarFn>)] = &[
    (&["rs"], "rust", Some(|| tree_sitter_rust::LANGUAGE.into())),
    (
        &["java"],
        "java",
        Some(|| tree_sitter_java::LANGUAGE.into()),
    ),
    (
        &["js", "mjs", "cjs"],
        "javascript",
        Some(|| tree_sitter_javascript::LANGUAGE.into()),
    ),
    (
        &["ts", "tsx"],
        "typescript",
        Some(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
    ),
    (
        &["html", "htm"],
        "html",
        Some(|| tree_sitter_html::LANGUAGE.into()),
    ),
    (&["css"], "css", Some(|| tree_sitter_css::LANGUAGE.into())),
    (
        // `gaviero-workspace` is JSON content with a project-specific
        // extension (multi-folder workspace files; see `Workspace::load`
        // in `gaviero-core::workspace`). Aliasing it to the JSON grammar
        // gives editors syntax highlighting, indentation, and structural
        // validation for free — same as opening `*.json` directly.
        //
        // `jsonl` (JSON Lines) is a sequence of standalone JSON values, one
        // per line. The grammar's `document` rule is `repeat($._value)`, so
        // multi-value files parse cleanly under the JSON grammar — no ERROR
        // nodes — making the alias sound for highlighting and validation.
        &["json", "jsonl", "gaviero-workspace"],
        "json",
        Some(|| tree_sitter_json::LANGUAGE.into()),
    ),
    (
        &["sh", "bash"],
        "bash",
        Some(|| tree_sitter_bash::LANGUAGE.into()),
    ),
    (
        &["toml"],
        "toml",
        Some(|| tree_sitter_toml_ng::LANGUAGE.into()),
    ),
    (&["c", "h"], "c", Some(|| tree_sitter_c::LANGUAGE.into())),
    (
        &["cpp", "hpp", "cc", "cxx"],
        "cpp",
        Some(|| tree_sitter_cpp::LANGUAGE.into()),
    ),
    (
        &["tex", "sty", "cls", "bib"],
        "latex",
        Some(|| codebook_tree_sitter_latex::LANGUAGE.into()),
    ),
    (
        &["py", "pyi"],
        "python",
        Some(|| tree_sitter_python::LANGUAGE.into()),
    ),
    (
        &["yml", "yaml"],
        "yaml",
        Some(|| tree_sitter_yaml::LANGUAGE.into()),
    ),
    (
        &["kt", "kts"],
        "kotlin",
        Some(|| tree_sitter_kotlin_ng::LANGUAGE.into()),
    ),
    (
        &["gaviero"],
        "gaviero",
        Some(|| tree_sitter_gaviero::LANGUAGE.into()),
    ),
    (&["md", "markdown"], "markdown", None),
];

fn lookup_extension(ext: &str) -> Option<(&'static str, Option<GrammarFn>)> {
    LANGUAGE_REGISTRY
        .iter()
        .find(|(exts, _, _)| exts.contains(&ext))
        .map(|(_, name, grammar)| (*name, *grammar))
}

/// Return the tree-sitter Language for a given file extension.
pub fn language_for_extension(ext: &str) -> Option<tree_sitter::Language> {
    lookup_extension(ext).and_then(|(_, grammar)| grammar.map(|f| f()))
}

/// Return the language name string for a given file extension.
pub fn language_name_for_extension(ext: &str) -> Option<&'static str> {
    lookup_extension(ext).map(|(name, _)| name)
}

// ── Syntax errors ───────────────────────────────────────────────

/// What kind of problem tree-sitter reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxErrorKind {
    /// Input the grammar could not place (`ERROR` node).
    Unexpected,
    /// A token the grammar expected but the source does not contain
    /// (`MISSING` node).
    Missing,
}

impl SyntaxErrorKind {
    /// Short human-readable label for a dialog or status line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unexpected => "unexpected input",
            Self::Missing => "missing token",
        }
    }
}

/// One syntax problem found in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Zero-based line of the problem.
    pub line: usize,
    /// Zero-based byte column of the problem.
    pub column: usize,
    pub kind: SyntaxErrorKind,
    /// The tree-sitter node kind. For [`SyntaxErrorKind::Missing`] this is the
    /// token the grammar expected (for example `}` or `identifier`).
    pub node_kind: String,
    /// The offending source line, trimmed, for display.
    pub line_text: String,
}

/// Parse `content` with the grammar selected by `ext` and report every syntax
/// error the parser can see.
///
/// The extension registry is the single source of truth, so every language
/// gaviero can highlight is also checked here. Returns [`None`] when the
/// extension has no grammar (Markdown, unknown files) — gaviero cannot decide
/// whether those are syntactically valid. `Some(vec![])` means the file parsed
/// cleanly.
pub fn syntax_errors(content: &str, ext: &str) -> Option<Vec<SyntaxError>> {
    let language = language_for_extension(ext)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(content, None)?;

    let lines: Vec<&str> = content.lines().collect();
    let mut errors = Vec::new();
    collect_syntax_errors(tree.root_node(), &lines, &mut errors);
    Some(errors)
}

/// Walk the tree collecting `ERROR` / `MISSING` nodes. An `ERROR` node is
/// recorded but not descended into: it already covers its whole subtree, and
/// descending would report the same broken input several times.
fn collect_syntax_errors(
    node: tree_sitter::Node<'_>,
    lines: &[&str],
    out: &mut Vec<SyntaxError>,
) {
    if node.is_error() || node.is_missing() {
        let pos = node.start_position();
        out.push(SyntaxError {
            line: pos.row,
            column: pos.column,
            kind: if node.is_missing() {
                SyntaxErrorKind::Missing
            } else {
                SyntaxErrorKind::Unexpected
            },
            node_kind: node.kind().to_string(),
            line_text: lines
                .get(pos.row)
                .map(|line| line.trim().to_string())
                .unwrap_or_default(),
        });
        if node.is_error() {
            return;
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_syntax_errors(child, lines, out);
        }
    }
}

// ── Structural enrichment ────────────────────────────────────────

/// Node kinds that represent meaningful enclosing structures.
const ENCLOSING_NODE_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_declaration",
    "method_definition",
    "class_declaration",
    "class_definition",
    "struct_item",
    "enum_item",
    "impl_item",
    "trait_item",
    "interface_declaration",
    "module",
    "mod_item",
    "const_item",
    "object_declaration",
    "companion_object",
];

/// Enrich diff hunks with structural (AST) context from the original file.
/// Parses the original content with tree-sitter and finds the enclosing
/// function/class/struct for each hunk.
pub fn enrich_hunks(
    hunks: Vec<DiffHunk>,
    original: &str,
    language: tree_sitter::Language,
) -> Vec<StructuralHunk> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        // Can't parse — return hunks without structural info
        return hunks
            .into_iter()
            .map(|h| {
                let desc = describe_hunk(&h, None);
                StructuralHunk {
                    diff_hunk: h,
                    enclosing_node: None,
                    description: desc,
                    status: HunkStatus::Pending,
                }
            })
            .collect();
    }

    let tree = match parser.parse(original, None) {
        Some(t) => t,
        None => {
            return hunks
                .into_iter()
                .map(|h| {
                    let desc = describe_hunk(&h, None);
                    StructuralHunk {
                        diff_hunk: h,
                        enclosing_node: None,
                        description: desc,
                        status: HunkStatus::Pending,
                    }
                })
                .collect();
        }
    };

    let source_bytes = original.as_bytes();

    hunks
        .into_iter()
        .map(|hunk| {
            let enclosing = find_enclosing_node(&tree, source_bytes, hunk.original_range.0);
            let desc = describe_hunk(&hunk, enclosing.as_ref());
            StructuralHunk {
                diff_hunk: hunk,
                enclosing_node: enclosing,
                description: desc,
                status: HunkStatus::Pending,
            }
        })
        .collect()
}

/// Walk up the tree from a given line to find the nearest enclosing named node
/// (function, class, struct, enum, trait, impl, method).
///
/// Used by: `enrich_hunks()`, `StructuralVerifier` (verification pipeline).
pub fn find_enclosing_node(
    tree: &tree_sitter::Tree,
    source: &[u8],
    line: usize,
) -> Option<NodeInfo> {
    let root = tree.root_node();

    // Find the deepest node at this line
    let point = tree_sitter::Point::new(line, 0);
    let mut node = root.descendant_for_point_range(point, point)?;

    loop {
        let kind = node.kind();
        if ENCLOSING_NODE_KINDS.contains(&kind) {
            let name = extract_node_name(&node, source);
            return Some(NodeInfo {
                kind: kind.to_string(),
                name,
                range: (node.start_position().row, node.end_position().row),
            });
        }
        node = node.parent()?;
    }
}

/// Try to extract a name from a named node (e.g., function name, class name).
fn extract_node_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Most tree-sitter grammars use a child named "name" for the identifier
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).ok().map(|s| s.to_string());
    }
    // Fallback: look for the first identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            return child.utf8_text(source).ok().map(|s| s.to_string());
        }
    }
    None
}

/// Generate a human-readable description of a hunk.
fn describe_hunk(hunk: &DiffHunk, enclosing: Option<&NodeInfo>) -> String {
    let action = match hunk.hunk_type {
        HunkType::Added => "Add",
        HunkType::Removed => "Remove",
        HunkType::Modified => "Modify",
    };

    let line_range = if hunk.original_range.0 == hunk.original_range.1.saturating_sub(1) {
        format!("line {}", hunk.original_range.0 + 1)
    } else {
        format!(
            "lines {}-{}",
            hunk.original_range.0 + 1,
            hunk.original_range.1
        )
    };

    match enclosing {
        Some(node) => {
            let name = node.name.as_deref().unwrap_or("<anonymous>");
            format!(
                "{} {} in {} `{}`",
                action,
                line_range,
                node.kind.replace('_', " "),
                name
            )
        }
        None => format!("{} {}", action, line_range),
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_extensions() {
        assert!(language_for_extension("rs").is_some());
        assert!(language_for_extension("java").is_some());
        assert!(language_for_extension("js").is_some());
        assert!(language_for_extension("ts").is_some());
        assert!(language_for_extension("html").is_some());
        assert!(language_for_extension("css").is_some());
        assert!(language_for_extension("json").is_some());
        assert!(language_for_extension("toml").is_some());
        assert!(language_for_extension("c").is_some());
        assert!(language_for_extension("cpp").is_some());
        assert!(language_for_extension("py").is_some());
    }

    #[test]
    fn test_unknown_extension() {
        assert!(language_for_extension("xyz").is_none());
        assert!(language_for_extension("").is_none());
    }

    #[test]
    fn gaviero_workspace_extension_aliases_json() {
        // Multi-folder workspace files use `.gaviero-workspace`; they're
        // JSON in disguise. The language registry must route them through
        // the JSON grammar so highlighting + structural validation work.
        assert!(language_for_extension("gaviero-workspace").is_some());
        assert_eq!(
            language_name_for_extension("gaviero-workspace"),
            Some("json")
        );
    }

    #[test]
    fn jsonl_extension_aliases_json() {
        // JSON Lines files (`*.jsonl`) are aliased to the JSON grammar so
        // highlighting, indentation, and structural validation work the same
        // as `*.json`. The grammar accepts multiple top-level values, so
        // line-delimited JSON parses without ERROR nodes.
        assert!(language_for_extension("jsonl").is_some());
        assert_eq!(language_name_for_extension("jsonl"), Some("json"));
    }

    #[test]
    fn test_language_names() {
        assert_eq!(language_name_for_extension("rs"), Some("rust"));
        assert_eq!(language_name_for_extension("ts"), Some("typescript"));
        assert_eq!(language_name_for_extension("xyz"), None);
    }

    #[test]
    fn syntax_errors_are_none_for_languages_without_a_grammar() {
        // Markdown is registered for highlighting but has no grammar, so
        // gaviero cannot decide whether it is syntactically valid.
        assert!(syntax_errors("# heading\n\n- item", "md").is_none());
        assert!(syntax_errors("anything", "xyz").is_none());
    }

    #[test]
    fn clean_source_reports_no_errors() {
        let errs = syntax_errors("fn main() {\n    println!(\"hi\");\n}\n", "rs")
            .expect("rust has a grammar");
        assert!(errs.is_empty(), "valid Rust must parse cleanly: {errs:?}");
    }

    #[test]
    fn missing_closing_brace_is_reported_with_a_line() {
        let source = "fn main() {\n    let x = 1;\n";
        let errs = syntax_errors(source, "rs").expect("rust has a grammar");
        assert!(!errs.is_empty(), "unclosed fn must be rejected");
        assert!(
            errs.iter().all(|e| e.line <= 2),
            "the error points inside the broken block: {errs:?}"
        );
        assert!(
            errs.iter().all(|e| !e.line_text.is_empty()),
            "line text comes from the source: {errs:?}"
        );
    }

    #[test]
    fn every_grammar_backed_extension_is_checked() {
        // Regression guard for "all languages gaviero can determine": every
        // registry entry that names a grammar must produce a verdict (even if
        // a permissive grammar finds no error in an empty snippet, the API
        // must not return None).
        for (exts, _, grammar) in LANGUAGE_REGISTRY {
            if grammar.is_none() {
                continue;
            }
            for ext in *exts {
                assert!(
                    syntax_errors("", ext).is_some(),
                    "{ext} has a grammar but syntax_errors returned None"
                );
            }
        }
    }

    #[test]
    fn syntax_errors_are_reported_for_non_rust_languages() {
        let json = syntax_errors("{\n  \"a\": 1,\n", "json").expect("json has a grammar");
        assert!(!json.is_empty(), "trailing open object must be rejected");

        let python = syntax_errors("def f(:\n    pass\n", "py").expect("python has a grammar");
        assert!(!python.is_empty(), "broken def must be rejected");

        let javascript =
            syntax_errors("function f( {\n", "js").expect("javascript has a grammar");
        assert!(
            !javascript.is_empty(),
            "unclosed parameter list must be rejected"
        );
    }

    #[test]
    fn test_parse_rust_source() {
        let lang = language_for_extension("rs").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "fn main() { println!(\"hello\"); }";
        let tree = parser.parse(source, None).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_enrich_hunks_finds_enclosing_function() {
        let original =
            "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n\nfn bar() {\n    let z = 3;\n}\n";
        let lang = language_for_extension("rs").unwrap();

        let hunk = DiffHunk {
            original_range: (1, 2),
            proposed_range: (1, 2),
            original_text: "    let x = 1;\n".into(),
            proposed_text: "    let x = 42;\n".into(),
            hunk_type: HunkType::Modified,
        };

        let enriched = enrich_hunks(vec![hunk], original, lang);
        assert_eq!(enriched.len(), 1);
        assert!(enriched[0].enclosing_node.is_some());
        let node = enriched[0].enclosing_node.as_ref().unwrap();
        assert_eq!(node.kind, "function_item");
        assert_eq!(node.name.as_deref(), Some("foo"));
        assert_eq!(enriched[0].status, HunkStatus::Pending);
        assert!(enriched[0].description.contains("foo"));
    }

    #[test]
    fn test_enrich_hunks_no_enclosing_node() {
        let original = "use std::io;\n\nfn main() {}\n";
        let lang = language_for_extension("rs").unwrap();

        let hunk = DiffHunk {
            original_range: (0, 1),
            proposed_range: (0, 1),
            original_text: "use std::io;\n".into(),
            proposed_text: "use std::fs;\n".into(),
            hunk_type: HunkType::Modified,
        };

        let enriched = enrich_hunks(vec![hunk], original, lang);
        assert_eq!(enriched.len(), 1);
        // Top-level use statement has no enclosing function/struct
        // It may or may not have an enclosing node depending on grammar
    }
}
