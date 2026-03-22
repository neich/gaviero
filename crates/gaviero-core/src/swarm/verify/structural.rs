//! Structural verifier: tree-sitter AST validation of modified files.
//!
//! Catches syntactic damage — broken ASTs, orphaned symbols, malformed imports.
//! Zero LLM calls, zero subprocess spawns, runs in milliseconds.

use std::path::{Path, PathBuf};

use super::{ErrorNode, FailureSeverity, StructuralFailure, StructuralReport};
use crate::tree_sitter::{find_enclosing_node, language_for_extension, language_name_for_extension};

/// Parse all modified files with tree-sitter and report any ERROR or MISSING nodes.
///
/// Files with unknown extensions are skipped (not failures).
/// Returns a `StructuralReport` with pass/fail status per file.
pub fn verify(modified_files: &[PathBuf], workspace_root: &Path) -> StructuralReport {
    let mut files_checked = 0;
    let mut files_passed = 0;
    let mut failures = Vec::new();

    for file_path in modified_files {
        let abs_path = if file_path.is_absolute() {
            file_path.clone()
        } else {
            workspace_root.join(file_path)
        };

        // Determine language from extension
        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let Some(language) = language_for_extension(ext) else {
            // Unknown extension — skip, not a failure
            continue;
        };

        let lang_name = language_name_for_extension(ext)
            .unwrap_or("unknown")
            .to_string();

        // Read file content
        let Ok(content) = std::fs::read_to_string(&abs_path) else {
            // Can't read file — report as error
            failures.push(StructuralFailure {
                path: file_path.clone(),
                language: lang_name,
                error_nodes: vec![ErrorNode {
                    line: 0,
                    column: 0,
                    byte_range: 0..0,
                    parent_symbol: None,
                    context_snippet: "File could not be read".into(),
                }],
                severity: FailureSeverity::ParseError,
            });
            files_checked += 1;
            continue;
        };

        files_checked += 1;

        // Parse with tree-sitter
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language).is_err() {
            continue; // Language setup failed — skip
        }

        let Some(tree) = parser.parse(&content, None) else {
            failures.push(StructuralFailure {
                path: file_path.clone(),
                language: lang_name,
                error_nodes: vec![ErrorNode {
                    line: 0,
                    column: 0,
                    byte_range: 0..0,
                    parent_symbol: None,
                    context_snippet: "tree-sitter parse returned None".into(),
                }],
                severity: FailureSeverity::ParseError,
            });
            continue;
        };

        // Walk AST for ERROR and MISSING nodes
        let source = content.as_bytes();
        let lines: Vec<&str> = content.lines().collect();
        let error_nodes = collect_error_nodes(&tree, source, &lines);

        if error_nodes.is_empty() {
            files_passed += 1;
        } else {
            // Determine worst severity
            let has_error = error_nodes.iter().any(|e| {
                matches!(
                    e.severity,
                    FailureSeverity::ParseError | FailureSeverity::MissingSymbol { .. }
                )
            });
            let severity = if has_error {
                FailureSeverity::ParseError
            } else {
                FailureSeverity::MissingNode
            };

            failures.push(StructuralFailure {
                path: file_path.clone(),
                language: lang_name,
                error_nodes: error_nodes.into_iter().map(|e| e.node).collect(),
                severity,
            });
        }
    }

    StructuralReport {
        files_checked,
        files_passed,
        failures,
    }
}

/// Check if specific symbols expected by coordinator instructions exist in the AST.
///
/// Extracts symbol names from the instructions via simple pattern matching,
/// then verifies they appear as named definitions in the parsed tree.
pub fn verify_expected_symbols(
    file_path: &Path,
    content: &str,
    language: tree_sitter::Language,
    expected_symbols: &[String],
) -> Vec<ErrorNode> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };

    let source = content.as_bytes();
    let defined_names = collect_defined_names(&tree, source);

    let mut missing = Vec::new();
    for expected in expected_symbols {
        if !defined_names.contains(expected) {
            missing.push(ErrorNode {
                line: 0,
                column: 0,
                byte_range: 0..0,
                parent_symbol: None,
                context_snippet: format!(
                    "Expected symbol '{}' not found in {}",
                    expected,
                    file_path.display()
                ),
            });
        }
    }

    missing
}

struct ErrorWithSeverity {
    node: ErrorNode,
    severity: FailureSeverity,
}

/// Walk the AST and collect all ERROR and MISSING nodes.
fn collect_error_nodes(
    tree: &tree_sitter::Tree,
    source: &[u8],
    lines: &[&str],
) -> Vec<ErrorWithSeverity> {
    let mut errors = Vec::new();
    let mut cursor = tree.walk();
    walk_for_errors(&mut cursor, source, lines, &tree, &mut errors);
    errors
}

fn walk_for_errors(
    cursor: &mut tree_sitter::TreeCursor,
    source: &[u8],
    lines: &[&str],
    tree: &tree_sitter::Tree,
    errors: &mut Vec<ErrorWithSeverity>,
) {
    let node = cursor.node();
    let kind = node.kind();

    if kind == "ERROR" || node.is_error() {
        let line = node.start_position().row;
        let col = node.start_position().column;
        let snippet = context_snippet(lines, line);
        let parent_symbol = find_enclosing_node(tree, source, line)
            .and_then(|n| n.name);

        errors.push(ErrorWithSeverity {
            node: ErrorNode {
                line,
                column: col,
                byte_range: node.start_byte()..node.end_byte(),
                parent_symbol,
                context_snippet: snippet,
            },
            severity: FailureSeverity::ParseError,
        });
    } else if kind == "MISSING" || node.is_missing() {
        let line = node.start_position().row;
        let col = node.start_position().column;
        let snippet = context_snippet(lines, line);
        let parent_symbol = find_enclosing_node(tree, source, line)
            .and_then(|n| n.name);

        errors.push(ErrorWithSeverity {
            node: ErrorNode {
                line,
                column: col,
                byte_range: node.start_byte()..node.end_byte(),
                parent_symbol,
                context_snippet: snippet,
            },
            severity: FailureSeverity::MissingNode,
        });
    }

    // Recurse into children
    if cursor.goto_first_child() {
        loop {
            walk_for_errors(cursor, source, lines, tree, errors);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Extract 3 lines of context around a given line.
fn context_snippet(lines: &[&str], line: usize) -> String {
    let start = line.saturating_sub(1);
    let end = (line + 2).min(lines.len());
    lines[start..end].join("\n")
}

/// Collect all named definitions (function, struct, class, etc.) from the AST.
fn collect_defined_names(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = tree.walk();
    walk_for_names(&mut cursor, source, &mut names);
    names
}

fn walk_for_names(
    cursor: &mut tree_sitter::TreeCursor,
    source: &[u8],
    names: &mut Vec<String>,
) {
    let node = cursor.node();
    let kind = node.kind();

    // Check if this node is a definition that has a name
    const DEF_KINDS: &[&str] = &[
        "function_item", "function_definition", "function_declaration",
        "method_declaration", "method_definition",
        "class_declaration", "class_definition",
        "struct_item", "enum_item", "impl_item", "trait_item",
        "interface_declaration", "const_item",
        "object_declaration", "companion_object",
        "variable_declarator", "lexical_declaration",
    ];

    if DEF_KINDS.contains(&kind) {
        // Try to get the name
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source) {
                names.push(name.to_string());
            }
        }
    }

    if cursor.goto_first_child() {
        loop {
            walk_for_names(cursor, source, names);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Extract expected symbol names from coordinator instructions.
///
/// Looks for patterns like:
/// - "create function validate_token"
/// - "add struct AuthConfig"
/// - "implement trait Authenticator"
pub fn extract_expected_symbols(instructions: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let patterns = [
        "create function ", "add function ", "define function ",
        "create struct ", "add struct ", "define struct ",
        "create enum ", "add enum ", "define enum ",
        "create trait ", "add trait ", "define trait ",
        "implement trait ", "create class ", "add class ",
        "create method ", "add method ", "define method ",
    ];

    let lower = instructions.to_lowercase();
    for pattern in &patterns {
        for (pos, _) in lower.match_indices(pattern) {
            let after = &instructions[pos + pattern.len()..];
            // Take the next word (identifier)
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                symbols.push(name);
            }
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_verify_valid_rust_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("valid.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

        let report = verify(&[PathBuf::from("valid.rs")], dir.path());
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_passed, 1);
        assert!(report.failures.is_empty());
    }

    #[test]
    fn test_verify_syntax_error_rust() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("broken.rs");
        // Missing closing brace
        std::fs::write(&file_path, "fn main() {\n    let x = 1;\n").unwrap();

        let report = verify(&[PathBuf::from("broken.rs")], dir.path());
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_passed, 0);
        assert!(!report.failures.is_empty());
        assert_eq!(report.failures[0].language, "rust");
    }

    #[test]
    fn test_verify_unknown_extension_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.xyz");
        std::fs::write(&file_path, "some random content").unwrap();

        let report = verify(&[PathBuf::from("data.xyz")], dir.path());
        assert_eq!(report.files_checked, 0); // skipped
        assert!(report.failures.is_empty());
    }

    #[test]
    fn test_verify_multiple_files_mixed() {
        let dir = tempfile::tempdir().unwrap();

        let good = dir.path().join("good.rs");
        std::fs::write(&good, "fn foo() {}\n").unwrap();

        let bad = dir.path().join("bad.rs");
        std::fs::write(&bad, "fn bar( {\n").unwrap();

        let json_good = dir.path().join("data.json");
        std::fs::write(&json_good, "{\"key\": \"value\"}\n").unwrap();

        let report = verify(
            &[
                PathBuf::from("good.rs"),
                PathBuf::from("bad.rs"),
                PathBuf::from("data.json"),
            ],
            dir.path(),
        );
        assert_eq!(report.files_checked, 3);
        assert_eq!(report.files_passed, 2); // good.rs + data.json
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].path, PathBuf::from("bad.rs"));
    }

    #[test]
    fn test_verify_valid_python() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("valid.py");
        std::fs::write(&file_path, "def hello():\n    print('world')\n").unwrap();

        let report = verify(&[PathBuf::from("valid.py")], dir.path());
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_passed, 1);
    }

    #[test]
    fn test_verify_valid_javascript() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("app.js");
        std::fs::write(&file_path, "function hello() { return 42; }\n").unwrap();

        let report = verify(&[PathBuf::from("app.js")], dir.path());
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_passed, 1);
    }

    #[test]
    fn test_error_node_has_context() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("ctx.rs");
        std::fs::write(
            &file_path,
            "fn good() {}\nfn broken( {\n    let x = 1;\n}\n",
        )
        .unwrap();

        let report = verify(&[PathBuf::from("ctx.rs")], dir.path());
        assert!(!report.failures.is_empty());
        let failure = &report.failures[0];
        assert!(!failure.error_nodes.is_empty());
        // Error node should have a context snippet
        assert!(!failure.error_nodes[0].context_snippet.is_empty());
    }

    #[test]
    fn test_verify_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let report = verify(&[PathBuf::from("nonexistent.rs")], dir.path());
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_passed, 0);
        assert_eq!(report.failures.len(), 1);
    }

    #[test]
    fn test_extract_expected_symbols() {
        let instructions = "Create function validate_token and add struct AuthConfig. \
                           Also define trait Authenticator for the auth module.";
        let symbols = extract_expected_symbols(instructions);
        assert!(symbols.contains(&"validate_token".to_string()));
        assert!(symbols.contains(&"AuthConfig".to_string()));
        assert!(symbols.contains(&"Authenticator".to_string()));
    }

    #[test]
    fn test_extract_expected_symbols_empty() {
        let symbols = extract_expected_symbols("Just fix the bug in auth module.");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_verify_expected_symbols_found() {
        let content = "fn validate_token() {}\nstruct AuthConfig {}\n";
        let lang = language_for_extension("rs").unwrap();
        let missing = verify_expected_symbols(
            Path::new("auth.rs"),
            content,
            lang,
            &["validate_token".into(), "AuthConfig".into()],
        );
        assert!(missing.is_empty());
    }

    #[test]
    fn test_verify_expected_symbols_missing() {
        let content = "fn validate_token() {}\n";
        let lang = language_for_extension("rs").unwrap();
        let missing = verify_expected_symbols(
            Path::new("auth.rs"),
            content,
            lang,
            &["validate_token".into(), "AuthConfig".into()],
        );
        assert_eq!(missing.len(), 1);
        assert!(missing[0].context_snippet.contains("AuthConfig"));
    }
}
