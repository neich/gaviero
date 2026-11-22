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
    wrap_rows(text, width).into_iter().map(|row| row.text).collect()
}

/// One row of [`wrap_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedRow {
    pub text: String,
    /// `Some(sep)` when the row continues the row above (a soft wrap). `sep`
    /// is the whitespace the break dropped, and is empty for a hard break
    /// inside a word. `None` when the row starts a line of the source text.
    pub join: Option<String>,
}

/// [`word_wrap`], also reporting which rows are soft wraps, so copying the
/// rows can rebuild the source line instead of inserting a newline at every
/// visual break.
pub fn wrap_rows(text: &str, width: usize) -> Vec<WrappedRow> {
    if width == 0 {
        return vec![WrappedRow {
            text: text.to_string(),
            join: None,
        }];
    }

    let text = text.replace('\t', "    ");
    let mut lines = Vec::new();

    for physical_line in text.split('\n') {
        if physical_line.is_empty() {
            lines.push(WrappedRow {
                text: String::new(),
                join: None,
            });
            continue;
        }

        let chars: Vec<char> = physical_line.chars().collect();
        let mut start = 0;
        let mut join = None;
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

            let piece: String = chars[start..break_at].iter().collect();
            let kept = piece.trim_end();
            let dropped = piece[kept.len()..].to_string();
            lines.push(WrappedRow {
                text: kept.to_string(),
                join: join.take(),
            });
            join = Some(dropped);
            start = break_at;
        }
    }

    lines
}

/// CSI final bytes per ECMA-48 / xterm: `0x40`…`0x7E` (`@`…`~`).
///
/// Letters (`m`, `H`, …) are the common case, but many sequences terminate on
/// non-letters — notably `~` (`ESC[3~` Delete, `ESC[200~` / `ESC[201~`
/// bracketed-paste). Stopping only on `is_ascii_alphabetic()` eats the `~`
/// and every following character until a letter, which truncates paths like
/// `~/…` and any text after a CSI-`~` sequence.
#[inline]
fn is_csi_final_byte(c: char) -> bool {
    matches!(c, '\x40'..='\x7E')
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
                        if is_csi_final_byte(c) {
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

    /// Joining every soft-wrapped row with its `sep` gives the source line
    /// back, whether the break fell on a space or inside a long word.
    #[test]
    fn wrap_rows_soft_wraps_rebuild_the_source_line() {
        let source = "cargo test -p gaviero-core --features api-embedders C:\\very\\long\\path\\to\\file.rs";
        let rows = wrap_rows(&format!("{source}\nnext line"), 12);
        assert!(rows.len() > 3, "{rows:?}");
        let mut rebuilt = String::new();
        for (i, row) in rows.iter().enumerate() {
            match &row.join {
                Some(sep) => rebuilt.push_str(sep),
                None if i > 0 => rebuilt.push('\n'),
                None => {}
            }
            rebuilt.push_str(&row.text);
        }
        assert_eq!(rebuilt, format!("{source}\nnext line"));
        assert_eq!(rows.last().unwrap().join, None, "a source newline is not a soft wrap");
        assert_eq!(
            word_wrap(source, 12),
            rows.iter().take_while(|r| r.text != "next line").map(|r| r.text.clone()).collect::<Vec<_>>()
        );
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

    #[test]
    fn strip_ansi_preserves_bare_tilde() {
        assert_eq!(strip_ansi("hello~world"), "hello~world");
        assert_eq!(strip_ansi("path=~/foo/bar"), "path=~/foo/bar");
        assert_eq!(strip_ansi("\x1b[0m~/home"), "~/home");
    }

    #[test]
    fn strip_ansi_csi_tilde_final_does_not_eat_following_text() {
        // Delete / bracketed-paste finals use `~` (0x7E), not a letter.
        assert_eq!(strip_ansi("before\x1b[3~after"), "beforeafter");
        assert_eq!(strip_ansi("\x1b[200~PASTE\x1b[201~end"), "PASTEend");
        assert_eq!(strip_ansi("x\x1b[1;2~y"), "xy");
    }
}
