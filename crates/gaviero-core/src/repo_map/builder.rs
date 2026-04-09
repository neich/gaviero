//! Build a file reference graph from a workspace using tree-sitter.
//!
//! For each source file in the workspace:
//! 1. Parse it with tree-sitter (if the extension is supported)
//! 2. Extract top-level definition names (functions, structs, traits, etc.)
//! 3. Record token count estimate (characters / 4, a rough proxy)
//!
//! Edges (file-to-file references) are future work; the initial graph has
//! nodes only. PageRank will rank files by their proximity to owned nodes.

use std::path::{Path, PathBuf};

use petgraph::graph::DiGraph;

use crate::tree_sitter::language_for_extension;

use super::{FileNode, ReferenceEdge, Symbol};

/// Walk `workspace` and build a `DiGraph<FileNode, ReferenceEdge>`.
///
/// Skips hidden directories (`.git`, `.cargo`, `target`, `node_modules`).
/// Unknown file extensions produce a node with an empty symbol list.
pub fn build(workspace: &Path) -> anyhow::Result<DiGraph<FileNode, ReferenceEdge>> {
    let mut graph: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
    walk_dir(workspace, workspace, &mut graph)?;
    Ok(graph)
}

fn walk_dir(
    dir: &Path,
    workspace: &Path,
    graph: &mut DiGraph<FileNode, ReferenceEdge>,
) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("repo_map: cannot read dir {}: {}", dir.display(), e);
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Skip hidden dirs and common build/vendor dirs
        if file_name.starts_with('.') || SKIP_DIRS.contains(&file_name) {
            continue;
        }

        if path.is_dir() {
            walk_dir(&path, workspace, graph)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(workspace).unwrap_or(&path).to_path_buf();
            if let Some(node) = build_node(&path, rel) {
                graph.add_node(node);
            }
        }
    }

    Ok(())
}

pub const SKIP_DIRS: &[&str] = &[
    "target", "node_modules", ".git", ".cargo", ".cache",
    "dist", "build", "__pycache__",
];

/// Build a `FileNode` for a single source file, or `None` if unreadable.
/// Maximum file size to read for repo map (1 MB). Larger files (binary data,
/// compiled artifacts, etc.) are skipped — they are not useful for context ranking.
const MAX_FILE_BYTES: u64 = 1_000_000;

fn build_node(abs_path: &Path, rel_path: PathBuf) -> Option<FileNode> {
    // Skip files that are too large to be source code (binary data, build artifacts, etc.)
    if let Ok(meta) = std::fs::metadata(abs_path) {
        if meta.len() > MAX_FILE_BYTES {
            tracing::debug!("repo_map: skipping large file ({} bytes): {}", meta.len(), abs_path.display());
            return None;
        }
    }

    let content = match std::fs::read_to_string(abs_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("repo_map: cannot read {}: {}", abs_path.display(), e);
            return None;
        }
    };
    let token_estimate = content.len() / 4;

    let ext = abs_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let symbols = if language_for_extension(ext).is_some() {
        extract_symbols(ext, &content)
    } else {
        Vec::new()
    };

    Some(FileNode {
        path: rel_path,
        token_estimate,
        symbols,
    })
}

/// Extract top-level definition names from source content via tree-sitter.
pub fn extract_symbols(ext: &str, content: &str) -> Vec<Symbol> {
    let Some(language) = language_for_extension(ext) else {
        return Vec::new();
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };

    let source = content.as_bytes();
    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), source, &mut symbols);
    symbols
}

const DEF_KINDS: &[&str] = &[
    "function_item", "function_definition", "function_declaration",
    "method_declaration", "method_definition",
    "class_declaration", "class_definition",
    "struct_item", "enum_item", "impl_item", "trait_item",
    "interface_declaration", "const_item",
    "object_declaration", "companion_object",
];

fn collect_symbols(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Symbol>) {
    if DEF_KINDS.contains(&node.kind()) {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source) {
                out.push(Symbol {
                    name: name.to_string(),
                    kind: node.kind().to_string(),
                    line: node.start_position().row,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, source, out);
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
    fn unknown_extension_returns_empty() {
        let syms = extract_symbols("xyz", "some content");
        assert!(syms.is_empty());
    }
}
