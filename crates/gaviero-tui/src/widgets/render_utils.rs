//! Shared rendering utilities for all panels.
//!
//! Provides common text rendering, word wrapping, and row fill operations
//! to eliminate duplicated character-by-character buffer writing loops.

use ratatui::buffer::Buffer;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

/// Write a string at `(x, y)` into the buffer, respecting `x_max` boundary.
/// Advances by each character's display width (handles wide/CJK chars).
/// Returns the x position after the last written character.
pub fn write_text(buf: &mut Buffer, x: u16, y: u16, x_max: u16, text: &str, style: Style) -> u16 {
    let mut cx = x;
    if y >= buf.area().bottom() {
        return cx;
    }
    for ch in text.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
        if cx + ch_w > x_max || cx >= buf.area().right() {
            break;
        }
        buf[(cx, y)].set_char(ch).set_style(style);
        cx += ch_w;
    }
    cx
}

/// Fill a row from `x` to `x + width` with spaces in the given style.
#[allow(dead_code)]
pub fn fill_row(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    if y >= buf.area().bottom() {
        return;
    }
    let x_max = (x + width).min(buf.area().right());
    for cx in x..x_max {
        buf[(cx, y)].set_char(' ').set_style(style);
    }
}

/// Word-wrap text into lines that fit within `width` display columns.
///
/// - Expands tabs to 4 spaces
/// - Respects Unicode display widths (CJK, emoji, etc.)
/// - Prefers breaking at spaces; hard-breaks if no space found
pub fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }

    let text = text.replace('\t', "    ");
    let mut lines = Vec::new();

    for physical_line in text.split('\n') {
        if physical_line.is_empty() {
            lines.push(String::new());
            continue;
        }

        let chars: Vec<char> = physical_line.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let mut display_w = 0usize;
            let mut end = start;
            while end < chars.len() {
                let cw = UnicodeWidthChar::width(chars[end]).unwrap_or(1);
                if display_w + cw > width {
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
                while bp > start && chars[bp] != ' ' {
                    bp -= 1;
                }
                if bp == start { end } else { bp + 1 }
            } else {
                end
            };

            let line: String = chars[start..break_at].iter().collect();
            lines.push(line.trim_end().to_string());
            start = break_at;
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn test_word_wrap_basic() {
        let lines = word_wrap("hello world foo", 11);
        assert_eq!(lines, vec!["hello world", "foo"]);
    }

    #[test]
    fn test_word_wrap_empty() {
        let lines = word_wrap("", 10);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn test_word_wrap_newlines() {
        let lines = word_wrap("a\nb\nc", 10);
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_word_wrap_tabs_expanded() {
        let lines = word_wrap("\tindented", 20);
        assert!(lines[0].starts_with("    "));
    }

    #[test]
    fn test_word_wrap_wide_chars() {
        let lines = word_wrap("日本語テスト", 6);
        assert_eq!(lines.len(), 2);
        assert_eq!(UnicodeWidthStr::width(lines[0].as_str()), 6);
    }
}
