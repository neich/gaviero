use anyhow::{Result, bail};
use gaviero_core::{Language, Node, Query, QueryCursor, Tree};
use ratatui::style::{Color, Modifier, Style};
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
    /// Number of dot-segments in the capture group name (e.g. "string" = 0,
    /// "string.special.key" = 2). Used as a tiebreaker in sorting: lower
    /// priority (less specific) comes first, so the last — most specific —
    /// span wins in the rendering loop.
    pub priority: usize,
}

/// Load highlight queries for a language.
pub fn load_highlight_config(language: Language, lang_name: &str) -> Result<HighlightConfig> {
    let scm = find_query_file(lang_name, "highlights.scm")?;
    let query = Query::new(&language, &scm)
        .map_err(|e| anyhow::anyhow!("tree-sitter query error for {}: {}", lang_name, e))?;
    let group_names = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
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
        "javascript" => {
            Ok(include_str!("../../../../queries/javascript/highlights.scm").to_string())
        }
        "typescript" => {
            Ok(include_str!("../../../../queries/typescript/highlights.scm").to_string())
        }
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
        "kotlin" => Ok(include_str!("../../../../queries/kotlin/highlights.scm").to_string()),
        "gaviero" => Ok(include_str!("../../../../queries/gaviero/highlights.scm").to_string()),
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
    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        for capture in m.captures {
            let group = &config.group_names[capture.index as usize];
            if let Some(style) = theme.highlight_style(group) {
                let node = capture.node;
                // Priority = number of dot-separators in the group name.
                // "string" → 0, "string.special" → 1, "string.special.key" → 2.
                let priority = group.chars().filter(|&c| c == '.').count();
                spans.push(StyledSpan {
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    style,
                    priority,
                });
            }
        }
    }

    // Sort: by start_byte ascending, then by span width descending (wider =
    // less specific = first), then by priority ascending (less specific first).
    // The rendering loop applies spans in order and the LAST match wins, so
    // more-specific spans (higher priority, narrower) end up last and win.
    spans.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| {
                let a_width = a.end_byte - a.start_byte;
                let b_width = b.end_byte - b.start_byte;
                b_width.cmp(&a_width) // wider first
            })
            .then_with(|| a.priority.cmp(&b.priority)) // less specific first
    });

    // Append error spans AFTER the sorted syntax spans so they render last
    // and visually override normal colors (red + underline = parse error marker).
    let error_style = Style::default()
        .fg(Color::Rgb(224, 108, 117))
        .add_modifier(Modifier::UNDERLINED);
    collect_error_spans(tree.root_node(), &visible_range, &mut spans, error_style);

    spans
}

/// Recursively collect byte ranges of ERROR and MISSING nodes within the
/// visible range and push a styled span for each into `out`.
fn collect_error_spans(
    node: Node<'_>,
    visible_range: &std::ops::Range<usize>,
    out: &mut Vec<StyledSpan>,
    style: Style,
) {
    // Skip nodes entirely outside the visible viewport
    if node.start_byte() >= visible_range.end || node.end_byte() <= visible_range.start {
        return;
    }
    if node.is_error() || node.is_missing() {
        out.push(StyledSpan {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            style,
            priority: usize::MAX, // always wins over syntax highlights
        });
        // Don't descend — the whole subtree is already covered by this span
        return;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_error_spans(child, visible_range, out, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_query_file_fallback() {
        let scm = find_query_file("rust", "highlights.scm");
        assert!(scm.is_ok(), "rust highlights.scm should be bundled");
        let content = scm.unwrap();
        assert!(
            content.contains("comment"),
            "should contain comment patterns"
        );
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
        let config =
            load_highlight_config(lang.clone(), "rust").expect("should load rust highlights");
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
        assert!(
            !spans.is_empty(),
            "should produce highlight spans for Rust code"
        );
    }

    #[test]
    fn test_python_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("py")
            .expect("should have python language");
        let config =
            load_highlight_config(lang.clone(), "python").expect("should load python highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "def hello(name: str) -> None:\n    print(f\"Hello {name}\")\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(
            !spans.is_empty(),
            "should produce highlight spans for Python code"
        );
    }

    #[test]
    fn test_yaml_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("yml")
            .expect("should have yaml language");
        let config =
            load_highlight_config(lang.clone(), "yaml").expect("should load yaml highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source =
            "name: my-app\nversion: 1.0\ndebug: true\n# comment\nitems:\n  - foo\n  - bar\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(
            !spans.is_empty(),
            "should produce highlight spans for YAML code"
        );
    }

    #[test]
    fn test_kotlin_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("kt")
            .expect("should have kotlin language");
        let config =
            load_highlight_config(lang.clone(), "kotlin").expect("should load kotlin highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "package com.example\n\nfun main() {\n    val x = 42\n    println(\"hello\")\n}\n\nclass Foo(val name: String)\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(
            !spans.is_empty(),
            "should produce highlight spans for Kotlin code"
        );
    }

    #[test]
    fn test_gaviero_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("gaviero")
            .expect("should have gaviero language");
        let config =
            load_highlight_config(lang.clone(), "gaviero").expect("should load gaviero highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "client opus { tier expensive model \"claude-opus-4-6\" }\n\nagent scan {\n    description \"Scan\"\n    client opus\n    memory {\n        read_ns [\"shared\"]\n        importance 0.9\n    }\n}\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        assert!(
            !spans.is_empty(),
            "should produce highlight spans for gaviero code"
        );

        // Keywords should be purple
        let keyword_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.style.fg == Some(ratatui::style::Color::Rgb(198, 120, 221)))
            .collect();
        assert!(
            !keyword_spans.is_empty(),
            "should produce purple spans for gaviero keywords"
        );

        // Strings should be green
        let string_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.style.fg == Some(ratatui::style::Color::Rgb(152, 195, 121)))
            .collect();
        assert!(
            !string_spans.is_empty(),
            "should produce green spans for gaviero strings"
        );
    }

    #[test]
    fn test_json_highlight_pipeline() {
        use crate::theme::Theme;

        let lang = gaviero_core::tree_sitter::language_for_extension("json")
            .expect("should have json language");
        let config =
            load_highlight_config(lang.clone(), "json").expect("should load json highlights");

        let mut parser = gaviero_core::Parser::new();
        parser.set_language(&lang).unwrap();
        let source = "{\n  \"name\": \"Alice\",\n  \"age\": 30,\n  \"active\": true\n}\n";
        let rope = ropey::Rope::from_str(source);
        let tree = parser.parse(source, None).unwrap();

        let theme = Theme::builtin_default();
        let spans = run_highlights(&tree, &rope, &config, &theme, 0..source.len());
        eprintln!("JSON spans ({}):", spans.len());
        for s in &spans {
            let text = &source[s.start_byte..s.end_byte];
            eprintln!(
                "  {:?}..{:?} {:?} => {:?}",
                s.start_byte, s.end_byte, text, s.style
            );
        }

        // String values (non-key strings) — green
        let string_val_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.style.fg == Some(ratatui::style::Color::Rgb(152, 195, 121)))
            .collect();
        assert!(
            !string_val_spans.is_empty(),
            "should produce green spans for JSON string values"
        );

        // Keys must be more specific (red/pink) and must come AFTER generic string (green)
        // so that they win in the last-wins rendering loop.
        let key_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.style.fg == Some(ratatui::style::Color::Rgb(224, 108, 117)))
            .collect();
        assert!(
            !key_spans.is_empty(),
            "should produce red spans for JSON keys"
        );
        // For each key span there must be a green span at the same range that
        // comes BEFORE it in the list (so the red span wins).
        for key_span in &key_spans {
            let green_before = spans
                .iter()
                .position(|s| {
                    s.start_byte == key_span.start_byte
                        && s.end_byte == key_span.end_byte
                        && s.style.fg == Some(ratatui::style::Color::Rgb(152, 195, 121))
                })
                .and_then(|gi| {
                    spans
                        .iter()
                        .position(|s| std::ptr::eq(s, *key_span))
                        .map(|ki| gi < ki)
                });
            assert_eq!(
                green_before,
                Some(true),
                "red key span at {}..{} must follow its green string span so red wins",
                key_span.start_byte,
                key_span.end_byte
            );
        }

        // Numbers — orange
        let number_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.style.fg == Some(ratatui::style::Color::Rgb(209, 154, 102)))
            .collect();
        assert!(
            !number_spans.is_empty(),
            "should produce orange spans for JSON numbers"
        );
    }
}
