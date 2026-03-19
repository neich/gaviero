use anyhow::{Result, bail};
use gaviero_core::{Language, Query, QueryCursor, Tree};
use ratatui::style::Style;
use streaming_iterator::StreamingIterator;

use crate::theme::Theme;

pub struct HighlightConfig {
    pub query: Query,
    pub group_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub style: Style,
}

/// Load highlight queries for a language.
pub fn load_highlight_config(language: Language, lang_name: &str) -> Result<HighlightConfig> {
    let scm = find_query_file(lang_name, "highlights.scm")?;
    let query = Query::new(&language, &scm)
        .map_err(|e| anyhow::anyhow!("tree-sitter query error for {}: {}", lang_name, e))?;
    let group_names = query.capture_names().iter().map(|s| s.to_string()).collect();
    Ok(HighlightConfig { query, group_names })
}

/// Find a highlight query file using the shared query loader.
fn find_query_file(lang: &str, file: &str) -> Result<String> {
    gaviero_core::query_loader::find_query_file(lang, file, bundled_highlight_query)
}

/// Bundled highlight queries compiled into the binary.
fn bundled_highlight_query(lang: &str, _file: &str) -> Result<String> {
    match lang {
        "rust" => Ok(include_str!("../../../../queries/rust/highlights.scm").to_string()),
        "java" => Ok(include_str!("../../../../queries/java/highlights.scm").to_string()),
        "javascript" => Ok(include_str!("../../../../queries/javascript/highlights.scm").to_string()),
        "typescript" => Ok(include_str!("../../../../queries/typescript/highlights.scm").to_string()),
        "html" => Ok(include_str!("../../../../queries/html/highlights.scm").to_string()),
        "css" => Ok(include_str!("../../../../queries/css/highlights.scm").to_string()),
        "json" => Ok(include_str!("../../../../queries/json/highlights.scm").to_string()),
        "bash" => Ok(include_str!("../../../../queries/bash/highlights.scm").to_string()),
        "toml" => Ok(include_str!("../../../../queries/toml/highlights.scm").to_string()),
        "c" => Ok(include_str!("../../../../queries/c/highlights.scm").to_string()),
        "cpp" => Ok(include_str!("../../../../queries/cpp/highlights.scm").to_string()),
        "latex" => Ok(include_str!("../../../../queries/latex/highlights.scm").to_string()),
        "python" => Ok(include_str!("../../../../queries/python/highlights.scm").to_string()),
        "yaml" => Ok(include_str!("../../../../queries/yaml/highlights.scm").to_string()),
        _ => bail!("no bundled highlights.scm for {}", lang),
    }
}

/// Run highlights for a visible viewport range. Returns styled spans sorted by position.
pub fn run_highlights(
    tree: &Tree,
    source: &ropey::Rope,
    config: &HighlightConfig,
    theme: &Theme,
    visible_range: std::ops::Range<usize>,
) -> Vec<StyledSpan> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(visible_range.clone());

    let source_str = source.to_string();
    let source_bytes = source_str.as_bytes();

    // QueryMatches implements StreamingIterator, not Iterator
    let mut matches = cursor.matches(&config.query, tree.root_node(), source_bytes);

    let mut spans = Vec::new();
    while let Some(m) = { matches.advance(); matches.get() } {
        for capture in m.captures {
            let group = &config.group_names[capture.index as usize];
            if let Some(style) = theme.highlight_style(group) {
                let node = capture.node;
                spans.push(StyledSpan {
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    style,
                });
            }
        }
    }

    // Sort: by start_byte ascending, then by span width descending.
    // Wider (less specific) spans come first so narrower (more specific)
    // spans override them in the "last match wins" rendering loop.
    spans.sort_by(|a, b| {
        a.start_byte.cmp(&b.start_byte)
            .then_with(|| {
                let a_width = a.end_byte - a.start_byte;
                let b_width = b.end_byte - b.start_byte;
                b_width.cmp(&a_width) // wider first
            })
    });
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_query_file_fallback() {
        let scm = find_query_file("rust", "highlights.scm");
        assert!(scm.is_ok(), "rust highlights.scm should be bundled");
        let content = scm.unwrap();
        assert!(content.contains("comment"), "should contain comment patterns");
    }

    #[test]
    fn test_unknown_language_query() {
        let result = find_query_file("unknown_lang", "highlights.scm");
        assert!(result.is_err());
    }

    #[test]
    fn test_rust_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("rs")
            .expect("should have rust language");
        let config = load_highlight_config(lang.clone(), "rust")
            .expect("should load rust highlights");
        eprintln!("Capture groups: {:?}", config.group_names);

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = r#"fn main() { let x = 42; }"#;
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        eprintln!("Spans produced: {}", spans.len());
        for s in &spans {
            eprintln!("  byte {}..{} => {:?}", s.start_byte, s.end_byte, s.style);
        }
        assert!(!spans.is_empty(), "should produce highlight spans for Rust code");
    }

    #[test]
    fn test_python_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("py")
            .expect("should have python language");
        let config = load_highlight_config(lang.clone(), "python")
            .expect("should load python highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "def hello(name: str) -> None:\n    print(f\"Hello {name}\")\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(!spans.is_empty(), "should produce highlight spans for Python code");
    }

    #[test]
    fn test_yaml_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("yml")
            .expect("should have yaml language");
        let config = load_highlight_config(lang.clone(), "yaml")
            .expect("should load yaml highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "name: my-app\nversion: 1.0\ndebug: true\n# comment\nitems:\n  - foo\n  - bar\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(!spans.is_empty(), "should produce highlight spans for YAML code");
    }
}
