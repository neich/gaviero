//! Markdown formatting for the chat panel and in-editor preview.
//!
//! Converts markdown text into a sequence of styled lines (`ChatLine`), each
//! composed of one or more `StyledSegment`s so a single visual line can carry
//! mixed inline styling (bold, italic, inline code, links) on top of its base
//! style. Handles tables, headings, code blocks, lists, and horizontal rules.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::editor::markdown::{SegmentKind, parse_inline};
use crate::theme;

/// A run of contiguous characters sharing a single style.
#[derive(Clone)]
pub struct StyledSegment {
    pub text: String,
    pub style: Style,
}

/// A single rendered line for the chat panel.
///
/// Holds one or more styled segments so a single visual line can mix inline
/// styles (bold, italic, inline code, links) on top of the line's base style.
pub struct ChatLine {
    pub segments: Vec<StyledSegment>,
}

impl ChatLine {
    /// Build a line with a single style covering the whole text.
    pub fn single(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        if text.is_empty() {
            Self {
                segments: Vec::new(),
            }
        } else {
            Self {
                segments: vec![StyledSegment { text, style }],
            }
        }
    }

    /// Plain-text concatenation of all segments (no style information).
    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    /// Style of the first segment, used as a fallback for whole-line
    /// effects like the browse-mode background highlight.
    pub fn primary_style(&self) -> Style {
        self.segments
            .first()
            .map(|s| s.style)
            .unwrap_or_default()
    }
}

/// Format markdown content into styled lines for the chat panel.
///
/// Processes tables with box-drawing characters, renders headings with visual
/// markers, indents code blocks, formats lists, and converts inline
/// `**bold**`, `*italic*`, `` `code` ``, and `[text](url)` markers into styled
/// segments that render with the corresponding visual attributes.
pub fn format_chat_markdown(text: &str, width: usize, base_style: Style) -> Vec<ChatLine> {
    let mut output = Vec::new();
    // Expand tabs to 4 spaces to prevent terminal rendering artifacts.
    // Tab characters in ratatui cells cause cursor jumps that corrupt display.
    let expanded_text = text.replace('\t', "    ");
    let src_lines: Vec<&str> = expanded_text.lines().collect();
    let mut i = 0;
    let mut in_code_block = false;

    let code_style = Style::default().fg(theme::CODE_GREEN);
    let heading_style = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD);
    let quote_style = Style::default().fg(theme::TOOL_DIM);
    let rule_style = Style::default().fg(theme::BORDER_DIM);
    let table_border_style = Style::default().fg(theme::BORDER_DIM);
    let table_header_style = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD);
    let reasoning_style = Style::default()
        .fg(theme::REASONING_FG)
        .add_modifier(Modifier::ITALIC);
    let mut in_thinking_block = false;

    while i < src_lines.len() {
        let line = src_lines[i];
        let trimmed = line.trim();

        // Thinking / reasoning block tags: <think> ... </think>
        if trimmed == "<think>" || trimmed.starts_with("<think>") {
            in_thinking_block = true;
            // If there's content after <think> on the same line, render it
            let after_tag = trimmed.strip_prefix("<think>").unwrap_or("");
            let after_tag = after_tag.strip_suffix("</think>").unwrap_or(after_tag);
            if !after_tag.is_empty() {
                push_inline_wrapped(
                    &mut output,
                    after_tag.trim(),
                    "  ",
                    "  ",
                    width,
                    reasoning_style,
                );
            }
            if trimmed.ends_with("</think>") {
                in_thinking_block = false;
            }
            i += 1;
            continue;
        }
        if trimmed == "</think>" || trimmed.ends_with("</think>") {
            // Render any text before the closing tag
            let before_tag = trimmed.strip_suffix("</think>").unwrap_or("");
            if !before_tag.is_empty() {
                push_inline_wrapped(
                    &mut output,
                    before_tag.trim(),
                    "  ",
                    "  ",
                    width,
                    reasoning_style,
                );
            }
            in_thinking_block = false;
            i += 1;
            continue;
        }
        if in_thinking_block {
            if trimmed.is_empty() {
                output.push(ChatLine::single(String::new(), reasoning_style));
            } else {
                push_inline_wrapped(
                    &mut output,
                    trimmed,
                    "  ",
                    "  ",
                    width,
                    reasoning_style,
                );
            }
            i += 1;
            continue;
        }

        // Code block fences
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            i += 1;
            continue;
        }

        if in_code_block {
            let budget = width.saturating_sub(2).max(1);
            for wl in crate::widgets::render_utils::word_wrap(line, budget) {
                output.push(ChatLine::single(format!("  {}", wl), code_style));
            }
            i += 1;
            continue;
        }

        // Table detection: current line has |, next line is separator
        if is_table_row(trimmed)
            && i + 1 < src_lines.len()
            && is_table_separator(src_lines[i + 1].trim())
        {
            let mut rows: Vec<Vec<String>> = Vec::new();
            rows.push(parse_table_cells(trimmed));
            i += 2; // skip header + separator
            while i < src_lines.len() && is_table_row(src_lines[i].trim()) {
                rows.push(parse_table_cells(src_lines[i].trim()));
                i += 1;
            }
            render_table(
                &rows,
                width,
                base_style,
                table_border_style,
                table_header_style,
                &mut output,
            );
            continue;
        }

        // Empty line
        if trimmed.is_empty() {
            output.push(ChatLine::single(String::new(), base_style));
            i += 1;
            continue;
        }

        // Horizontal rule
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().filter(|c| !c.is_whitespace()).count() >= 3
            && trimmed
                .chars()
                .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
        {
            let rule: String = "─".repeat(width.min(60));
            output.push(ChatLine::single(rule, rule_style));
            i += 1;
            continue;
        }

        // Headings
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count().min(6);
            if trimmed
                .get(level..level + 1)
                .map_or(true, |c| c == " " || c.is_empty())
            {
                let content = strip_inline_markers(trimmed[level..].trim());
                let marker = match level {
                    1 => "█ ",
                    2 => "▌ ",
                    _ => "▎ ",
                };
                for wl in crate::widgets::render_utils::word_wrap(
                    &format!("{}{}", marker, content),
                    width,
                ) {
                    output.push(ChatLine::single(wl, heading_style));
                }
                i += 1;
                continue;
            }
        }

        // Block quotes
        if trimmed.starts_with('>') {
            push_inline_wrapped(
                &mut output,
                trimmed[1..].trim_start(),
                "│ ",
                "│ ",
                width,
                quote_style,
            );
            i += 1;
            continue;
        }

        // Unordered list items
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let indent = (line.len() - line.trim_start().len()) / 2;
            let pad: String = "  ".repeat(indent);
            push_inline_wrapped(
                &mut output,
                &trimmed[2..],
                &format!("{}• ", pad),
                &format!("{}  ", pad),
                width,
                base_style,
            );
            i += 1;
            continue;
        }

        // Ordered list items
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                let marker = format!("{}. ", prefix);
                let cont_indent: String = " ".repeat(marker.chars().count());
                push_inline_wrapped(
                    &mut output,
                    &trimmed[dot_pos + 2..],
                    &marker,
                    &cont_indent,
                    width,
                    base_style,
                );
                i += 1;
                continue;
            }
        }

        // Tool call and file proposal markers: [Read ...], [Write ...], [wrote ...], etc.
        // Style with dim color to visually distinguish from regular text
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains("](") {
            let marker_style = Style::default().fg(theme::TOOL_DIM);
            for wl in crate::widgets::render_utils::word_wrap(&format!("  {}", trimmed), width) {
                output.push(ChatLine::single(wl, marker_style));
            }
            i += 1;
            continue;
        }

        // Regular text: preserve inline styling (bold/italic/code/links).
        push_inline_wrapped(&mut output, trimmed, "", "", width, base_style);
        i += 1;
    }

    reflow_overwide_lines(output, width)
}

/// Re-wrap any rendered line that still exceeds `width` display columns.
///
/// Catches formatted output that can exceed the panel budget after layout; uses
/// the line's primary style for the wrapped parts. Table rows are left intact —
/// they use multiline cells instead of post-hoc line breaking.
fn reflow_overwide_lines(lines: Vec<ChatLine>, width: usize) -> Vec<ChatLine> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let text = line.text();
        if UnicodeWidthStr::width(text.as_str()) <= width || is_table_rendered_line(&text) {
            out.push(line);
            continue;
        }
        let style = line.primary_style();
        for wl in crate::widgets::render_utils::word_wrap(&text, width) {
            out.push(ChatLine::single(wl, style));
        }
    }
    out
}

/// True for box-drawn table borders and data rows (already width-constrained).
fn is_table_rendered_line(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let first = t.chars().next().unwrap_or(' ');
    matches!(first, '│' | '┌' | '├' | '└')
}

/// Word-wrap a markdown text fragment with inline styling preserved, prepending
/// `first_prefix` to the first wrapped line and `cont_prefix` to subsequent
/// lines. Prefixes inherit `base_style`.
fn push_inline_wrapped(
    output: &mut Vec<ChatLine>,
    text: &str,
    first_prefix: &str,
    cont_prefix: &str,
    width: usize,
    base_style: Style,
) {
    let inline = parse_inline_styled(text, base_style);

    // Width budget shrinks by the prefix display width on each wrapped line.
    let first_prefix_w = UnicodeWidthStr::width(first_prefix);
    let cont_prefix_w = UnicodeWidthStr::width(cont_prefix);
    let first_budget = width.saturating_sub(first_prefix_w).max(1);
    let cont_budget = width.saturating_sub(cont_prefix_w).max(1);

    let wrapped = word_wrap_segments(&inline, first_budget, cont_budget);

    if wrapped.is_empty() {
        if !first_prefix.is_empty() {
            output.push(ChatLine::single(first_prefix.to_string(), base_style));
        } else {
            output.push(ChatLine::single(String::new(), base_style));
        }
        return;
    }

    for (j, wrapped_segments) in wrapped.into_iter().enumerate() {
        let prefix = if j == 0 { first_prefix } else { cont_prefix };
        let mut segments: Vec<StyledSegment> = Vec::with_capacity(wrapped_segments.len() + 1);
        if !prefix.is_empty() {
            segments.push(StyledSegment {
                text: prefix.to_string(),
                style: base_style,
            });
        }
        segments.extend(wrapped_segments);
        if segments.is_empty() {
            output.push(ChatLine::single(String::new(), base_style));
        } else {
            output.push(ChatLine { segments });
        }
    }
}

/// Convert markdown inline segments into styled segments, mapping each marker
/// kind to a concrete `Style` on top of `base_style`.
fn parse_inline_styled(text: &str, base_style: Style) -> Vec<StyledSegment> {
    // Colors mirror the source-editor markdown highlights in `theme.rs`
    // (`markup.bold`, `markup.italic`, `markup.code`, `markup.link`) so the
    // preview reads the same as the source view. The BOLD/ITALIC text
    // modifiers alone render too subtly on many terminals — the visible
    // distinction comes from the colour change.
    const BOLD_FG: ratatui::style::Color = ratatui::style::Color::Rgb(229, 192, 123);
    const ITALIC_FG: ratatui::style::Color = ratatui::style::Color::Rgb(198, 120, 221);

    parse_inline(text)
        .into_iter()
        .filter_map(|seg| {
            if seg.text.is_empty() {
                return None;
            }
            let style = match seg.kind {
                SegmentKind::Plain => base_style,
                SegmentKind::Bold => base_style.fg(BOLD_FG).add_modifier(Modifier::BOLD),
                SegmentKind::Italic => base_style.fg(ITALIC_FG).add_modifier(Modifier::ITALIC),
                SegmentKind::Code => Style::default().fg(theme::CODE_GREEN),
                SegmentKind::Link(_) => base_style
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::UNDERLINED),
            };
            Some(StyledSegment {
                text: seg.text,
                style,
            })
        })
        .collect()
}

/// Word-wrap styled segments into multiple lines, preserving styles.
///
/// Each output line is a sequence of segments (style runs) covering exactly the
/// characters that fit within the line budget. `first_width` is the budget for
/// the first wrapped line and `cont_width` for subsequent continuation lines,
/// so the caller can reserve room for prefix indents.
fn word_wrap_segments(
    segments: &[StyledSegment],
    first_width: usize,
    cont_width: usize,
) -> Vec<Vec<StyledSegment>> {
    // Flatten to (char, style) pairs, expanding tabs to 4 spaces to match
    // `render_utils::word_wrap`'s behaviour.
    let mut chars: Vec<(char, Style)> = Vec::new();
    for seg in segments {
        for ch in seg.text.chars() {
            if ch == '\t' {
                for _ in 0..4 {
                    chars.push((' ', seg.style));
                }
            } else {
                chars.push((ch, seg.style));
            }
        }
    }

    if chars.is_empty() {
        return Vec::new();
    }

    let mut result: Vec<Vec<StyledSegment>> = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let budget = if result.is_empty() {
            first_width
        } else {
            cont_width
        }
        .max(1);

        let mut display_w = 0usize;
        let mut end = start;
        while end < chars.len() {
            let cw = UnicodeWidthChar::width(chars[end].0).unwrap_or(1);
            if display_w + cw > budget {
                break;
            }
            display_w += cw;
            end += 1;
        }
        if end == start {
            end = start + 1;
        }

        let break_at = if end < chars.len() {
            let mut bp = end;
            while bp > start && chars[bp].0 != ' ' {
                bp -= 1;
            }
            if bp == start { end } else { bp + 1 }
        } else {
            end
        };

        // Trim trailing spaces on non-final wrapped lines for visual cleanliness.
        let mut line_end = break_at;
        if break_at < chars.len() {
            while line_end > start && chars[line_end - 1].0 == ' ' {
                line_end -= 1;
            }
        }

        result.push(coalesce_segments(&chars[start..line_end]));
        start = break_at;
    }

    result
}

/// Group consecutive characters with the same style into segments.
fn coalesce_segments(chars: &[(char, Style)]) -> Vec<StyledSegment> {
    let mut segments: Vec<StyledSegment> = Vec::new();
    let mut iter = chars.iter();
    if let Some(&(ch, style)) = iter.next() {
        let mut current_text = String::new();
        let mut current_style = style;
        current_text.push(ch);
        for &(ch, style) in iter {
            if style == current_style {
                current_text.push(ch);
            } else {
                segments.push(StyledSegment {
                    text: std::mem::take(&mut current_text),
                    style: current_style,
                });
                current_text.push(ch);
                current_style = style;
            }
        }
        if !current_text.is_empty() {
            segments.push(StyledSegment {
                text: current_text,
                style: current_style,
            });
        }
    }
    segments
}

/// Paint pre-formatted markdown lines into a ratatui buffer (editor preview pane).
pub fn render_lines_to_buffer(
    lines: &[ChatLine],
    area: Rect,
    buf: &mut Buffer,
    scroll_top: usize,
    clear_style: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    for row in 0..area.height as usize {
        let line_idx = scroll_top + row;
        let y = area.y + row as u16;

        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(' ').set_style(clear_style);
            }
        }

        if line_idx >= lines.len() {
            continue;
        }

        let line = &lines[line_idx];
        let x_start = area.x.saturating_add(1);
        let mut cx = x_start;
        for seg in &line.segments {
            cx = crate::widgets::render_utils::write_text(
                buf,
                cx,
                y,
                area.right(),
                &seg.text,
                seg.style,
            );
            if cx >= area.right() {
                break;
            }
        }
    }
}

// ── Table formatting ─────────────────────────────────────────

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
        && trimmed.contains('-')
}

fn parse_table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_matches('|');
    trimmed
        .split('|')
        .map(|cell| strip_inline_markers(cell.trim()))
        .collect()
}

fn render_table(
    rows: &[Vec<String>],
    max_width: usize,
    base_style: Style,
    border_style: Style,
    header_style: Style,
    output: &mut Vec<ChatLine>,
) {
    if rows.is_empty() {
        return;
    }

    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return;
    }

    // Compute column widths
    let mut col_widths: Vec<usize> = vec![0; num_cols];
    for row in rows {
        for (j, cell) in row.iter().enumerate() {
            if j < num_cols {
                col_widths[j] = col_widths[j].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    // Cap widths to fit within max_width
    // Total: borders (num_cols + 1) + padding (num_cols * 2) + content
    let overhead = num_cols + 1 + num_cols * 2;
    let content_budget = max_width.saturating_sub(overhead);
    let total_content: usize = col_widths.iter().sum();
    if total_content > content_budget {
        if content_budget > 0 {
            // Proportionally shrink columns
            for w in &mut col_widths {
                *w = (*w * content_budget / total_content).max(1);
            }
        } else {
            // Panel too narrow for the table overhead — force minimal columns.
            for w in &mut col_widths {
                *w = 1;
            }
        }
    }

    // Build box-drawing lines
    let top = build_table_border(&col_widths, '┌', '┬', '┐', '─');
    let mid = build_table_border(&col_widths, '├', '┼', '┤', '─');
    let bot = build_table_border(&col_widths, '└', '┴', '┘', '─');

    output.push(ChatLine::single(top, border_style));

    for (row_idx, row) in rows.iter().enumerate() {
        let style = if row_idx == 0 {
            header_style
        } else {
            base_style
        };

        let wrapped_cells: Vec<Vec<String>> = col_widths
            .iter()
            .enumerate()
            .map(|(j, w)| {
                let cell = row.get(j).map(|s| s.as_str()).unwrap_or("");
                wrap_cell_content(cell, *w)
            })
            .collect();
        let row_height = wrapped_cells
            .iter()
            .map(|c| c.len())
            .max()
            .unwrap_or(1);

        for line_idx in 0..row_height {
            let line = build_table_row_line(&wrapped_cells, &col_widths, line_idx);
            output.push(ChatLine::single(line, style));
        }

        if row_idx == 0 && rows.len() > 1 {
            output.push(ChatLine::single(mid.clone(), border_style));
        }
    }

    output.push(ChatLine::single(bot, border_style));
}

/// Word-wrap cell text to `col_width` display columns; empty cells yield one blank line.
fn wrap_cell_content(cell: &str, col_width: usize) -> Vec<String> {
    let budget = col_width.max(1);
    if cell.is_empty() {
        return vec![String::new()];
    }
    crate::widgets::render_utils::word_wrap(cell, budget)
}

/// Pad a single cell line to exactly `col_width` display columns.
fn pad_cell_line(line: &str, col_width: usize) -> String {
    let display_w = UnicodeWidthStr::width(line);
    if display_w >= col_width {
        return line.to_string();
    }
    let mut padded = line.to_string();
    for _ in 0..(col_width - display_w) {
        padded.push(' ');
    }
    padded
}

/// Build one visual row of a table at `line_idx` across wrapped cell lines.
fn build_table_row_line(
    wrapped_cells: &[Vec<String>],
    col_widths: &[usize],
    line_idx: usize,
) -> String {
    let mut line = String::new();
    for (j, w) in col_widths.iter().enumerate() {
        line.push('│');
        line.push(' ');
        let cell_line = wrapped_cells
            .get(j)
            .and_then(|lines| lines.get(line_idx))
            .map(|s| s.as_str())
            .unwrap_or("");
        line.push_str(&pad_cell_line(cell_line, *w));
        line.push(' ');
    }
    line.push('│');
    line
}

fn build_table_border(
    col_widths: &[usize],
    left: char,
    mid: char,
    right: char,
    fill: char,
) -> String {
    let mut s = String::new();
    for (j, w) in col_widths.iter().enumerate() {
        s.push(if j == 0 { left } else { mid });
        // +2 for padding on each side of cell content
        for _ in 0..(*w + 2) {
            s.push(fill);
        }
    }
    s.push(right);
    s
}

// ── Inline marker stripping ─────────────────────────────────

/// Strip markdown inline formatting markers, returning plain text.
fn strip_inline_markers(text: &str) -> String {
    let segments = parse_inline(text);
    segments.into_iter().map(|s| s.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_inline_markers() {
        assert_eq!(strip_inline_markers("**bold** text"), "bold text");
        assert_eq!(strip_inline_markers("*italic* text"), "italic text");
        assert_eq!(strip_inline_markers("`code` text"), "code text");
    }

    #[test]
    fn test_table_detection() {
        assert!(is_table_row("| a | b |"));
        assert!(!is_table_row("not a table"));
        assert!(is_table_separator("|---|---|"));
        assert!(is_table_separator("| --- | :---: |"));
    }

    #[test]
    fn test_format_table() {
        let text = "| Name | Value |\n|------|-------|\n| foo | 42 |\n| bar | 99 |";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        // Should produce: top border, header, mid border, 2 data rows, bottom border
        assert!(lines.len() >= 6);
        assert!(lines[0].text().contains('┌'));
        assert!(lines[1].text().contains("Name"));
        assert!(lines[2].text().contains('├'));
    }

    #[test]
    fn test_format_heading() {
        let text = "# Hello World";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert!(!lines.is_empty());
        assert!(lines[0].text().contains("█ Hello World"));
    }

    #[test]
    fn test_format_code_block() {
        let text = "```\nlet x = 1;\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().contains("let x = 1;"));
    }

    #[test]
    fn test_format_code_block_wraps_long_lines() {
        let long = "x".repeat(50);
        let text = format!("```\n{long}\n```");
        let style = Style::default();
        let width = 20;
        let lines = format_chat_markdown(&text, width, style);
        assert!(lines.len() > 1, "long code lines should wrap");
        for line in &lines {
            assert!(
                UnicodeWidthStr::width(line.text().as_str()) <= width,
                "wrapped code line {:?} exceeds width {}",
                line.text(),
                width
            );
            assert!(
                line.text().starts_with("  "),
                "code lines keep the 2-space indent"
            );
        }
    }

    #[test]
    fn test_tool_marker_wraps_long_line() {
        let style = Style::default();
        let width = 30;
        let lines = format_chat_markdown(&format!("[Read {}]", "a".repeat(60)), width, style);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(UnicodeWidthStr::width(line.text().as_str()) <= width);
        }
    }

    #[test]
    fn test_all_output_lines_fit_width() {
        let text = "plain paragraph with enough words to require wrapping across the panel\n\n| Col A | Col B |\n|-------|-------|\n| alpha | beta |\n\n```\nfn main() { println!(\"hello\"); }\n```";
        let style = Style::default();
        let width = 24;
        let lines = format_chat_markdown(text, width, style);
        for line in &lines {
            assert!(
                UnicodeWidthStr::width(line.text().as_str()) <= width,
                "line {:?} exceeds width {}",
                line.text(),
                width
            );
        }
    }

    #[test]
    fn test_bold_text_preserves_style() {
        let text = "This is **important** info";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        // Plain-text concatenation strips the ** markers …
        assert_eq!(lines[0].text(), "This is important info");
        // … but the bold run keeps its own segment with both the BOLD
        // modifier and the bold foreground color so it is visually distinct
        // (BOLD alone renders too subtly in many terminals).
        let bold_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "important")
            .expect("expected a segment containing the bold text");
        assert!(
            bold_seg.style.add_modifier.contains(Modifier::BOLD),
            "bold segment must carry the BOLD modifier"
        );
        assert!(
            bold_seg.style.fg.is_some(),
            "bold segment must set a foreground color so the change is visible"
        );
    }

    #[test]
    fn test_italic_text_preserves_style() {
        let text = "An *italic* word";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        let italic_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "italic")
            .expect("expected italic segment");
        assert!(italic_seg.style.add_modifier.contains(Modifier::ITALIC));
        assert!(italic_seg.style.fg.is_some());
    }

    #[test]
    fn test_inline_code_preserves_style() {
        let text = "Run `cargo test` now";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        let code_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "cargo test")
            .expect("expected inline code segment");
        assert_eq!(code_seg.style.fg, Some(theme::CODE_GREEN));
    }

    #[test]
    fn test_bold_inside_list_item() {
        let text = "- a **bold** item";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().starts_with("• "));
        let bold_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "bold")
            .expect("expected bold segment inside list item");
        assert!(bold_seg.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_tabs_expanded_in_code_block() {
        let text = "```\n\tindented\n\t\tdouble\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text().contains("    indented"));
        assert!(lines[1].text().contains("        double"));
        // Ensure no tab characters remain
        for line in &lines {
            assert!(
                !line.text().contains('\t'),
                "tab character should be expanded"
            );
        }
    }

    #[test]
    fn test_wide_char_word_wrap() {
        // CJK chars are 2 display columns each. 5 chars = 10 columns.
        let text = "日本語テスト";
        let style = Style::default();
        // Width 6 columns: fits 3 CJK chars (6 cols), then wraps
        let lines = format_chat_markdown(text, 6, style);
        assert_eq!(lines.len(), 2);
        assert_eq!(UnicodeWidthStr::width(lines[0].text().as_str()), 6);
        assert_eq!(UnicodeWidthStr::width(lines[1].text().as_str()), 6);
    }

    #[test]
    fn test_wide_char_table_column_width() {
        let text = "| Name | 値 |\n|------|---|\n| foo | 日本 |";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        // The column for "日本" should be 4 display cols wide (not 2)
        let data_row = lines[3].text(); // top, header, separator, first data row
        // "日本" has display width 4, cell should be padded correctly
        assert!(
            data_row.contains("日本"),
            "data row should contain wide chars"
        );
    }

    #[test]
    fn test_table_multiline_cells_show_full_content() {
        let text = "| Feature | Description |\n|---------|-------------|\n| Auth | Supports JWT tokens and refresh rotation |\n| API | REST and GraphQL endpoints |";
        let style = Style::default();
        let width = 30;
        let lines = format_chat_markdown(text, width, style);

        let joined = lines.iter().map(|l| l.text()).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("JWT tokens"),
            "full cell text must appear, got:\n{joined}"
        );
        assert!(
            joined.contains("rotation"),
            "wrapped continuation must appear, got:\n{joined}"
        );
        assert!(
            !joined.contains('…'),
            "tables must not truncate cells with ellipsis"
        );

        for line in &lines {
            if is_table_rendered_line(&line.text()) {
                assert!(
                    UnicodeWidthStr::width(line.text().as_str()) <= width,
                    "table line {:?} exceeds width {}",
                    line.text(),
                    width
                );
            }
        }
    }

    #[test]
    fn test_table_row_height_matches_tallest_wrapped_cell() {
        let text = "| Col | Text |\n|-----|------|\n| a | word1 word2 word3 word4 |";
        let style = Style::default();
        let width = 18;
        let lines = format_chat_markdown(text, width, style);

        let body_rows: Vec<String> = lines
            .iter()
            .map(|l| l.text())
            .skip_while(|t| !t.starts_with('├'))
            .skip(1)
            .take_while(|t| !t.starts_with('└'))
            .collect();
        assert!(
            body_rows.len() >= 2,
            "tall cell should span multiple │ rows, got {body_rows:?}"
        );
        assert!(body_rows.iter().all(|t| t.starts_with('│')));
    }
}
