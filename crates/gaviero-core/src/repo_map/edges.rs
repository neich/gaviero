//! Extract cross-file reference edges from parsed source files.
//!
//! Uses tree-sitter AST walking to find:
//! - `use` / `import` statements → Imports edges
//! - Function call identifiers → potential Calls edges (resolved against symbol table)
//! - `impl Trait for Type` → Implements edges
//! - Test files/calls → TestOf edges

use super::store::{EdgeKind, GraphStore, NodeKind};

/// A raw reference extracted from source code, before cross-file resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawReference {
    /// The referenced name: the last path segment of a call or an imported
    /// item (`helper` for `beta_core::util::helper()`).
    pub target_name: String,
    /// What kind of reference this is.
    pub kind: RefKind,
    /// Line number in source where the reference appears.
    pub line: usize,
    /// The written path, all segments including `target_name`
    /// (`["beta_core", "util", "helper"]`), for scoped calls, imports and
    /// implemented traits. Empty for bare and method calls.
    pub path: Vec<String>,
    /// How a call was written. `Bare` for non-calls.
    pub style: CallStyle,
    /// Local binding of an import when it differs from `target_name`
    /// (`use a::b as c;`, `import { x as y }`, `from m import f as g`).
    pub alias: Option<String>,
    /// Import of everything in a module (`use p::*`, `from m import *`).
    pub glob: bool,
    /// Re-export (`pub use`, TS `export … from`).
    pub public: bool,
    /// TypeScript / Python import source as written (`./math`, `.tools`,
    /// `pkg.core`).
    pub module: Option<String>,
    /// TypeScript / Python member call receiver when it is a plain
    /// identifier (`m` in `m.mul()`).
    pub receiver: Option<String>,
    /// Rust method calls: how the receiver was written, locals
    /// substituted (receiver-typing plan R3).
    pub receiver_expr: Option<super::types::ReceiverExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefKind {
    /// `use x::y` or `mod x` — module-level import.
    Import,
    /// `foo()` — function/method call.
    #[default]
    Call,
    /// `impl Trait for Type` — trait implementation.
    Implements,
}

/// How a call was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallStyle {
    /// `foo()`
    #[default]
    Bare,
    /// `a::b::foo()`
    Scoped,
    /// `x.foo()`
    Method,
}

impl RawReference {
    pub(super) fn new(target_name: impl Into<String>, kind: RefKind, line: usize) -> Self {
        Self {
            target_name: target_name.into(),
            kind,
            line,
            ..Default::default()
        }
    }
}

/// Extract raw references from a supported source extension.
///
/// Resolution order:
/// 1. **Structured extractors** for Rust, TypeScript / TSX and Python:
///    iterative AST walks that keep what [`super::resolve`] needs (written
///    paths, call style, import bindings, re-exports). They run the same
///    everywhere, whether or not a `queries/` directory is found.
/// 2. **Query-driven** (`.scm`) extraction when an `edges.scm` is bundled
///    or installed for another language ([`extract_via_edges_query`]).
/// 3. **Generic AST walker** for the remaining tree-sitter languages.
pub fn extract_references_for_extension(ext: &str, source: &str) -> Vec<RawReference> {
    // Languages with a structured extractor use it everywhere: the result
    // must not depend on whether a `queries/` directory happens to be found
    // next to the binary or in the current directory.
    match ext {
        "rs" => return extract_rust_references(source),
        "ts" | "tsx" | "py" | "pyi" => {
            if let Some(refs) = extract_structured(ext, source) {
                return refs;
            }
        }
        _ => {}
    }
    if let Some(refs) = extract_via_edges_query(ext, source) {
        return refs;
    }
    extract_generic_references(ext, source)
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
        let target = target
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .to_string();
        if target.is_empty() {
            continue;
        }
        if matches!(kind, RefKind::Call) && is_noise_call(&target) {
            continue;
        }
        refs.push(RawReference::new(
            target,
            kind,
            head_node.unwrap_or(node).start_position().row,
        ));
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
    extract_rust_file(source).refs
}

/// A file's references and, for Rust, its type facts.
#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub refs: Vec<RawReference>,
    pub facts: Option<super::rust_walk::RustFileFacts>,
}

/// References of any supported file plus, for Rust, the type facts the
/// receiver-typing resolver needs. One parse.
pub fn extract_file(ext: &str, source: &str) -> Extracted {
    if ext == "rs" {
        let e = extract_rust_file(source);
        return Extracted { refs: e.refs, facts: Some(e.facts) };
    }
    Extracted { refs: extract_references_for_extension(ext, source), facts: None }
}

/// Rust references and type facts.
pub fn extract_rust_file(source: &str) -> super::rust_walk::RustExtracted {
    let language = crate::tree_sitter::language_for_extension("rs");
    let Some(language) = language else {
        // Should never happen — Rust grammar is statically registered.
        // Log loudly because zero edges from a Rust file is otherwise
        // indistinguishable from "the file legitimately has no refs".
        tracing::warn!(target: "repo_map_edges", ext = "rs", "rust grammar unavailable; emitting zero edges");
        return Default::default();
    };

    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&language) {
        tracing::warn!(target: "repo_map_edges", ext = "rs", error = %e, "rust parser language assignment failed");
        return Default::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        tracing::warn!(target: "repo_map_edges", ext = "rs", source_bytes = source.len(), "rust parse returned no tree");
        return Default::default();
    };
    super::rust_walk::walk(tree.root_node(), source.as_bytes())
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
                out.push(RawReference::new(
                    name,
                    RefKind::Import,
                    node.start_position().row,
                ));
            }
        }
        "call_expression" | "call" => {
            if let Some(func_node) = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))
                && let Some(name) = terminal_identifier(func_node, source)
                && !is_noise_call(&name)
            {
                out.push(RawReference::new(
                    name,
                    RefKind::Call,
                    node.start_position().row,
                ));
            }
        }
        "class_declaration" => {
            for field in ["superclass", "extends", "base_class"] {
                if let Some(parent) = node.child_by_field_name(field)
                    && let Some(name) = terminal_identifier(parent, source)
                {
                    out.push(RawReference::new(
                        name,
                        RefKind::Implements,
                        node.start_position().row,
                    ));
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

/// Rewrite leading `super`s written `depth` inline modules deep so the path
/// is relative to the file's module: `super::x` → `self::x` at depth 1,
/// `super::super::x` → `super::x`.
pub(super) fn lift_inline_super(path: &mut Vec<String>, depth: usize) {
    let supers = path.iter().take_while(|s| *s == "super").count();
    if supers == 0 {
        return;
    }
    let rest = path.split_off(supers);
    *path = if supers <= depth {
        vec!["self".to_string()]
    } else {
        vec!["super".to_string(); supers - depth]
    };
    path.extend(rest);
}

/// The reference a call's `function` node makes, if it is worth an edge.
pub(super) fn rust_call(func: tree_sitter::Node, source: &[u8], line: usize) -> Option<RawReference> {
    let func = match func.kind() {
        // `foo::<T>(…)` / `a::b::<T>(…)`
        "generic_function" => func.child_by_field_name("function")?,
        _ => func,
    };
    match func.kind() {
        "identifier" => {
            let name = func.utf8_text(source).ok()?;
            (!is_noise_call(name)).then(|| RawReference::new(name, RefKind::Call, line))
        }
        "scoped_identifier" => {
            let text = func.utf8_text(source).ok()?;
            let path = path_segments(text);
            let joined = path.join("::");
            let name = path.last()?.clone();
            if is_noise_call(&joined) {
                return None;
            }
            Some(RawReference {
                path,
                style: CallStyle::Scoped,
                ..RawReference::new(name, RefKind::Call, line)
            })
        }
        "field_expression" => {
            let name = func
                .child_by_field_name("field")
                .and_then(|n| n.utf8_text(source).ok())?;
            (!is_noise_call(name)).then(|| RawReference {
                style: CallStyle::Method,
                ..RawReference::new(name, RefKind::Call, line)
            })
        }
        _ => None,
    }
}

/// Split a written Rust path into segments, dropping generic arguments
/// (`Vec::<u8>::new` → `["Vec", "new"]`) and a leading `::`.
pub(crate) fn path_segments(text: &str) -> Vec<String> {
    let mut clean = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => clean.push(ch),
            _ => {}
        }
    }
    clean
        .split("::")
        .map(|s| s.trim().trim_start_matches("dyn ").trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Expand one `use` tree into leaf references, prefixing `prefix`.
/// Bounded nesting (malformed input degrades to fewer references).
pub(super) fn expand_use_tree(
    node: tree_sitter::Node,
    source: &[u8],
    prefix: &[String],
    public: bool,
    line: usize,
    depth: usize,
    out: &mut Vec<RawReference>,
) {
    if depth > 32 {
        return;
    }
    let joined = |tail: Vec<String>| -> Vec<String> {
        let mut p = prefix.to_vec();
        p.extend(tail);
        p
    };
    let text = |n: tree_sitter::Node| n.utf8_text(source).ok().map(path_segments).unwrap_or_default();
    let leaf = |path: Vec<String>, alias: Option<String>, glob: bool, out: &mut Vec<RawReference>| {
        if path.is_empty() {
            return;
        }
        // `self` inside a list names the prefix module itself.
        let mut path = path;
        if path.last().is_some_and(|s| s == "self") && path.len() > 1 {
            path.pop();
        }
        let name = path.last().cloned().unwrap_or_default();
        out.push(RawReference {
            path,
            alias,
            glob,
            public,
            ..RawReference::new(name, RefKind::Import, line)
        });
    };
    match node.kind() {
        "use_as_clause" => {
            let path = node.child_by_field_name("path").map(text).unwrap_or_default();
            let alias = node
                .child_by_field_name("alias")
                .and_then(|n| n.utf8_text(source).ok())
                .map(str::to_string);
            leaf(joined(path), alias, false, out);
        }
        "use_wildcard" => {
            // `p::*`: the path is the wildcard's first named child, if any.
            let path = node.named_child(0).map(text).unwrap_or_default();
            leaf(joined(path), None, true, out);
        }
        "scoped_use_list" => {
            let base = joined(node.child_by_field_name("path").map(text).unwrap_or_default());
            if let Some(list) = node.child_by_field_name("list") {
                let mut c = list.walk();
                for child in list.named_children(&mut c) {
                    expand_use_tree(child, source, &base, public, line, depth + 1, out);
                }
            }
        }
        "use_list" => {
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                expand_use_tree(child, source, prefix, public, line, depth + 1, out);
            }
        }
        "scoped_identifier" | "identifier" | "crate" | "self" | "super" | "metavariable" => {
            leaf(joined(text(node)), None, false, out);
        }
        _ => {}
    }
}

/// Parse with the extension's grammar and run its structured walker.
/// `None` when the grammar is unavailable (falls back to the other paths).
fn extract_structured(ext: &str, source: &str) -> Option<Vec<RawReference>> {
    let language = crate::tree_sitter::language_for_extension(ext)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let Some(tree) = parser.parse(source, None) else {
        tracing::warn!(target: "repo_map_edges", ext, source_bytes = source.len(), "parse returned no tree");
        return Some(Vec::new());
    };
    let bytes = source.as_bytes();
    let mut refs = Vec::new();
    let mut visit = |node: tree_sitter::Node| match ext {
        "py" | "pyi" => python_node_references(node, bytes, &mut refs),
        _ => ts_node_references(node, bytes, &mut refs),
    };
    walk_preorder(tree.root_node(), &mut visit);
    Some(refs)
}

/// Iterative pre-order traversal (deep trees must not overflow the stack).
fn walk_preorder<'t>(root: tree_sitter::Node<'t>, visit: &mut impl FnMut(tree_sitter::Node<'t>)) {
    let mut cursor = root.walk();
    loop {
        visit(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

fn node_text<'s>(node: tree_sitter::Node, source: &'s [u8]) -> Option<&'s str> {
    node.utf8_text(source).ok()
}

/// A string literal's contents (`'./math'` → `./math`).
fn string_literal(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let s = node_text(node, source)?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    (!s.is_empty()).then(|| s.to_string())
}

/// `a`, `a.b.c` — a receiver made only of names (no calls or indexing).
fn plain_receiver(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let text = node_text(node, source)?.trim();
    let ok = !text.is_empty()
        && text
            .split('.')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'));
    ok.then(|| text.to_string())
}

/// TypeScript / TSX references at one node: one `Import` per import
/// binding (default → `default`, namespace → `*`, side-effect → the
/// specifier), `export … from` as public imports, calls and `new` with
/// their receiver, and `extends` / `implements` clauses.
fn ts_node_references(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawReference>) {
    let line = node.start_position().row;
    let import = |target: &str, alias: Option<String>, module: &str, public: bool, glob: bool| RawReference {
        alias,
        module: Some(module.to_string()),
        public,
        glob,
        ..RawReference::new(target, RefKind::Import, line)
    };
    match node.kind() {
        "import_statement" => {
            let Some(module) = node
                .child_by_field_name("source")
                .and_then(|s| string_literal(s, source))
            else {
                return;
            };
            let mut c = node.walk();
            let clause = node.children(&mut c).find(|n| n.kind() == "import_clause");
            let Some(clause) = clause else {
                out.push(import(&module, None, &module, false, false));
                return;
            };
            let mut c = clause.walk();
            for part in clause.named_children(&mut c) {
                match part.kind() {
                    "identifier" => {
                        let local = node_text(part, source).map(str::to_string);
                        out.push(import("default", local, &module, false, false));
                    }
                    "namespace_import" => {
                        let mut c = part.walk();
                        let local = part
                            .named_children(&mut c)
                            .find(|n| n.kind() == "identifier")
                            .and_then(|n| node_text(n, source))
                            .map(str::to_string);
                        out.push(import("*", local, &module, false, false));
                    }
                    "named_imports" => {
                        let mut c = part.walk();
                        for spec in part.named_children(&mut c).filter(|n| n.kind() == "import_specifier") {
                            let Some(name) = spec.child_by_field_name("name").and_then(|n| node_text(n, source)) else {
                                continue;
                            };
                            let alias = spec
                                .child_by_field_name("alias")
                                .and_then(|n| node_text(n, source))
                                .map(str::to_string);
                            out.push(import(name, alias, &module, false, false));
                        }
                    }
                    _ => {}
                }
            }
        }
        "export_statement" => {
            let Some(module) = node
                .child_by_field_name("source")
                .and_then(|s| string_literal(s, source))
            else {
                return;
            };
            let mut c = node.walk();
            let clause = node.children(&mut c).find(|n| n.kind() == "export_clause");
            match clause {
                Some(clause) => {
                    let mut c = clause.walk();
                    for spec in clause.named_children(&mut c).filter(|n| n.kind() == "export_specifier") {
                        let Some(name) = spec.child_by_field_name("name").and_then(|n| node_text(n, source)) else {
                            continue;
                        };
                        let alias = spec
                            .child_by_field_name("alias")
                            .and_then(|n| node_text(n, source))
                            .map(str::to_string);
                        out.push(import(name, alias, &module, true, false));
                    }
                }
                // `export * from 'm'` / `export * as ns from 'm'`
                None => out.push(import("*", None, &module, true, true)),
            }
        }
        "call_expression" | "new_expression" => {
            let field = if node.kind() == "call_expression" { "function" } else { "constructor" };
            let Some(func) = node.child_by_field_name(field) else { return };
            if let Some(r) = js_like_call(func, source, line, "member_expression", "object", "property") {
                out.push(r);
            }
        }
        "extends_clause" | "implements_clause" => {
            let mut c = node.walk();
            for ty in node.named_children(&mut c) {
                let ty = if ty.kind() == "generic_type" {
                    ty.child_by_field_name("name").unwrap_or(ty)
                } else {
                    ty
                };
                if let Some(mut r) = js_like_call(ty, source, line, "member_expression", "object", "property") {
                    r.kind = RefKind::Implements;
                    out.push(r);
                }
            }
        }
        _ => {}
    }
}

/// The reference a call target makes: a plain name (`Bare`) or a member
/// access (`Method`, with its receiver when that is plain names).
fn js_like_call(
    func: tree_sitter::Node,
    source: &[u8],
    line: usize,
    member_kind: &str,
    object_field: &str,
    property_field: &str,
) -> Option<RawReference> {
    match func.kind() {
        "identifier" | "type_identifier" => {
            let name = node_text(func, source)?;
            (!is_noise_call(name)).then(|| RawReference::new(name, RefKind::Call, line))
        }
        k if k == member_kind || k == "nested_type_identifier" => {
            let (object, property) = if k == member_kind {
                (func.child_by_field_name(object_field)?, func.child_by_field_name(property_field)?)
            } else {
                (func.child_by_field_name("module")?, func.child_by_field_name("name")?)
            };
            let name = node_text(property, source)?;
            if is_noise_call(name) {
                return None;
            }
            Some(RawReference {
                style: CallStyle::Method,
                receiver: plain_receiver(object, source),
                ..RawReference::new(name, RefKind::Call, line)
            })
        }
        _ => None,
    }
}

/// Python references at one node: `import a.b [as x]` (empty target, the
/// module in `module`), `from m import x [as y]` / `*` (relative dots kept
/// in `module`), calls with their dotted receiver, and base classes.
fn python_node_references(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawReference>) {
    let line = node.start_position().row;
    match node.kind() {
        "import_statement" => {
            let mut c = node.walk();
            for name in node.children_by_field_name("name", &mut c) {
                let (module, alias) = match name.kind() {
                    "aliased_import" => (
                        name.child_by_field_name("name").and_then(|n| node_text(n, source)),
                        name.child_by_field_name("alias")
                            .and_then(|n| node_text(n, source))
                            .map(str::to_string),
                    ),
                    _ => (node_text(name, source), None),
                };
                if let Some(module) = module {
                    out.push(RawReference {
                        module: Some(module.trim().to_string()),
                        alias,
                        ..RawReference::new("", RefKind::Import, line)
                    });
                }
            }
        }
        "import_from_statement" => {
            let Some(module) = node
                .child_by_field_name("module_name")
                .and_then(|n| node_text(n, source))
                .map(|s| s.split_whitespace().collect::<String>())
            else {
                return;
            };
            let mut c = node.walk();
            if node.children(&mut c).any(|n| n.kind() == "wildcard_import") {
                out.push(RawReference {
                    module: Some(module),
                    glob: true,
                    ..RawReference::new("*", RefKind::Import, line)
                });
                return;
            }
            let mut c = node.walk();
            for name in node.children_by_field_name("name", &mut c) {
                let (target, alias) = match name.kind() {
                    "aliased_import" => (
                        name.child_by_field_name("name").and_then(|n| node_text(n, source)),
                        name.child_by_field_name("alias")
                            .and_then(|n| node_text(n, source))
                            .map(str::to_string),
                    ),
                    _ => (node_text(name, source), None),
                };
                if let Some(target) = target {
                    out.push(RawReference {
                        module: Some(module.clone()),
                        alias,
                        ..RawReference::new(target.trim(), RefKind::Import, line)
                    });
                }
            }
        }
        "call" => {
            if let Some(func) = node.child_by_field_name("function")
                && let Some(r) = js_like_call(func, source, line, "attribute", "object", "attribute")
            {
                out.push(r);
            }
        }
        "class_definition" => {
            if let Some(bases) = node.child_by_field_name("superclasses") {
                let mut c = bases.walk();
                for base in bases.named_children(&mut c) {
                    if let Some(mut r) = js_like_call(base, source, line, "attribute", "object", "attribute") {
                        r.kind = RefKind::Implements;
                        out.push(r);
                    }
                }
            }
        }
        _ => {}
    }
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
        .split(['/', '\\'])
        .any(|seg| matches!(seg, "tests" | "test" | "__tests__" | "spec"))
}

/// Map a tree-sitter node kind to a NodeKind for the graph.
pub fn node_kind_from_ts(ts_kind: &str, file_path: &str) -> NodeKind {
    match ts_kind {
        "function_item"
        | "function_signature_item"
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
    use super::super::store::BlastRadiusMode;
    use super::*;

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

    fn imports(src: &str) -> Vec<(Vec<String>, Option<String>, bool, bool)> {
        extract_rust_references(src)
            .into_iter()
            .filter(|r| r.kind == RefKind::Import)
            .map(|r| (r.path, r.alias, r.glob, r.public))
            .collect()
    }

    fn p(s: &str) -> Vec<String> {
        s.split("::").map(str::to_string).collect()
    }

    #[test]
    fn use_trees_expand_to_leaves_with_aliases_globs_and_visibility() {
        let got = imports(
            "extern crate beta_core;\n\
             extern crate foo as bar;\n\
             use beta_core::shapes::widget::Widget;\n\
             use beta_core::util::{self, helper as h, nested::{deep, deeper::*}};\n\
             use engine::*;\n\
             use super::widget::Widget as W2;\n\
             pub use shapes::gadget::Gadget;\n\
             pub(crate) use crate::x;\n\
             use self::parts::Part;\n",
        );
        let want = vec![
            (p("beta_core"), None, false, false),
            (p("foo"), Some("bar".into()), false, false),
            (p("beta_core::shapes::widget::Widget"), None, false, false),
            (p("beta_core::util"), None, false, false),
            (p("beta_core::util::helper"), Some("h".into()), false, false),
            (p("beta_core::util::nested::deep"), None, false, false),
            (p("beta_core::util::nested::deeper"), None, true, false),
            (p("engine"), None, true, false),
            (p("super::widget::Widget"), Some("W2".into()), false, false),
            (p("shapes::gadget::Gadget"), None, false, true),
            (p("crate::x"), None, false, true),
            (p("self::parts::Part"), None, false, false),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn calls_keep_their_written_path_and_style() {
        let refs = extract_rust_references(
            "fn main() {\n\
             h();\n\
             util::only_here();\n\
             Widget::new();\n\
             w.spin();\n\
             beta_core::util::helper();\n\
             Vec::<u8>::with_capacity(1);\n\
             parse::<u32>();\n\
             String::new();\n\
             v.push(1);\n\
             }\n\
             impl beta_core::observe::Observer for X {}\n",
        );
        let calls: Vec<(String, CallStyle, Vec<String>)> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| (r.target_name.clone(), r.style, r.path.clone()))
            .collect();
        assert_eq!(
            calls,
            vec![
                ("h".into(), CallStyle::Bare, vec![]),
                ("only_here".into(), CallStyle::Scoped, p("util::only_here")),
                ("new".into(), CallStyle::Scoped, p("Widget::new")),
                ("spin".into(), CallStyle::Method, vec![]),
                ("helper".into(), CallStyle::Scoped, p("beta_core::util::helper")),
                ("with_capacity".into(), CallStyle::Scoped, p("Vec::with_capacity")),
                ("parse".into(), CallStyle::Bare, vec![]),
            ],
            "String::new and v.push stay filtered as noise"
        );
        let implements: Vec<&RawReference> =
            refs.iter().filter(|r| r.kind == RefKind::Implements).collect();
        assert_eq!(implements.len(), 1);
        assert_eq!(implements[0].target_name, "Observer");
        assert_eq!(implements[0].path, p("beta_core::observe::Observer"));
    }

    #[test]
    fn malformed_use_degrades_without_panicking() {
        let refs = extract_rust_references("use a::{b, ;\nuse ::{;\nuse ;\nfn f() { g(); }\n");
        assert!(refs.iter().any(|r| r.kind == RefKind::Call && r.target_name == "g"));
    }

    #[test]
    fn super_inside_an_inline_module_stays_in_the_file() {
        let src = "use super::outer;\n\
                   mod tests {\n    use super::*;\n    use super::super::up;\n    fn t() { super::helper(); }\n}\n\
                   fn after() { super::x(); }\n";
        let paths: Vec<String> = extract_rust_references(src)
            .iter()
            .map(|r| r.path.join("::"))
            .collect();
        assert_eq!(
            paths,
            ["super::outer", "self", "super::up", "self::helper", "super::x"],
            "only references inside `mod tests {{ }}` are lifted"
        );
    }

    #[test]
    fn calls_to_local_closures_are_not_references() {
        let src = "fn f() { let row = |x: i32| x + 1; row(1); other(2); }\n";
        let calls: Vec<String> = extract_rust_references(src)
            .into_iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.target_name)
            .collect();
        assert_eq!(calls, ["other"]);
    }

    /// `(kind, target, alias, module, receiver, public, glob)` per reference.
    type Row = (RefKind, String, Option<String>, Option<String>, Option<String>, bool, bool);

    fn rows(refs: &[RawReference]) -> Vec<Row> {
        refs.iter()
            .map(|r| {
                (
                    r.kind,
                    r.target_name.clone(),
                    r.alias.clone(),
                    r.module.clone(),
                    r.receiver.clone(),
                    r.public,
                    r.glob,
                )
            })
            .collect()
    }

    fn row(kind: RefKind, t: &str, alias: Option<&str>, module: Option<&str>, recv: Option<&str>, public: bool, glob: bool) -> Row {
        (
            kind,
            t.into(),
            alias.map(Into::into),
            module.map(Into::into),
            recv.map(Into::into),
            public,
            glob,
        )
    }

    #[test]
    fn typescript_imports_exports_and_calls_keep_their_bindings() {
        let src = "import { add, mul as times } from './math';\n\
                   import * as ns from '../lib';\n\
                   import makeCircle, { Circle } from './shapes';\n\
                   import 'polyfill';\n\
                   export { Circle as Round } from './circle';\n\
                   export * from './all';\n\
                   class Big extends Base implements Shape {}\n\
                   add(1); ns.f(); a.b.c(); new Circle(); this.x.y(); makeCircle().area();\n";
        let refs = extract_references_for_extension("ts", src);
        let (imp, call) = (RefKind::Import, RefKind::Call);
        let got = rows(&refs);
        for want in [
            row(imp, "add", None, Some("./math"), None, false, false),
            row(imp, "mul", Some("times"), Some("./math"), None, false, false),
            row(imp, "*", Some("ns"), Some("../lib"), None, false, false),
            row(imp, "default", Some("makeCircle"), Some("./shapes"), None, false, false),
            row(imp, "Circle", None, Some("./shapes"), None, false, false),
            row(imp, "polyfill", None, Some("polyfill"), None, false, false),
            row(imp, "Circle", Some("Round"), Some("./circle"), None, true, false),
            row(imp, "*", None, Some("./all"), None, true, true),
            row(RefKind::Implements, "Base", None, None, None, false, false),
            row(RefKind::Implements, "Shape", None, None, None, false, false),
            row(call, "add", None, None, None, false, false),
            row(call, "f", None, None, Some("ns"), false, false),
            row(call, "c", None, None, Some("a.b"), false, false),
            row(call, "Circle", None, None, None, false, false),
            row(call, "y", None, None, Some("this.x"), false, false),
            row(call, "area", None, None, None, false, false),
        ] {
            assert!(got.contains(&want), "missing {want:?} in {got:#?}");
        }
    }

    #[test]
    fn python_imports_and_calls_keep_modules_aliases_and_receivers() {
        let src = "import os\nimport pkg.core as pc, json\n\
                   from .tools import tool, other as o\nfrom .. import sibling\nfrom pkg.extra import *\n\
                   class C(Base, mod.Mixin):\n    pass\n\
                   tool()\npkg.core.compute()\nself.save()\nget().go()\n";
        let refs = extract_references_for_extension("py", src);
        let (imp, call) = (RefKind::Import, RefKind::Call);
        let got = rows(&refs);
        for want in [
            row(imp, "", None, Some("os"), None, false, false),
            row(imp, "", Some("pc"), Some("pkg.core"), None, false, false),
            row(imp, "", None, Some("json"), None, false, false),
            row(imp, "tool", None, Some(".tools"), None, false, false),
            row(imp, "other", Some("o"), Some(".tools"), None, false, false),
            row(imp, "sibling", None, Some(".."), None, false, false),
            row(imp, "*", None, Some("pkg.extra"), None, false, true),
            row(RefKind::Implements, "Base", None, None, None, false, false),
            row(RefKind::Implements, "Mixin", None, None, Some("mod"), false, false),
            row(call, "tool", None, None, None, false, false),
            row(call, "compute", None, None, Some("pkg.core"), false, false),
            row(call, "save", None, None, Some("self"), false, false),
            row(call, "go", None, None, None, false, false),
        ] {
            assert!(got.contains(&want), "missing {want:?} in {got:#?}");
        }
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
            refs.iter()
                .any(|r| r.kind == RefKind::Call && r.target_name == "compute"),
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
            refs.iter()
                .any(|r| r.kind == RefKind::Call && r.target_name == "process"),
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
    /// queries. Rust, TypeScript and Python must extract the same with or
    /// without them (their structured extractors never consult the
    /// queries). Returns `false` when the queries dir isn't on disk
    /// (stripped CI checkouts) — those tests early-return then.
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
                .any(|r| r.kind == RefKind::Import && r.module.as_deref() == Some("./base")),
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
                .any(|r| r.kind == RefKind::Import && r.module.as_deref() == Some("service.worker")),
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

        let refs = vec![RawReference::new("process", RefKind::Call, 7)];

        let count = resolve_and_insert_edges(&store, "tests/lib_test.rs", "tests::lib_test", &refs)
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

        let refs = vec![RawReference::new("compute", RefKind::Call, 5)];

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
