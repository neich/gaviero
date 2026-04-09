//! Extract cross-file reference edges from parsed source files.
//!
//! Uses tree-sitter AST walking to find:
//! - `use` / `import` statements → Imports edges
//! - Function call identifiers → potential Calls edges (resolved against symbol table)
//! - `impl Trait for Type` → Implements edges
//! - Test annotations → TestedBy edges

use std::collections::HashMap;

use super::store::{EdgeKind, GraphStore, NodeKind};

/// A raw reference extracted from source code, before cross-file resolution.
#[derive(Debug, Clone)]
pub struct RawReference {
    /// The identifier or path being referenced (e.g. "foo", "crate::bar::baz").
    pub target_name: String,
    /// What kind of reference this is.
    pub kind: RefKind,
    /// Line number in source where the reference appears.
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `use x::y` or `mod x` — module-level import.
    Import,
    /// `foo()` — function/method call.
    Call,
    /// `impl Trait for Type` — trait implementation.
    Implements,
}

/// Extract raw references from Rust source using tree-sitter.
pub fn extract_rust_references(source: &str) -> Vec<RawReference> {
    let language = crate::tree_sitter::language_for_extension("rs");
    let Some(language) = language else { return Vec::new() };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let bytes = source.as_bytes();
    let mut refs = Vec::new();
    collect_rust_references(tree.root_node(), bytes, &mut refs);
    refs
}

fn collect_rust_references(
    node: tree_sitter::Node,
    source: &[u8],
    out: &mut Vec<RawReference>,
) {
    match node.kind() {
        // `use foo::bar::baz;` or `use foo::bar::{baz, qux};`
        "use_declaration" => {
            if let Some(path) = extract_use_path(node, source) {
                out.push(RawReference {
                    target_name: path,
                    kind: RefKind::Import,
                    line: node.start_position().row,
                });
            }
        }

        // `impl Trait for Type { ... }`
        "impl_item" => {
            // Look for trait name in `impl <trait> for <type>`
            if let Some(trait_node) = node.child_by_field_name("trait") {
                if let Ok(trait_name) = trait_node.utf8_text(source) {
                    out.push(RawReference {
                        target_name: trait_name.to_string(),
                        kind: RefKind::Implements,
                        line: node.start_position().row,
                    });
                }
            }
        }

        // `foo(args)` — function calls
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let name = match func_node.kind() {
                    "identifier" => func_node.utf8_text(source).ok().map(|s| s.to_string()),
                    "field_expression" => {
                        // method.call() — extract the method name
                        func_node.child_by_field_name("field")
                            .and_then(|n| n.utf8_text(source).ok())
                            .map(|s| s.to_string())
                    }
                    "scoped_identifier" => {
                        // module::function() — extract the full path
                        func_node.utf8_text(source).ok().map(|s| s.to_string())
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    // Skip common stdlib calls that don't produce useful edges
                    if !is_noise_call(&name) {
                        out.push(RawReference {
                            target_name: name,
                            kind: RefKind::Call,
                            line: node.start_position().row,
                        });
                    }
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_references(child, source, out);
    }
}

/// Extract the path from a `use` declaration.
/// Handles: `use foo::bar;`, `use foo::bar as baz;`, `use foo::bar::*;`
fn extract_use_path(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // The argument child contains the path
    let arg = node.child_by_field_name("argument")
        .or_else(|| {
            // Some tree-sitter versions use a different structure
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "use_as_clause"
                    || c.kind() == "scoped_use_list"
                    || c.kind() == "use_wildcard"
                    || c.kind() == "scoped_identifier"
                    || c.kind() == "identifier")
        })?;

    // Get the text and clean it up
    let text = arg.utf8_text(source).ok()?;
    // Strip `as alias` suffix
    let path = text.split(" as ").next().unwrap_or(text);
    // Strip trailing ::* or ::{...}
    let path = path.split("::{").next().unwrap_or(path);
    let path = path.strip_suffix("::*").unwrap_or(path);
    Some(path.trim().to_string())
}

/// Filter out common stdlib/macro calls that are noise for dependency analysis.
fn is_noise_call(name: &str) -> bool {
    matches!(name,
        "println" | "eprintln" | "print" | "eprint"
        | "format" | "write" | "writeln"
        | "vec" | "assert" | "assert_eq" | "assert_ne" | "debug_assert"
        | "panic" | "unreachable" | "unimplemented" | "todo"
        | "dbg" | "cfg" | "env" | "include" | "include_str"
        | "Some" | "None" | "Ok" | "Err"
        | "Box::new" | "Arc::new" | "Rc::new"
        | "String::new" | "String::from" | "Vec::new"
        | "Default::default"
        | "Into::into" | "From::from"
        | "clone" | "to_string" | "to_owned" | "as_str" | "as_ref"
        | "unwrap" | "expect" | "unwrap_or" | "unwrap_or_default" | "unwrap_or_else"
        | "map" | "and_then" | "or_else" | "filter" | "collect" | "iter"
        | "push" | "pop" | "insert" | "remove" | "contains" | "get" | "len" | "is_empty"
    )
}

/// Resolve raw references against the graph's symbol table and insert edges.
///
/// For each reference in `file_path`, looks up the `target_name` in the graph
/// to find which file defines that symbol. If found (and it's a different file),
/// inserts an edge.
pub fn resolve_and_insert_edges(
    store: &GraphStore,
    file_path: &str,
    file_qn_prefix: &str,
    refs: &[RawReference],
) -> anyhow::Result<usize> {
    let mut edges_added = 0;

    for r in refs {
        let edge_kind = match r.kind {
            RefKind::Import => EdgeKind::Imports,
            RefKind::Call => EdgeKind::Calls,
            RefKind::Implements => EdgeKind::Implements,
        };

        // Try to resolve the target name to a qualified name in the graph
        let targets = store.resolve_symbol(&r.target_name)?;

        for (target_qn, target_file) in &targets {
            // Skip self-references (same file)
            if target_file == file_path {
                continue;
            }

            let source_qn = format!("{}::{}", file_qn_prefix, r.target_name);
            store.insert_edge(edge_kind, &source_qn, target_qn, file_path, r.line as i64)?;
            edges_added += 1;
        }
    }

    Ok(edges_added)
}

/// Detect if a file is a test file (heuristic).
pub fn is_test_file(file_path: &str) -> bool {
    file_path.contains("test")
        || file_path.contains("tests/")
        || file_path.starts_with("tests/")
        || file_path.ends_with("_test.rs")
        || file_path.ends_with("_test.py")
        || file_path.ends_with(".test.ts")
        || file_path.ends_with(".test.js")
        || file_path.ends_with("_spec.rb")
}

/// Map a tree-sitter node kind to a NodeKind for the graph.
pub fn node_kind_from_ts(ts_kind: &str, file_path: &str) -> NodeKind {
    match ts_kind {
        "function_item" | "function_definition" | "function_declaration"
        | "method_declaration" | "method_definition" => {
            if is_test_file(file_path) {
                NodeKind::Test
            } else {
                NodeKind::Function
            }
        }
        "struct_item" => NodeKind::Struct,
        "enum_item" => NodeKind::Enum,
        "trait_item" => NodeKind::Trait,
        "impl_item" => NodeKind::Impl,
        "const_item" => NodeKind::Const,
        "class_declaration" | "class_definition" => NodeKind::Class,
        "interface_declaration" => NodeKind::Interface,
        _ => NodeKind::Function,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_use_statements() {
        let src = r#"
use std::collections::HashMap;
use crate::types::FileScope;
use super::models::WorkUnit;
"#;
        let refs = extract_rust_references(src);
        let imports: Vec<&str> = refs.iter()
            .filter(|r| r.kind == RefKind::Import)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(imports.iter().any(|i| i.contains("HashMap")), "imports: {:?}", imports);
        assert!(imports.iter().any(|i| i.contains("FileScope")), "imports: {:?}", imports);
    }

    #[test]
    fn extract_function_calls() {
        let src = r#"
fn main() {
    let x = compute_value(42);
    let y = module::helper();
    x.process();
}
"#;
        let refs = extract_rust_references(src);
        let calls: Vec<&str> = refs.iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"compute_value"), "calls: {:?}", calls);
        assert!(calls.iter().any(|c| c.contains("helper")), "calls: {:?}", calls);
        assert!(calls.contains(&"process"), "calls: {:?}", calls);
    }

    #[test]
    fn extract_impl_trait() {
        let src = r#"
impl Display for MyType {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "hello")
    }
}
"#;
        let refs = extract_rust_references(src);
        let impls: Vec<&str> = refs.iter()
            .filter(|r| r.kind == RefKind::Implements)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(impls.contains(&"Display"), "impls: {:?}", impls);
    }

    #[test]
    fn noise_calls_filtered() {
        let src = r#"
fn example() {
    println!("hello");
    let v = vec![1, 2, 3];
    let s = String::new();
    let x = real_function(42);
}
"#;
        let refs = extract_rust_references(src);
        let calls: Vec<&str> = refs.iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.target_name.as_str())
            .collect();
        // real_function should be there, noise should not
        assert!(calls.contains(&"real_function"), "calls: {:?}", calls);
        assert!(!calls.contains(&"println"), "calls: {:?}", calls);
    }

    #[test]
    fn test_file_detection() {
        assert!(is_test_file("tests/auth_test.rs"));
        assert!(is_test_file("src/auth/test_session.rs"));
        assert!(is_test_file("tests/integration/api.rs"));
        assert!(!is_test_file("src/auth/session.rs"));
    }

    #[test]
    fn resolve_cross_file_edges() {
        let store = GraphStore::open_memory().unwrap();

        // File a.rs defines `compute`
        store.upsert_node(
            NodeKind::Function, "compute", "a.rs::compute", "a.rs",
            Some(1), Some(5), Some("rust"), None,
        ).unwrap();

        // File b.rs defines `caller` and calls `compute`
        store.upsert_node(
            NodeKind::Function, "caller", "b.rs::caller", "b.rs",
            Some(1), Some(10), Some("rust"), None,
        ).unwrap();

        let refs = vec![RawReference {
            target_name: "compute".into(),
            kind: RefKind::Call,
            line: 5,
        }];

        let count = resolve_and_insert_edges(&store, "b.rs", "b.rs", &refs).unwrap();
        assert_eq!(count, 1);

        // When a.rs changes, b.rs should be affected (b calls a)
        let result = store.impact_radius(&["a.rs"], 3).unwrap();
        assert!(result.affected_files.contains(&"b.rs".to_string()),
            "b.rs should be affected when a.rs changes: {:?}", result.affected_files);
    }
}
