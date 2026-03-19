//! Markdown formatting for the chat panel.
//!
//! Converts markdown text into styled lines suitable for the chat panel's
//! single-style-per-line rendering pipeline. Handles tables, headings,
//! code blocks, lists, horizontal rules, and strips inline markers.

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::editor::markdown::parse_inline;
use crate::theme;

/// A single rendered line for the chat panel.
pub struct ChatLine {
    pub style: Style,
    pub text: String,
}

/// Format markdown content into styled lines for the chat panel.
///
/// Processes tables with box-drawing characters, renders headings with visual
/// markers, indents code blocks, formats lists, and strips inline `**`/`*`/`` ` ``
/// markers from regular text.
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

    while i < src_lines.len() {
        let line = src_lines[i];
        let trimmed = line.trim();

        // Code block fences
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            i += 1;
            continue;
        }

        if in_code_block {
            output.push(ChatLine {
                style: code_style,
                text: format!("  {}", line),
            });
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
            output.push(ChatLine {
                style: base_style,
                text: String::new(),
            });
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
            output.push(ChatLine {
                style: rule_style,
                text: rule,
            });
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
                for wl in crate::widgets::render_utils::word_wrap(&format!("{}{}", marker, content), width) {
                    output.push(ChatLine {
                        style: heading_style,
                        text: wl,
                    });
                }
                i += 1;
                continue;
            }
        }

        // Block quotes
        if trimmed.starts_with('>') {
            let content = strip_inline_markers(trimmed[1..].trim_start());
            for wl in crate::widgets::render_utils::word_wrap(&format!("│ {}", content), width) {
                output.push(ChatLine {
                    style: quote_style,
                    text: wl,
                });
            }
            i += 1;
            continue;
        }

        // Unordered list items
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let indent = (line.len() - line.trim_start().len()) / 2;
            let pad: String = "  ".repeat(indent);
            let content = strip_inline_markers(&trimmed[2..]);
            let first = format!("{}• {}", pad, content);
            for (j, wl) in crate::widgets::render_utils::word_wrap(&first, width).into_iter().enumerate() {
                if j == 0 {
                    output.push(ChatLine {
                        style: base_style,
                        text: wl,
                    });
                } else {
                    output.push(ChatLine {
                        style: base_style,
                        text: format!("{}  {}", pad, wl.trim_start()),
                    });
                }
            }
            i += 1;
            continue;
        }

        // Ordered list items
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                let content = strip_inline_markers(&trimmed[dot_pos + 2..]);
                let marker = format!("{}. ", prefix);
                let first = format!("{}{}", marker, content);
                for wl in crate::widgets::render_utils::word_wrap(&first, width) {
                    output.push(ChatLine {
                        style: base_style,
                        text: wl,
                    });
                }
                i += 1;
                continue;
            }
        }

        // File proposal markers: [wrote ...] and [writing ...]
        // Style with dim cyan to visually distinguish from regular text
        if (trimmed.starts_with("[wrote ") || trimmed.starts_with("[writing "))
            && trimmed.ends_with(']')
        {
            let file_marker_style = Style::default()
                .fg(theme::INFO_CYAN)
                .add_modifier(Modifier::DIM);
            output.push(ChatLine {
                style: file_marker_style,
                text: trimmed.to_string(),
            });
            i += 1;
            continue;
        }

        // Regular text: strip inline markers and word-wrap
        let cleaned = strip_inline_markers(trimmed);
        for wl in crate::widgets::render_utils::word_wrap(&cleaned, width) {
            output.push(ChatLine {
                style: base_style,
                text: wl,
            });
        }
        i += 1;
    }

    output
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
    if total_content > content_budget && content_budget > 0 {
        // Proportionally shrink columns
        for w in &mut col_widths {
            *w = (*w * content_budget / total_content).max(1);
        }
    }

    // Build box-drawing lines
    let top = build_table_border(&col_widths, '┌', '┬', '┐', '─');
    let mid = build_table_border(&col_widths, '├', '┼', '┤', '─');
    let bot = build_table_border(&col_widths, '└', '┴', '┘', '─');

    output.push(ChatLine {
        style: border_style,
        text: top,
    });

    for (row_idx, row) in rows.iter().enumerate() {
        let style = if row_idx == 0 {
            header_style
        } else {
            base_style
        };

        let mut line = String::new();
        for (j, w) in col_widths.iter().enumerate() {
            line.push('│');
            line.push(' ');
            let cell = row.get(j).map(|s| s.as_str()).unwrap_or("");
            let cell_display_w = UnicodeWidthStr::width(cell);
            if cell_display_w <= *w {
                line.push_str(cell);
                for _ in 0..(*w - cell_display_w) {
                    line.push(' ');
                }
            } else {
                // Truncate by display width
                let mut tw = 0;
                let mut truncated = String::new();
                for ch in cell.chars() {
                    let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
                    if tw + cw >= *w {
                        break;
                    }
                    truncated.push(ch);
                    tw += cw;
                }
                line.push_str(&truncated);
                line.push('…');
                // Pad remaining space if truncation left a gap
                for _ in 0..(*w - tw - 1) {
                    line.push(' ');
                }
            }
            line.push(' ');
        }
        line.push('│');

        output.push(ChatLine { style, text: line });

        if row_idx == 0 && rows.len() > 1 {
            output.push(ChatLine {
                style: border_style,
                text: mid.clone(),
            });
        }
    }

    output.push(ChatLine {
        style: border_style,
        text: bot,
    });
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
        assert!(lines[0].text.contains('┌'));
        assert!(lines[1].text.contains("Name"));
        assert!(lines[2].text.contains('├'));
    }

    #[test]
    fn test_format_heading() {
        let text = "# Hello World";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert!(!lines.is_empty());
        assert!(lines[0].text.contains("█ Hello World"));
    }

    #[test]
    fn test_format_code_block() {
        let text = "```\nlet x = 1;\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("let x = 1;"));
    }

    #[test]
    fn test_strips_bold_in_text() {
        let text = "This is **important** info";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "This is important info");
    }

    #[test]
    fn test_tabs_expanded_in_code_block() {
        let text = "```\n\tindented\n\t\tdouble\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.contains("    indented"));
        assert!(lines[1].text.contains("        double"));
        // Ensure no tab characters remain
        for line in &lines {
            assert!(!line.text.contains('\t'), "tab character should be expanded");
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
        assert_eq!(UnicodeWidthStr::width(lines[0].text.as_str()), 6);
        assert_eq!(UnicodeWidthStr::width(lines[1].text.as_str()), 6);
    }

    #[test]
    fn test_wide_char_table_column_width() {
        let text = "| Name | 値 |\n|------|---|\n| foo | 日本 |";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        // The column for "日本" should be 4 display cols wide (not 2)
        let data_row = &lines[3].text; // top, header, separator, first data row
        // "日本" has display width 4, cell should be padded correctly
        assert!(data_row.contains("日本"), "data row should contain wide chars");
    }
}
