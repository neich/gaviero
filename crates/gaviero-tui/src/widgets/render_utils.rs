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

/// Strip ANSI escape sequences from text so it is safe to write into ratatui cells.
///
/// Raw escape codes written char-by-char corrupt the terminal display — they bypass
/// ratatui's style model and change terminal attributes mid-render. Call this on any
/// text that originated from an external process before storing it for display.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\x07' || c == '\x1b' {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
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

    #[test]
    fn test_strip_ansi_csi() {
        assert_eq!(strip_ansi("\x1b[32mgreen\x1b[0m"), "green");
    }

    #[test]
    fn test_strip_ansi_plain() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn test_strip_ansi_osc() {
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
    }

    #[test]
    fn test_strip_ansi_bold_reset() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m normal"), "bold normal");
    }
}
