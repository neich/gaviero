//! Regex-based markdown syntax highlighting for the source editor.
//!
//! Rendered preview uses `panels::chat_markdown::format_chat_markdown`.
//! We can't use tree-sitter-md because it requires tree-sitter 0.24 while we
//! use 0.25. Markdown syntax is regular enough that regex works well.

use super::highlight::StyledSpan;
use crate::theme::Theme;

// ── Regex-based highlighting (produces StyledSpan like tree-sitter) ──

/// Generate StyledSpans for markdown source text within a byte range.
/// Scans from the beginning of the file to correctly track code block state,
/// but only emits spans overlapping the visible byte range.
pub fn highlight_markdown(
    source: &str,
    theme: &Theme,
    byte_range: std::ops::Range<usize>,
) -> Vec<StyledSpan> {
    let mut spans = Vec::new();
    let mut in_code_block = false;
    let mut code_block_start: usize = 0;
    let bytes = source.as_bytes();
    let len = bytes.len();

    // Walk through ALL lines from the start to correctly track code block state
    let mut pos = 0;
    while pos < len {
        let line_end = memchr_newline(bytes, pos).unwrap_or(len);
        let line = &source[pos..line_end];

        let is_fence = line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");

        if is_fence {
            if in_code_block {
                // Closing fence — emit span if it overlaps visible range
                in_code_block = false;
                let block_end = line_end;
                if block_end >= byte_range.start && code_block_start < byte_range.end {
                    if let Some(style) = theme.highlight_style("markup.code.block") {
                        spans.push(StyledSpan {
                            priority: 0,
                            start_byte: code_block_start,
                            end_byte: block_end,
                            style,
                        });
                    }
                }
            } else {
                in_code_block = true;
                code_block_start = pos;
            }
        } else if !in_code_block && line_end >= byte_range.start && pos < byte_range.end {
            // Only highlight non-code-block lines in visible range
            highlight_markdown_line(line, pos, theme, &mut spans);
        }

        pos = if line_end < len { line_end + 1 } else { len };
    }

    // Handle unclosed code block
    if in_code_block && len >= byte_range.start && code_block_start < byte_range.end {
        if let Some(style) = theme.highlight_style("markup.code.block") {
            spans.push(StyledSpan {
                priority: 0,
                start_byte: code_block_start,
                end_byte: len,
                style,
            });
        }
    }

    spans.sort_by_key(|s| s.start_byte);
    spans
}

/// Find the next newline byte in the slice starting from `start`.
fn memchr_newline(bytes: &[u8], start: usize) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| start + p)
}

fn highlight_markdown_line(line: &str, offset: usize, theme: &Theme, spans: &mut Vec<StyledSpan>) {
    let trimmed = line.trim_start();

    // Headings: # ## ### etc.
    if trimmed.starts_with('#') {
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if hashes <= 6
            && trimmed
                .get(hashes..hashes + 1)
                .map_or(true, |c| c == " " || c.is_empty())
        {
            if let Some(style) = theme.highlight_style("markup.heading") {
                spans.push(StyledSpan {
                    priority: 0,
                    start_byte: offset,
                    end_byte: offset + line.len(),
                    style,
                });
            }
            return;
        }
    }

    // Block quotes: > text
    if trimmed.starts_with('>') {
        if let Some(style) = theme.highlight_style("markup.quote") {
            spans.push(StyledSpan {
                priority: 0,
                start_byte: offset,
                end_byte: offset + line.len(),
                style,
            });
        }
        return;
    }

    // List markers: - * + or 1. 2. etc.
    if trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || (trimmed.len() >= 3 && trimmed.as_bytes()[0].is_ascii_digit() && trimmed.contains(". "))
    {
        let marker_end = trimmed.find(' ').unwrap_or(0) + 1;
        let marker_start = line.len() - trimmed.len();
        if let Some(style) = theme.highlight_style("markup.list") {
            spans.push(StyledSpan {
                priority: 0,
                start_byte: offset + marker_start,
                end_byte: offset + marker_start + marker_end,
                style,
            });
        }
    }

    // Inline patterns within the line
    highlight_inline(line, offset, theme, spans);
}

fn highlight_inline(line: &str, offset: usize, theme: &Theme, spans: &mut Vec<StyledSpan>) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Inline code: `...`
        if bytes[i] == b'`' && !matches!(bytes.get(i + 1), Some(b'`')) {
            if let Some(end) = find_closing(line, i + 1, b'`') {
                if let Some(style) = theme.highlight_style("markup.code") {
                    spans.push(StyledSpan {
                        priority: 0,
                        start_byte: offset + i,
                        end_byte: offset + end + 1,
                        style,
                    });
                }
                i = end + 1;
                continue;
            }
        }

        // Bold: **...** or __...__
        if i + 1 < len
            && ((bytes[i] == b'*' && bytes[i + 1] == b'*')
                || (bytes[i] == b'_' && bytes[i + 1] == b'_'))
        {
            let marker = bytes[i];
            if let Some(end) = find_double_closing(line, i + 2, marker) {
                if let Some(style) = theme.highlight_style("markup.bold") {
                    spans.push(StyledSpan {
                        priority: 0,
                        start_byte: offset + i,
                        end_byte: offset + end + 2,
                        style,
                    });
                }
                i = end + 2;
                continue;
            }
        }

        // Italic: *...* or _..._  (but not ** or __)
        if (bytes[i] == b'*' || bytes[i] == b'_')
            && !matches!(bytes.get(i + 1), Some(b) if *b == bytes[i])
        {
            let marker = bytes[i];
            if let Some(end) = find_closing(line, i + 1, marker) {
                if let Some(style) = theme.highlight_style("markup.italic") {
                    spans.push(StyledSpan {
                        priority: 0,
                        start_byte: offset + i,
                        end_byte: offset + end + 1,
                        style,
                    });
                }
                i = end + 1;
                continue;
            }
        }

        // Links: [text](url)
        if bytes[i] == b'[' {
            if let Some(bracket_end) = find_closing(line, i + 1, b']') {
                if bracket_end + 1 < len && bytes[bracket_end + 1] == b'(' {
                    if let Some(paren_end) = find_closing(line, bracket_end + 2, b')') {
                        if let Some(style) = theme.highlight_style("markup.link") {
                            spans.push(StyledSpan {
                                priority: 0,
                                start_byte: offset + i,
                                end_byte: offset + bracket_end + 1,
                                style,
                            });
                        }
                        if let Some(style) = theme.highlight_style("markup.link.url") {
                            spans.push(StyledSpan {
                                priority: 0,
                                start_byte: offset + bracket_end + 1,
                                end_byte: offset + paren_end + 1,
                                style,
                            });
                        }
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
}

pub(crate) fn find_closing(line: &str, start: usize, marker: u8) -> Option<usize> {
    let bytes = line.as_bytes();
    for i in start..bytes.len() {
        if bytes[i] == marker && (i == 0 || bytes[i - 1] != b'\\') {
            return Some(i);
        }
    }
    None
}

pub(crate) fn find_double_closing(line: &str, start: usize, marker: u8) -> Option<usize> {
    let bytes = line.as_bytes();
    for i in start..bytes.len().saturating_sub(1) {
        if bytes[i] == marker && bytes[i + 1] == marker {
            return Some(i);
        }
    }
    None
}

#[derive(Clone)]
pub(crate) struct TextSegment {
    pub text: String,
    pub kind: SegmentKind,
}

#[derive(Clone)]
pub(crate) enum SegmentKind {
    Plain,
    Bold,
    Italic,
    Code,
    Link(#[allow(dead_code)] String), // URL retained for future link rendering
}

pub(crate) fn parse_inline(text: &str) -> Vec<TextSegment> {
    let mut segments = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current = String::new();

    while i < len {
        // Inline code
        if bytes[i] == b'`' {
            if let Some(end) = find_closing(text, i + 1, b'`') {
                flush_plain(&mut current, &mut segments);
                segments.push(TextSegment {
                    text: text[i + 1..end].to_string(),
                    kind: SegmentKind::Code,
                });
                i = end + 1;
                continue;
            }
        }

        // Bold
        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(end) = find_double_closing(text, i + 2, b'*') {
                flush_plain(&mut current, &mut segments);
                segments.push(TextSegment {
                    text: text[i + 2..end].to_string(),
                    kind: SegmentKind::Bold,
                });
                i = end + 2;
                continue;
            }
        }

        // Italic
        if bytes[i] == b'*' && !matches!(bytes.get(i + 1), Some(b'*')) {
            if let Some(end) = find_closing(text, i + 1, b'*') {
                flush_plain(&mut current, &mut segments);
                segments.push(TextSegment {
                    text: text[i + 1..end].to_string(),
                    kind: SegmentKind::Italic,
                });
                i = end + 1;
                continue;
            }
        }

        // Links: [text](url)
        if bytes[i] == b'[' {
            if let Some(bracket_end) = find_closing(text, i + 1, b']') {
                if bracket_end + 1 < len && bytes[bracket_end + 1] == b'(' {
                    if let Some(paren_end) = find_closing(text, bracket_end + 2, b')') {
                        flush_plain(&mut current, &mut segments);
                        let link_text = text[i + 1..bracket_end].to_string();
                        let url = text[bracket_end + 2..paren_end].to_string();
                        segments.push(TextSegment {
                            text: link_text,
                            kind: SegmentKind::Link(url),
                        });
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // Advance by full UTF-8 character to avoid corrupting multi-byte chars.
        // All markdown markers we check are ASCII, so non-ASCII bytes are always plain text.
        if bytes[i] < 0x80 {
            current.push(bytes[i] as char);
            i += 1;
        } else {
            // Decode the full UTF-8 char starting at byte i
            let rest = &text[i..];
            if let Some(ch) = rest.chars().next() {
                current.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }

    flush_plain(&mut current, &mut segments);
    segments
}

pub(crate) fn flush_plain(current: &mut String, segments: &mut Vec<TextSegment>) {
    if !current.is_empty() {
        segments.push(TextSegment {
            text: std::mem::take(current),
            kind: SegmentKind::Plain,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn test_heading_highlight() {
        let theme = Theme::builtin_default();
        let source = "# Hello World\nSome text\n";
        let spans = highlight_markdown(source, &theme, 0..source.len());
        assert!(!spans.is_empty(), "should produce spans for heading");
    }

    #[test]
    fn test_code_block_highlight() {
        let theme = Theme::builtin_default();
        let source = "```\nlet x = 1;\n```\n";
        let spans = highlight_markdown(source, &theme, 0..source.len());
        assert!(!spans.is_empty(), "should produce spans for code block");
    }

    #[test]
    fn test_inline_formatting() {
        let segments = parse_inline("hello **bold** and *italic* and `code`");
        assert!(segments.len() >= 5);
        assert!(segments.iter().any(|s| matches!(s.kind, SegmentKind::Bold)));
        assert!(
            segments
                .iter()
                .any(|s| matches!(s.kind, SegmentKind::Italic))
        );
        assert!(segments.iter().any(|s| matches!(s.kind, SegmentKind::Code)));
    }

    #[test]
    fn test_link_parsing() {
        let segments = parse_inline("see [example](https://example.com) here");
        assert!(
            segments
                .iter()
                .any(|s| matches!(s.kind, SegmentKind::Link(_)))
        );
    }

    #[test]
    fn test_format_preview_mixed() {
        use crate::panels::chat_markdown::format_chat_markdown;
        use ratatui::style::Style;

        let source = "# Title\n\nSome text\n\n- item 1\n- item 2\n\n> quote\n\n```\ncode\n```\n";
        let lines = format_chat_markdown(source, 80, Style::default());
        assert!(lines.len() >= 6);
    }
}
