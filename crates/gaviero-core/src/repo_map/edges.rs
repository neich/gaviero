//! Extract cross-file reference edges from parsed source files.
//!
//! Uses tree-sitter AST walking to find:
//! - `use` / `import` statements → Imports edges
//! - Function call identifiers → potential Calls edges (resolved against symbol table)
//! - `impl Trait for Type` → Implements edges
//! - Test files/calls → TestOf edges

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

/// Extract raw references from a supported source extension.
///
/// Resolution order:
/// 1. **Query-driven** (`.scm`) extraction when an `edges.scm` is bundled
///    or installed for the language ([`extract_via_edges_query`]).
///    Available today for Rust, TypeScript / TSX, and Python.
/// 2. **Hand-rolled Rust extractor** for the legacy code path —
///    redundant once the query path is shown to be byte-identical, but
///    kept so a regression in the `.scm` file silently degrades to the
///    proven walker rather than producing zero edges.
/// 3. **Generic AST walker** for the remaining tree-sitter languages.
///
/// Go is intentionally not in the priority set: its grammar isn't
/// registered in the workspace yet. Once `tree-sitter-go` lands, drop
/// in `queries/go/edges.scm` and it will be picked up automatically.
pub fn extract_references_for_extension(ext: &str, source: &str) -> Vec<RawReference> {
    if let Some(refs) = extract_via_edges_query(ext, source) {
        return refs;
    }
    match ext {
        "rs" => extract_rust_references(source),
        _ => extract_generic_references(ext, source),
    }
}

/// Map a file extension to the language directory under `queries/`.
fn query_language_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "py" => Some("python"),
        _ => None,
    }
}

/// C4 G4: query-driven extractor. Returns `Some(refs)` when an
/// `edges.scm` for the language is found and parses cleanly; `None`
/// triggers the AST-walker fallback.
///
/// Failure modes are deliberately distinguished:
/// - "no edges.scm bundled / on disk" is the normal fallback path (no log)
/// - a found-but-malformed `edges.scm` logs a `warn!` so a regression
///   in the query file doesn't silently degrade to zero edges
/// - tree-sitter parse failure on the source logs a `warn!`
fn extract_via_edges_query(ext: &str, source: &str) -> Option<Vec<RawReference>> {
    let lang_name = query_language_for_extension(ext)?;
    let query_src = crate::query_loader::find_query_file(lang_name, "edges.scm", |_, _| {
        Err(anyhow::anyhow!("no bundled edges query"))
    })
    .ok()?;
    let language = crate::tree_sitter::language_for_extension(ext)?;
    let query = match tree_sitter::Query::new(&language, &query_src) {
        Ok(q) => q,
        Err(e) => {
            tracing::warn!(
                target: "repo_map_edges",
                ext,
                lang = lang_name,
                error = %e,
                "edges.scm failed to parse; falling back to AST walker"
            );
            return None;
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&language) {
        tracing::warn!(
            target: "repo_map_edges",
            ext,
            error = %e,
            "query-path parser language assignment failed; falling back"
        );
        return None;
    }
    let Some(tree) = parser.parse(source, None) else {
        tracing::warn!(
            target: "repo_map_edges",
            ext,
            source_bytes = source.len(),
            "query-path parse returned no tree; falling back"
        );
        return None;
    };
    let bytes = source.as_bytes();

    use streaming_iterator::StreamingIterator;
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    let mut refs = Vec::new();
    while let Some(m) = matches.next() {
        let mut kind: Option<RefKind> = None;
        let mut detail_node: Option<tree_sitter::Node> = None;
        let mut head_node: Option<tree_sitter::Node> = None;

        for cap in m.captures {
            let name = &query.capture_names()[cap.index as usize];
            let (k, is_detail) = classify_capture(name);
            if let Some(k) = k {
                if kind.is_none() {
                    kind = Some(k);
                }
                if is_detail {
                    detail_node = Some(cap.node);
                } else {
                    head_node = Some(cap.node);
                }
            }
        }

        let (Some(kind), Some(node)) = (kind, detail_node.or(head_node)) else {
            continue;
        };
        let target = match kind {
            RefKind::Import => node
                .utf8_text(bytes)
                .ok()
                .and_then(import_target_name)
                .or_else(|| node.utf8_text(bytes).ok().map(str::to_string)),
            RefKind::Call | RefKind::Implements => node.utf8_text(bytes).ok().map(|s| {
                last_path_segment(s.trim())
                    .map(str::to_string)
                    .unwrap_or_else(|| s.trim().to_string())
            }),
        };
        let Some(target) = target else { continue };
        let target = target.trim().trim_matches(|c: char| c == '"' || c == '\'').to_string();
        if target.is_empty() {
            continue;
        }
        if matches!(kind, RefKind::Call) && is_noise_call(&target) {
            continue;
        }
        refs.push(RawReference {
            target_name: target,
            kind,
            line: head_node.unwrap_or(node).start_position().row,
        });
    }
    Some(refs)
}

/// Map a `@capture` name from `edges.scm` to a [`RefKind`] plus whether
/// the capture identifies the *detail* (target name) vs the *head*
/// (source location).
fn classify_capture(name: &str) -> (Option<RefKind>, bool) {
    if let Some(rest) = name.strip_prefix("import") {
        return (Some(RefKind::Import), !rest.is_empty());
    }
    if let Some(rest) = name.strip_prefix("call") {
        return (Some(RefKind::Call), !rest.is_empty());
    }
    if let Some(rest) = name.strip_prefix("implements") {
        return (Some(RefKind::Implements), !rest.is_empty());
    }
    (None, false)
}

/// Extract raw references from Rust source using tree-sitter.
pub fn extract_rust_references(source: &str) -> Vec<RawReference> {
    let language = crate::tree_sitter::language_for_extension("rs");
    let Some(language) = language else {
        // Should never happen — Rust grammar is statically registered.
        // Log loudly because zero edges from a Rust file is otherwise
        // indistinguishable from "the file legitimately has no refs".
        tracing::warn!(target: "repo_map_edges", ext = "rs", "rust grammar unavailable; emitting zero edges");
        return Vec::new();
    };

    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&language) {
        tracing::warn!(target: "repo_map_edges", ext = "rs", error = %e, "rust parser language assignment failed");
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        tracing::warn!(target: "repo_map_edges", ext = "rs", source_bytes = source.len(), "rust parse returned no tree");
        return Vec::new();
    };

    let bytes = source.as_bytes();
    let mut refs = Vec::new();
    collect_rust_references(tree.root_node(), bytes, &mut refs);
    refs
}

fn extract_generic_references(ext: &str, source: &str) -> Vec<RawReference> {
    let Some(language) = crate::tree_sitter::language_for_extension(ext) else {
        // Common case for unsupported extensions; debug-only so build
        // logs aren't spammed by every text/binary/markup file.
        tracing::debug!(target: "repo_map_edges", ext, "no tree-sitter grammar; skipping edge extraction");
        return Vec::new();
    };

    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&language) {
        tracing::warn!(target: "repo_map_edges", ext, error = %e, "parser language assignment failed");
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        tracing::warn!(target: "repo_map_edges", ext, source_bytes = source.len(), "parse returned no tree (likely malformed source)");
        return Vec::new();
    };

    let bytes = source.as_bytes();
    let mut refs = Vec::new();
    collect_generic_references(tree.root_node(), bytes, &mut refs);
    refs
}

fn collect_generic_references(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawReference>) {
    match node.kind() {
        "import_statement" | "import_from_statement" | "import_declaration" => {
            if let Ok(text) = node.utf8_text(source)
                && let Some(name) = import_target_name(text)
            {
                out.push(RawReference {
                    target_name: name,
                    kind: RefKind::Import,
                    line: node.start_position().row,
                });
            }
        }
        "call_expression" | "call" => {
            if let Some(func_node) = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))
                && let Some(name) = terminal_identifier(func_node, source)
                && !is_noise_call(&name)
            {
                out.push(RawReference {
                    target_name: name,
                    kind: RefKind::Call,
                    line: node.start_position().row,
                });
            }
        }
        "class_declaration" => {
            for field in ["superclass", "extends", "base_class"] {
                if let Some(parent) = node.child_by_field_name(field)
                    && let Some(name) = terminal_identifier(parent, source)
                {
                    out.push(RawReference {
                        target_name: name,
                        kind: RefKind::Implements,
                        line: node.start_position().row,
                    });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_generic_references(child, source, out);
    }
}

fn import_target_name(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_end_matches(';')
        .trim_matches('"')
        .trim_matches('\'');

    if let Some(rest) = cleaned.strip_prefix("from ") {
        return rest
            .split_whitespace()
            .next()
            .and_then(last_path_segment)
            .map(str::to_string);
    }
    if let Some(rest) = cleaned.strip_prefix("import ") {
        let candidate = rest
            .split_whitespace()
            .last()
            .unwrap_or(rest)
            .trim_matches('"')
            .trim_matches('\'');
        return last_path_segment(candidate).map(str::to_string);
    }
    if cleaned.starts_with("import") && cleaned.contains(" from ") {
        let candidate = cleaned
            .split(" from ")
            .nth(1)
            .unwrap_or("")
            .trim_matches('"')
            .trim_matches('\'');
        return last_path_segment(candidate).map(str::to_string);
    }
    None
}

fn last_path_segment(text: &str) -> Option<&str> {
    text.rsplit([':', '/', '.', '\\']).find(|s| !s.is_empty())
}

fn terminal_identifier(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier"
        | "property_identifier"
        | "field_identifier"
        | "type_identifier"
        | "attribute" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "member_expression" | "field_expression" | "attribute_expression" => node
            .child_by_field_name("property")
            .or_else(|| node.child_by_field_name("field"))
            .or_else(|| node.child_by_field_name("attribute"))
            .and_then(|n| terminal_identifier(n, source)),
        "scoped_identifier" | "qualified_name" | "dotted_name" => node
            .utf8_text(source)
            .ok()
            .and_then(last_path_segment)
            .map(str::to_string),
        _ => {
            let mut cursor = node.walk();
            let mut last = None;
            for child in node.children(&mut cursor) {
                if let Some(name) = terminal_identifier(child, source) {
                    last = Some(name);
                }
            }
            last
        }
    }
}

fn collect_rust_references(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawReference>) {
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
                        func_node
                            .child_by_field_name("field")
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
    let arg = node.child_by_field_name("argument").or_else(|| {
        // Some tree-sitter versions use a different structure
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|c| {
            c.kind() == "use_as_clause"
                || c.kind() == "scoped_use_list"
                || c.kind() == "use_wildcard"
                || c.kind() == "scoped_identifier"
                || c.kind() == "identifier"
        })
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
    matches!(
        name,
        "println"
            | "eprintln"
            | "print"
            | "eprint"
            | "format"
            | "write"
            | "writeln"
            | "vec"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "panic"
            | "unreachable"
            | "unimplemented"
            | "todo"
            | "dbg"
            | "cfg"
            | "env"
            | "include"
            | "include_str"
            | "Some"
            | "None"
            | "Ok"
            | "Err"
            | "Box::new"
            | "Arc::new"
            | "Rc::new"
            | "String::new"
            | "String::from"
            | "Vec::new"
            | "Default::default"
            | "Into::into"
            | "From::from"
            | "clone"
            | "to_string"
            | "to_owned"
            | "as_str"
            | "as_ref"
            | "unwrap"
            | "expect"
            | "unwrap_or"
            | "unwrap_or_default"
            | "unwrap_or_else"
            | "map"
            | "and_then"
            | "or_else"
            | "filter"
            | "collect"
            | "iter"
            | "push"
            | "pop"
            | "insert"
            | "remove"
            | "contains"
            | "get"
            | "len"
            | "is_empty"
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
    let source_is_test = is_test_file(file_path);
    // Memoize `is_test_file` per unique target path. `resolve_symbol`
    // may return the same target file across many references, and
    // `is_test_file` walks each path component every call. Cheap
    // individually, but for a 1000-reference file this avoids
    // thousands of redundant scans.
    let mut target_is_test_cache: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

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

            // When a test file references a non-test file, also emit a
            // TestOf edge so `mode=tests` blast-radius queries can find
            // the test from the symbol-under-test. The original typed
            // edge (Calls/Imports/Implements) is kept so the other modes
            // remain accurate.
            if source_is_test {
                let target_is_test = *target_is_test_cache
                    .entry(target_file.clone())
                    .or_insert_with(|| is_test_file(target_file));
                if !target_is_test {
                    store.insert_edge(
                        EdgeKind::TestOf,
                        &source_qn,
                        target_qn,
                        file_path,
                        r.line as i64,
                    )?;
                    edges_added += 1;
                }
            }
        }
    }

    Ok(edges_added)
}

/// Detect if a file is a test file. Directory- and suffix-aware so common
/// substrings like `latest`, `attest`, `contest`, or a `test_utils` helper
/// module that isn't itself a test don't trip the classifier.
pub fn is_test_file(file_path: &str) -> bool {
    if file_path.ends_with("_test.rs")
        || file_path.ends_with("_test.go")
        || file_path.ends_with("_test.py")
        || file_path.ends_with(".test.ts")
        || file_path.ends_with(".test.tsx")
        || file_path.ends_with(".test.js")
        || file_path.ends_with(".test.jsx")
        || file_path.ends_with(".spec.ts")
        || file_path.ends_with(".spec.js")
        || file_path.ends_with("_spec.rb")
    {
        return true;
    }

    // Directory-aware: a path component literally named `tests`, `test`,
    // `__tests__`, or `spec` (any path separator) makes this a test path.
    file_path
        .split(|c| c == '/' || c == '\\')
        .any(|seg| matches!(seg, "tests" | "test" | "__tests__" | "spec"))
}

/// Map a tree-sitter node kind to a NodeKind for the graph.
pub fn node_kind_from_ts(ts_kind: &str, file_path: &str) -> NodeKind {
    match ts_kind {
        "function_item"
        | "function_definition"
        | "function_declaration"
        | "method_declaration"
        | "method_definition" => {
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
    use super::super::store::BlastRadiusMode;

    #[test]
    fn extract_use_statements() {
        let src = r#"
use std::collections::HashMap;
use crate::types::FileScope;
use super::models::WorkUnit;
"#;
        let refs = extract_rust_references(src);
        let imports: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Import)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            imports.iter().any(|i| i.contains("HashMap")),
            "imports: {:?}",
            imports
        );
        assert!(
            imports.iter().any(|i| i.contains("FileScope")),
            "imports: {:?}",
            imports
        );
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
        let calls: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"compute_value"), "calls: {:?}", calls);
        assert!(
            calls.iter().any(|c| c.contains("helper")),
            "calls: {:?}",
            calls
        );
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
        let impls: Vec<&str> = refs
            .iter()
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
        let calls: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.target_name.as_str())
            .collect();
        // real_function should be there, noise should not
        assert!(calls.contains(&"real_function"), "calls: {:?}", calls);
        assert!(!calls.contains(&"println"), "calls: {:?}", calls);
    }

    #[test]
    fn extract_typescript_references() {
        let src = r#"
import { compute } from './math';
function run() {
    return compute(1);
}
"#;
        let refs = extract_references_for_extension("ts", src);
        assert!(
            refs.iter().any(|r| r.kind == RefKind::Import),
            "refs: {:?}",
            refs
        );
        assert!(
            refs.iter().any(|r| r.kind == RefKind::Call && r.target_name == "compute"),
            "refs: {:?}",
            refs
        );
    }

    #[test]
    fn extract_python_references() {
        let src = r#"
from service.worker import process
def run():
    return process()
"#;
        let refs = extract_references_for_extension("py", src);
        assert!(
            refs.iter().any(|r| r.kind == RefKind::Import),
            "refs: {:?}",
            refs
        );
        assert!(
            refs.iter().any(|r| r.kind == RefKind::Call && r.target_name == "process"),
            "refs: {:?}",
            refs
        );
    }

    #[test]
    fn test_file_detection() {
        // Real test files
        assert!(is_test_file("tests/auth_test.rs"));
        assert!(is_test_file("tests/integration/api.rs"));
        assert!(is_test_file("src/auth/tests/session.rs"));
        assert!(is_test_file("src/foo_test.rs"));
        assert!(is_test_file("src/foo_test.go"));
        assert!(is_test_file("src/foo.test.ts"));
        assert!(is_test_file("src/foo.spec.js"));
        assert!(is_test_file("src/__tests__/widget.tsx"));

        // Source files that previously tripped the loose substring match
        assert!(!is_test_file("src/auth/session.rs"));
        assert!(!is_test_file("src/latest_release.rs"));
        assert!(!is_test_file("src/attestation.rs"));
        assert!(!is_test_file("src/contest.rs"));
        assert!(!is_test_file("src/test_utils.rs")); // helper module, not a test
        assert!(!is_test_file("src/auth/fastest_path.rs"));
    }

    /// Pin the `GAVIERO_QUERIES` env var to the workspace's bundled
    /// queries so the query-driven extractor takes precedence over the
    /// AST-walker fallback. Returns `false` when the queries dir isn't
    /// on disk (stripped CI checkouts) — tests that need the query
    /// path should early-return in that case.
    fn pin_workspace_queries() -> bool {
        let workspace_queries =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../queries");
        if !workspace_queries.exists() {
            return false;
        }
        // Safety: tests in this module don't read `GAVIERO_QUERIES`
        // concurrently. The override sticks for the rest of the
        // process; that's fine because every fixture in this suite
        // either prefers the query path or is byte-equivalent to it.
        unsafe {
            std::env::set_var("GAVIERO_QUERIES", &workspace_queries);
        }
        true
    }

    #[test]
    fn query_driven_path_extracts_typescript_class_hierarchy() {
        if !pin_workspace_queries() {
            return;
        }
        let src = r#"
import { Base } from './base';
class Widget extends Base implements Renderable {
    render() { compute(); }
}
"#;
        let refs = extract_references_for_extension("ts", src);
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Import && r.target_name.contains("base")),
            "TS query path missed import: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Implements && r.target_name == "Base"),
            "TS query path missed extends Base: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Implements && r.target_name == "Renderable"),
            "TS query path missed implements Renderable: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Call && r.target_name == "compute"),
            "TS query path missed call: {refs:?}"
        );
    }

    #[test]
    fn query_driven_path_extracts_python_superclass() {
        if !pin_workspace_queries() {
            return;
        }
        let src = r#"
from service.worker import Worker
class Pipeline(Worker):
    def run(self):
        process()
"#;
        let refs = extract_references_for_extension("py", src);
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Import && r.target_name.contains("worker")),
            "Python query path missed import: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Implements && r.target_name == "Worker"),
            "Python query path missed superclass: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Call && r.target_name == "process"),
            "Python query path missed call: {refs:?}"
        );
    }

    #[test]
    fn query_driven_path_extracts_rust_imports_and_calls() {
        if !pin_workspace_queries() {
            return;
        }
        let src = r#"
use std::collections::HashMap;
fn main() {
    let x = compute_value(42);
}
"#;
        let refs = extract_references_for_extension("rs", src);
        assert!(
            refs.iter().any(|r| r.kind == RefKind::Import),
            "query-driven extractor missed imports: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == RefKind::Call && r.target_name == "compute_value"),
            "query-driven extractor missed call: {refs:?}"
        );
    }

    #[test]
    fn test_file_emits_both_typed_and_testof_edges() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "process",
                "src/lib.rs::process",
                "src/lib.rs",
                Some(1),
                Some(5),
                Some("rust"),
                None,
            )
            .unwrap();

        let refs = vec![RawReference {
            target_name: "process".into(),
            kind: RefKind::Call,
            line: 7,
        }];

        let count =
            resolve_and_insert_edges(&store, "tests/lib_test.rs", "tests::lib_test", &refs)
                .unwrap();

        // One Calls edge (preserves intent) AND one TestOf edge (so
        // `mode=tests` finds the test from the symbol-under-test).
        assert_eq!(count, 2);

        let callers = store
            .impact_radius_with_mode(&["src/lib.rs"], 2, BlastRadiusMode::Callers)
            .unwrap();
        assert!(
            callers
                .affected_files
                .contains(&"tests/lib_test.rs".to_string()),
            "Calls edge from test must still surface in mode=callers: {:?}",
            callers.affected_files
        );

        let tests = store
            .impact_radius_with_mode(&["src/lib.rs"], 2, BlastRadiusMode::Tests)
            .unwrap();
        assert!(
            tests
                .affected_files
                .contains(&"tests/lib_test.rs".to_string()),
            "TestOf edge from test must surface in mode=tests: {:?}",
            tests.affected_files
        );
    }

    #[test]
    fn resolve_cross_file_edges() {
        let store = GraphStore::open_memory().unwrap();

        // File a.rs defines `compute`
        store
            .upsert_node(
                NodeKind::Function,
                "compute",
                "a.rs::compute",
                "a.rs",
                Some(1),
                Some(5),
                Some("rust"),
                None,
            )
            .unwrap();

        // File b.rs defines `caller` and calls `compute`
        store
            .upsert_node(
                NodeKind::Function,
                "caller",
                "b.rs::caller",
                "b.rs",
                Some(1),
                Some(10),
                Some("rust"),
                None,
            )
            .unwrap();

        let refs = vec![RawReference {
            target_name: "compute".into(),
            kind: RefKind::Call,
            line: 5,
        }];

        let count = resolve_and_insert_edges(&store, "b.rs", "b.rs", &refs).unwrap();
        assert_eq!(count, 1);

        // When a.rs changes, b.rs should be affected (b calls a)
        let result = store.impact_radius(&["a.rs"], 3).unwrap();
        assert!(
            result.affected_files.contains(&"b.rs".to_string()),
            "b.rs should be affected when a.rs changes: {:?}",
            result.affected_files
        );
    }
}
