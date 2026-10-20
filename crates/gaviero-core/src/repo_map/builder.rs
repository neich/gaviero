//! Tree-sitter symbol extraction helpers and the shared skip / exclude
//! rules used by the code graph ([`super::graph_builder`]).
//!
//! The repo outline no longer walks the workspace itself: [`super::RepoMap`]
//! is a projection of the code graph ([`super::RepoMap::from_store`]).

use std::path::Path;

use super::Symbol;

/// Returns `true` if `rel_path` (or its basename) matches any exclude pattern.
///
/// Rules:
/// - An entry without `/` is treated as a basename match (e.g. `node_modules`
///   excludes any directory with that name at any depth).
/// - An entry with `/` is treated as a glob-style pattern matched against the
///   workspace-relative path (reuses [`crate::path_pattern::matches`]).
pub fn is_excluded(file_name: &str, rel_path: &Path, excludes: &[String]) -> bool {
    if excludes.is_empty() {
        return false;
    }
    let rel_str = rel_path.to_string_lossy();
    for pat in excludes {
        let trimmed = pat.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains('/') && trimmed == file_name {
            return true;
        }
        if crate::path_pattern::matches(trimmed, &rel_str) {
            return true;
        }
    }
    false
}

pub const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".cargo",
    ".cache",
    "dist",
    "build",
    "__pycache__",
    // Scratch trees. `tmp/hermes-agent` in this workspace was 23k of 24k
    // walked files and made MCP `blast_radius` PageRank take minutes while
    // holding the graph mutex — chat sat on a spinner with no tokens.
    "tmp",
    "temp",
];

pub use super::extraction::{MAX_FILE_BYTES, MAX_PARSE_BYTES};

/// Extract top-level definition names from source content via tree-sitter,
/// with the 2 s deadline but **no** size cap. Compatibility wrapper for
/// callers that need only symbols; prefer [`super::extraction::extract`],
/// which applies the whole policy and reports why a list is empty.
pub fn extract_symbols(ext: &str, content: &str) -> Vec<Symbol> {
    let started = std::time::Instant::now();
    super::extraction::parse_with_deadline(ext, content, &mut || {
        started.elapsed().as_micros() as u64 > super::extraction::PARSE_TIMEOUT_MICROS
    })
    .symbols
}

const DEF_KINDS: &[&str] = &[
    "function_item",
    // Bodiless trait methods (`fn notify(&self);`): the resolver needs them
    // to send a `dyn Trait` method call to the trait's file.
    "function_signature_item",
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
    "const_item",
    "object_declaration",
    "companion_object",
];

/// The owner a Rust container gives the items inside it: `impl:Type`,
/// `impl:Type:Trait` or `trait:Name`. `None` for any other node.
pub(super) fn container_owner(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let text = |field: &str| {
        node.child_by_field_name(field)
            .and_then(|n| n.utf8_text(source).ok())
            .map(type_base_name)
            .filter(|s| !s.is_empty())
    };
    match node.kind() {
        "impl_item" => {
            let ty = text("type")?;
            Some(match text("trait") {
                Some(tr) => format!("impl:{ty}:{tr}"),
                None => format!("impl:{ty}"),
            })
        }
        "trait_item" => text("name").map(|n| format!("trait:{n}")),
        _ => None,
    }
}

/// The outer attributes of a Rust item (`attribute_item` siblings right
/// before it, comments skipped), whitespace removed: `#[tokio::test]`.
fn outer_attributes(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "attribute_item" => {
                if let Ok(text) = p.utf8_text(source) {
                    out.push(text.chars().filter(|c| !c.is_whitespace()).collect());
                }
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        prev = p.prev_sibling();
    }
    out
}

/// An attribute's text between `#[` and `]`.
fn attribute_inner(attr: &str) -> &str {
    attr.strip_prefix("#[")
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(attr)
}

/// A test attribute: its path (before any `(`) is `test` or ends in
/// `::test` (`#[test]`, `#[tokio::test(flavor = "…")]`).
fn has_test_attr(node: tree_sitter::Node, source: &[u8]) -> bool {
    outer_attributes(node, source).iter().any(|a| {
        let path = attribute_inner(a).split('(').next().unwrap_or("");
        path == "test" || path.ends_with("::test")
    })
}

/// Exactly `#[cfg(test)]` (not `cfg(all(test, …))`).
fn has_cfg_test(node: tree_sitter::Node, source: &[u8]) -> bool {
    outer_attributes(node, source)
        .iter()
        .any(|a| attribute_inner(a) == "cfg(test)")
}

/// `crate::a::Foo<T>` / `&mut Foo` / `dyn Foo + Send` → `Foo`.
pub(super) fn type_base_name(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("dyn ").unwrap_or(t);
    let t = t.trim_start_matches(['&', ' ']).trim_start_matches("mut ").trim();
    let t = t.split(['<', '+', '(']).next().unwrap_or(t).trim();
    t.rsplit("::").next().unwrap_or(t).trim().to_string()
}

/// Pre-order walk collecting definitions. Iterative on purpose: a deeply
/// nested input (e.g. 180,000 nested parentheses) parses fine but blew the
/// stack of the old recursive walk and aborted the whole process.
pub(super) fn collect_symbols(root: tree_sitter::Node, source: &[u8], out: &mut Vec<Symbol>) {
    let mut cursor = root.walk();
    // Enclosing Rust `impl` / `trait` blocks as (end byte, owner), innermost
    // last. Pre-order visits a container before its contents, so a block is
    // popped once the walk reaches a node starting at or after its end.
    let mut owners: Vec<(usize, String)> = Vec::new();
    // End bytes of the enclosing Rust `#[cfg(test)]` modules, innermost
    // last, popped the same way.
    let mut test_mods: Vec<usize> = Vec::new();
    loop {
        let node = cursor.node();
        while owners.last().is_some_and(|(end, _)| node.start_byte() >= *end) {
            owners.pop();
        }
        while test_mods.last().is_some_and(|end| node.start_byte() >= *end) {
            test_mods.pop();
        }
        if DEF_KINDS.contains(&node.kind())
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(name) = name_node.utf8_text(source)
        {
            let test = node.kind() == "function_item"
                && (!test_mods.is_empty() || has_test_attr(node, source));
            out.push(Symbol {
                name: name.to_string(),
                kind: node.kind().to_string(),
                line: node.start_position().row,
                end_line: node.end_position().row,
                owner: owners.last().map(|(_, o)| o.clone()),
                test,
            });
        }
        if let Some(owner) = container_owner(node, source) {
            owners.push((node.end_byte(), owner));
        }
        if node.kind() == "mod_item" && has_cfg_test(node, source) {
            test_mods.push(node.end_byte());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rust_symbols() {
        let src = r#"
fn foo() {}
struct Bar { x: i32 }
pub trait Baz {}
"#;
        let syms = extract_symbols("rs", src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"foo"), "{:?}", names);
        assert!(names.contains(&"Bar"), "{:?}", names);
    }

    #[test]
    fn symbols_record_their_end_lines() {
        let syms = extract_symbols("rs", "fn a() {\n    1;\n}\nstruct B;\n");
        let span = |name: &str| {
            syms.iter()
                .find(|s| s.name == name)
                .map(|s| (s.line, s.end_line))
                .unwrap_or_else(|| panic!("{name} missing: {syms:?}"))
        };
        assert_eq!(span("a"), (0, 2));
        assert_eq!(span("B"), (3, 3));
    }

    #[test]
    fn inline_rust_tests_are_marked() {
        let src = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    use super::*;\n    struct Fixture;\n    fn helper() {}\n    #[test]\n    fn checks() { prod(); }\n}\n#[tokio::test]\nasync fn t2() {}\nfn after() {}\n";
        let syms = extract_symbols("rs", src);
        let test = |name: &str| {
            syms.iter()
                .find(|s| s.name == name)
                .map(|s| s.test)
                .unwrap_or_else(|| panic!("{name} missing: {syms:?}"))
        };
        for name in ["helper", "checks", "t2"] {
            assert!(test(name), "{name} is test code");
        }
        for name in ["prod", "Fixture", "after"] {
            assert!(!test(name), "{name} is not test code");
        }
    }

    #[test]
    fn symbols_record_their_impl_and_trait_owners() {
        let src = "pub trait Observer {\n    fn notify(&self);\n    fn default_hook(&self) {}\n}\n\
                   pub struct Gadget;\n\
                   impl Gadget {\n    pub fn new() -> Self { Gadget }\n}\n\
                   impl<T> crate::observe::Observer for Wrapper<T> {\n    fn notify(&self) {}\n}\n\
                   pub fn free() {}\n";
        let syms = extract_symbols("rs", src);
        let owner = |name: &str, line: usize| {
            syms.iter()
                .find(|s| s.name == name && s.line == line)
                .map(|s| s.owner.clone())
                .unwrap_or_else(|| panic!("{name}@{line} missing: {syms:?}"))
        };
        assert_eq!(owner("Observer", 0), None);
        assert_eq!(owner("notify", 1), Some("trait:Observer".into()), "bodiless trait method");
        assert_eq!(owner("default_hook", 2), Some("trait:Observer".into()));
        assert_eq!(owner("Gadget", 4), None);
        assert_eq!(owner("new", 6), Some("impl:Gadget".into()));
        assert_eq!(owner("notify", 9), Some("impl:Wrapper:Observer".into()));
        assert_eq!(owner("free", 11), None, "owner scope ends with the block");
    }

    #[test]
    fn type_base_names_drop_paths_generics_and_references() {
        assert_eq!(type_base_name("crate::a::Foo<T>"), "Foo");
        assert_eq!(type_base_name("&mut Foo"), "Foo");
        assert_eq!(type_base_name("dyn Foo + Send"), "Foo");
        assert_eq!(type_base_name("Vec<Vec<u8>>"), "Vec");
    }

    #[test]
    fn unknown_extension_returns_empty() {
        let syms = extract_symbols("xyz", "some content");
        assert!(syms.is_empty());
    }

    #[test]
    fn skip_dirs_includes_scratch_trees() {
        assert!(SKIP_DIRS.contains(&"tmp"));
        assert!(SKIP_DIRS.contains(&"temp"));
        assert!(SKIP_DIRS.contains(&"target"));
    }
}
