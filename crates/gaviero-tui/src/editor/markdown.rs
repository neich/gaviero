//! Regex-based markdown syntax highlighting for the source editor.
//!
//! Rendered preview uses `panels::chat_markdown::format_chat_markdown`.
//! We can't use tree-sitter-md because it requires tree-sitter 0.24 while we
//! use 0.25. Markdown syntax is regular enough that regex works well.

use std::path::{Path, PathBuf};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextSegment {
    pub text: String,
    pub kind: SegmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    Plain,
    Bold,
    Italic,
    Code,
    Link(String),
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

        // Autolinks: <https://…>, <http://…>, <mailto:…>
        if bytes[i] == b'<' {
            if let Some(end) = find_closing(text, i + 1, b'>') {
                let inner = &text[i + 1..end];
                if is_autolink_url(inner) {
                    flush_plain(&mut current, &mut segments);
                    segments.push(TextSegment {
                        text: inner.to_string(),
                        kind: SegmentKind::Link(inner.to_string()),
                    });
                    i = end + 1;
                    continue;
                }
            }
        }

        // Bare http(s) URLs (not already consumed as a markdown dest).
        if is_bare_url_start(bytes, i) {
            let (url, end) = take_bare_url(text, i);
            if end > i {
                flush_plain(&mut current, &mut segments);
                segments.push(TextSegment {
                    text: url.clone(),
                    kind: SegmentKind::Link(url),
                });
                i = end;
                continue;
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

fn is_autolink_url(inner: &str) -> bool {
    starts_with_ignore_ascii_case(inner.as_bytes(), b"https://")
        || starts_with_ignore_ascii_case(inner.as_bytes(), b"http://")
        || starts_with_ignore_ascii_case(inner.as_bytes(), b"mailto:")
}

fn starts_with_ignore_ascii_case(hay: &[u8], prefix: &[u8]) -> bool {
    hay.len() >= prefix.len() && hay[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn is_bare_url_start(bytes: &[u8], i: usize) -> bool {
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return false;
    }
    starts_with_ignore_ascii_case(&bytes[i..], b"https://")
        || starts_with_ignore_ascii_case(&bytes[i..], b"http://")
}

fn take_bare_url(text: &str, start: usize) -> (String, usize) {
    let rest = &text[start..];
    let mut byte_len = 0;
    for ch in rest.chars() {
        if ch.is_whitespace() || ch == '<' || ch == '>' {
            break;
        }
        byte_len += ch.len_utf8();
    }
    let mut end = start + byte_len;
    while end > start {
        let Some(last) = text[..end].chars().next_back() else {
            break;
        };
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}') {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    if end <= start + "http://x".len() {
        return (String::new(), start);
    }
    (text[start..end].to_string(), end)
}

/// Where a markdown link destination should go when followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownLinkTarget {
    External(String),
    Local {
        path: PathBuf,
        fragment: Option<String>,
    },
    Fragment(String),
}

/// Classify `[text](dest)` / autolink destinations. Unknown or unsafe schemes
/// (`javascript:`, `data:`, …) return `None`.
pub(crate) fn classify_markdown_link(raw: &str) -> Option<MarkdownLinkTarget> {
    let dest = normalize_link_dest(raw);
    if dest.is_empty() {
        return None;
    }
    if dest.starts_with('#') {
        let frag = dest.trim_start_matches('#');
        if frag.is_empty() {
            return None;
        }
        return Some(MarkdownLinkTarget::Fragment(percent_decode(frag)));
    }
    let lower = dest.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return None;
    }
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
    {
        return Some(MarkdownLinkTarget::External(dest.to_string()));
    }
    if lower.starts_with("file:") {
        let (path_str, fragment) = split_fragment(dest);
        let path = parse_file_url(&path_str)?;
        return Some(MarkdownLinkTarget::Local { path, fragment });
    }
    if url_scheme(dest).is_some_and(|scheme| scheme.len() > 1) {
        return None;
    }
    let (path_str, fragment) = split_fragment(dest);
    Some(MarkdownLinkTarget::Local {
        path: PathBuf::from(percent_decode(&path_str)),
        fragment,
    })
}

/// Resolve a relative markdown path against the current file, then workspace
/// roots if that candidate is missing.
pub(crate) fn resolve_local_markdown_path(
    dest: &Path,
    base_dir: Option<&Path>,
    workspace_roots: &[&Path],
) -> PathBuf {
    let from_file = if dest.is_absolute() {
        None
    } else {
        Some(match base_dir {
            Some(dir) => dir.join(dest),
            None => dest.to_path_buf(),
        })
    };
    if let Some(path) = &from_file
        && path.exists()
    {
        return path.clone();
    }
    if dest.is_absolute() && dest.exists() {
        return dest.to_path_buf();
    }
    let dest_str = dest.to_string_lossy();
    let stripped = dest_str.trim_start_matches(['/', '\\']);
    for root in workspace_roots {
        let candidate = root.join(stripped);
        if candidate.exists() {
            return candidate;
        }
    }
    from_file.unwrap_or_else(|| dest.to_path_buf())
}

pub(crate) fn heading_slug(text: &str) -> String {
    let mut out = String::new();
    let mut pending_hyphen = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(ch.to_ascii_lowercase());
        } else if ch == ' ' || ch == '-' {
            pending_hyphen = true;
        }
    }
    out
}

pub(crate) fn strip_heading_marker(text: &str) -> &str {
    text.trim_start()
        .trim_start_matches(['█', '▌', '▎'])
        .trim_start()
}

fn normalize_link_dest(raw: &str) -> &str {
    let s = raw.trim();
    if let Some(inner) = s.strip_prefix('<')
        && let Some(end) = inner.find('>')
    {
        return inner[..end].trim();
    }
    if let Some(space) = s.find(char::is_whitespace) {
        let rest = s[space..].trim_start();
        if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('(') {
            return s[..space].trim();
        }
    }
    s
}

fn url_scheme(s: &str) -> Option<&str> {
    let colon = s.find(':')?;
    let cand = &s[..colon];
    if cand.is_empty() || !cand.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(cand)
}

fn split_fragment(s: &str) -> (String, Option<String>) {
    match s.find('#') {
        Some(i) => {
            let frag = percent_decode(&s[i + 1..]);
            (
                s[..i].to_string(),
                if frag.is_empty() { None } else { Some(frag) },
            )
        }
        None => (s.to_string(), None),
    }
}

fn parse_file_url(dest: &str) -> Option<PathBuf> {
    let rest = dest.strip_prefix("file:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let path_part = if rest.starts_with('/') {
        rest
    } else if let Some(slash) = rest.find('/') {
        &rest[slash..]
    } else {
        rest
    };
    let decoded = percent_decode(path_part);
    #[cfg(windows)]
    {
        let t = decoded.trim_start_matches('/');
        if t.len() >= 2 {
            let bytes = t.as_bytes();
            if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                return Some(PathBuf::from(t));
            }
        }
    }
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = hex_byte(bytes[i + 1], bytes[i + 2])
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_byte(h: u8, l: u8) -> Option<u8> {
    Some((hex_val(h)? << 4) | hex_val(l)?)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
        let link = segments
            .iter()
            .find(|s| matches!(s.kind, SegmentKind::Link(_)))
            .expect("parsed [text](url)");
        assert_eq!(link.text, "example");
        assert_eq!(link.kind, SegmentKind::Link("https://example.com".into()));
    }

    #[test]
    fn parse_inline_autolink_and_bare_url() {
        let auto = parse_inline("go <https://example.com/a> now");
        assert!(auto.iter().any(|s| {
            s.kind == SegmentKind::Link("https://example.com/a".into())
                && s.text == "https://example.com/a"
        }));

        let bare = parse_inline("see https://example.com/b.");
        assert!(
            bare.iter()
                .any(|s| { s.kind == SegmentKind::Link("https://example.com/b".into()) })
        );
    }

    #[test]
    fn classify_http_local_fragment_and_rejects_javascript() {
        assert_eq!(
            classify_markdown_link("https://example.com/x"),
            Some(MarkdownLinkTarget::External("https://example.com/x".into()))
        );
        assert_eq!(
            classify_markdown_link("<docs/foo.md> \"title\""),
            Some(MarkdownLinkTarget::Local {
                path: PathBuf::from("docs/foo.md"),
                fragment: None,
            })
        );
        assert_eq!(
            classify_markdown_link("README.md#Install"),
            Some(MarkdownLinkTarget::Local {
                path: PathBuf::from("README.md"),
                fragment: Some("Install".into()),
            })
        );
        assert_eq!(
            classify_markdown_link("#heading-id"),
            Some(MarkdownLinkTarget::Fragment("heading-id".into()))
        );
        assert_eq!(classify_markdown_link("javascript:alert(1)"), None);
        assert_eq!(classify_markdown_link("data:text/html,x"), None);
    }

    #[test]
    fn classify_strips_title_and_percent_decodes_local_paths() {
        assert_eq!(
            classify_markdown_link("https://example.com/x \"docs\""),
            Some(MarkdownLinkTarget::External("https://example.com/x".into()))
        );
        assert_eq!(
            classify_markdown_link("dir%20name/a.md"),
            Some(MarkdownLinkTarget::Local {
                path: PathBuf::from("dir name/a.md"),
                fragment: None,
            })
        );
    }

    #[test]
    fn heading_slug_github_style() {
        assert_eq!(heading_slug("Foo Bar!"), "foo-bar");
        assert_eq!(heading_slug("API"), "api");
        assert_eq!(strip_heading_marker("█ Title Here"), "Title Here");
    }

    #[test]
    fn resolve_local_markdown_path_prefers_file_dir_then_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let file_dir = tmp.path().join("notes");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        std::fs::write(file_dir.join("local.md"), "x").unwrap();
        std::fs::write(ws.join("docs/root.md"), "y").unwrap();

        let hit =
            resolve_local_markdown_path(Path::new("local.md"), Some(&file_dir), &[ws.as_path()]);
        assert_eq!(hit, file_dir.join("local.md"));

        let rooted = resolve_local_markdown_path(
            Path::new("/docs/root.md"),
            Some(&file_dir),
            &[ws.as_path()],
        );
        assert_eq!(rooted, ws.join("docs/root.md"));
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
