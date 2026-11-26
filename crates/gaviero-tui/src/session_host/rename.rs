//! Tab-rename popup (Ctrl+Alt+R): a compact single-line box pre-filled with
//! the active tab's current label (selected so typing replaces it at once).
//! Confirming with Enter locks the label on the tab so nothing — `cd` in a
//! shell, an SSH folder report, a restart, or quitting and reopening — ever
//! changes it back.

use crate::widgets::text_input::TextInput;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use unicode_width::UnicodeWidthChar;

const EMPTY_ERROR: &str = "Type a name";
const HINT: &str = "Enter save · Esc cancel";
const MAX_WIDTH: u16 = 52;
const HEIGHT: u16 = 4;

/// A key the rename popup understands, from either input path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenameKey {
    Char(char),
    Backspace,
    Delete,
    DeleteWordBack,
    Clear,
    Left,
    Right,
    WordLeft,
    WordRight,
    SelectLeft,
    SelectRight,
    Home,
    End,
    SelectAll,
    Paste,
    Enter,
    Esc,
}

/// What the host does after a rename-popup key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RenameOutcome {
    Stay,
    Close,
    Apply(String),
}

pub(crate) struct RenameDialog {
    /// The session this popup is renaming, so closing another tab (or this
    /// one) while the popup is open can never apply the name to the wrong tab.
    pub tab_id: u64,
    pub input: TextInput,
    pub error: Option<String>,
}

impl RenameDialog {
    pub fn new(tab_id: u64, current_label: &str) -> Self {
        let mut input = TextInput::with_text(current_label.to_string());
        input.select_all();
        Self {
            tab_id,
            input,
            error: None,
        }
    }

    /// Insert pasted text as a single line: line breaks and tabs become
    /// spaces, and other control characters are dropped.
    pub fn paste(&mut self, text: &str) {
        let clean = sanitize_paste(text);
        if !clean.is_empty() {
            self.input.insert_str(&clean);
            self.error = None;
        }
    }

    /// Apply one key. `Apply` carries the trimmed non-empty tab name.
    pub fn on_key(&mut self, key: RenameKey) -> RenameOutcome {
        match key {
            RenameKey::Esc => return RenameOutcome::Close,
            RenameKey::Enter => {
                let name = self.input.text.trim();
                if name.is_empty() {
                    self.error = Some(EMPTY_ERROR.to_string());
                    return RenameOutcome::Stay;
                }
                return RenameOutcome::Apply(name.to_string());
            }
            RenameKey::Char(c) => {
                self.input.insert_char(c);
                self.error = None;
            }
            RenameKey::Backspace => {
                self.input.backspace();
                self.error = None;
            }
            RenameKey::Delete => {
                self.input.delete();
                self.error = None;
            }
            RenameKey::DeleteWordBack => {
                if self.input.has_selection() {
                    self.input.delete_selection();
                } else {
                    self.input.delete_word_back();
                }
                self.error = None;
            }
            RenameKey::Clear => {
                self.input.clear();
                self.error = None;
            }
            RenameKey::Left => self.input.move_left(),
            RenameKey::Right => self.input.move_right(),
            RenameKey::WordLeft => {
                self.input.clear_selection();
                self.input.move_word_left();
            }
            RenameKey::WordRight => {
                self.input.clear_selection();
                self.input.move_word_right();
            }
            RenameKey::SelectLeft => self.input.select_left(),
            RenameKey::SelectRight => self.input.select_right(),
            RenameKey::Home => self.input.move_home(),
            RenameKey::End => self.input.move_end(),
            RenameKey::SelectAll => self.input.select_all(),
            RenameKey::Paste => {
                if let Some(text) = super::clipboard::get_text() {
                    self.paste(&text);
                }
            }
        }
        RenameOutcome::Stay
    }

    /// A small centred box over `area`: prompt line and hint / error line.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let width = area.width.saturating_sub(4).min(MAX_WIDTH);
        let height = HEIGHT.min(area.height);
        if width < 12 || height < HEIGHT {
            return;
        }
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        Clear.render(rect, buf);
        let base = Style::default().fg(Color::White).bg(Color::Rgb(35, 39, 46));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Rename tab ")
            .border_style(Style::default().fg(Color::Rgb(229, 192, 123)))
            .style(base);
        let inner = block.inner(rect);
        block.render(rect, buf);
        if inner.height < 2 || inner.width < 5 {
            return;
        }

        buf.set_stringn(
            inner.x + 1,
            inner.y,
            "> ",
            inner.width.saturating_sub(1) as usize,
            Style::default().fg(Color::Rgb(229, 192, 123)).bg(Color::Rgb(35, 39, 46)),
        );

        let field_x = inner.x + 3;
        let field_w = inner.width.saturating_sub(4) as usize;
        if field_w > 0 {
            self.render_input(field_x, inner.y, field_w, base, buf);
        }

        let (note, style) = match &self.error {
            Some(err) => (err.as_str(), Style::default().fg(Color::Rgb(224, 108, 117))),
            None => (HINT, Style::default().fg(Color::Rgb(157, 165, 180))),
        };
        buf.set_stringn(
            inner.x + 1,
            inner.y + 1,
            note,
            inner.width.saturating_sub(2) as usize,
            style,
        );
    }

    fn render_input(
        &self,
        field_x: u16,
        y: u16,
        field_w: usize,
        base: Style,
        buf: &mut Buffer,
    ) {
        let chars: Vec<char> = self.input.text.chars().collect();
        let widths: Vec<usize> = chars.iter().map(|c| c.width().unwrap_or(0)).collect();
        let cursor_w: usize = widths[..self.input.cursor].iter().sum();
        let min_scroll = cursor_w.saturating_sub(field_w.saturating_sub(1));

        let mut start = 0;
        let mut skipped = 0;
        while start < chars.len() && skipped < min_scroll {
            skipped += widths[start];
            start += 1;
        }

        let sel = self.input.selection_range();
        let sel_style = Style::default()
            .fg(Color::Rgb(30, 34, 42))
            .bg(Color::Rgb(97, 175, 239));

        let mut col = 0usize;
        let mut tmp = [0u8; 4];
        for idx in start..chars.len() {
            let cw = widths[idx];
            if col + cw > field_w {
                break;
            }
            let style = if sel.is_some_and(|(a, b)| idx >= a && idx < b) {
                sel_style
            } else {
                base
            };
            if cw > 0 {
                let s = chars[idx].encode_utf8(&mut tmp);
                buf.set_stringn(field_x + col as u16, y, s, cw, style);
                col += cw;
            }
        }

        if sel.is_none() {
            let caret_off: usize = if self.input.cursor >= start {
                widths[start..self.input.cursor].iter().sum()
            } else {
                0
            };
            let caret_x = field_x + caret_off.min(field_w.saturating_sub(1)) as u16;
            buf[(caret_x, y)].set_style(base.add_modifier(Modifier::REVERSED));
        }
    }
}

fn sanitize_paste(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\r' | '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_with_current_label_selected_so_typing_replaces_it() {
        let mut d = RenameDialog::new(7, "alpha");
        assert_eq!(d.tab_id, 7);
        assert_eq!(d.input.text, "alpha");
        assert!(d.input.has_selection());
        assert_eq!(d.on_key(RenameKey::Char('b')), RenameOutcome::Stay);
        assert_eq!(d.on_key(RenameKey::Char('o')), RenameOutcome::Stay);
        assert_eq!(d.on_key(RenameKey::Char('x')), RenameOutcome::Stay);
        assert_eq!(d.on_key(RenameKey::Enter), RenameOutcome::Apply("box".into()));
    }

    #[test]
    fn moving_clears_selection_to_edit_in_place() {
        let mut d = RenameDialog::new(1, "alpha");
        d.on_key(RenameKey::Right);
        assert!(!d.input.has_selection());
        d.on_key(RenameKey::Char('-'));
        d.on_key(RenameKey::Char('2'));
        assert_eq!(d.on_key(RenameKey::Enter), RenameOutcome::Apply("alpha-2".into()));
    }

    #[test]
    fn empty_or_blank_name_is_refused_and_clears_on_edit() {
        let mut d = RenameDialog::new(1, "alpha");
        d.on_key(RenameKey::Backspace);
        assert_eq!(d.input.text, "");
        assert_eq!(d.on_key(RenameKey::Enter), RenameOutcome::Stay);
        assert_eq!(d.error.as_deref(), Some(EMPTY_ERROR));
        d.on_key(RenameKey::Char(' '));
        assert_eq!(d.error, None);
        assert_eq!(d.on_key(RenameKey::Enter), RenameOutcome::Stay);
        assert_eq!(d.error.as_deref(), Some(EMPTY_ERROR));
        d.on_key(RenameKey::Char('x'));
        assert_eq!(d.on_key(RenameKey::Enter), RenameOutcome::Apply("x".into()));
    }

    #[test]
    fn esc_cancels_and_pasting_strips_line_breaks() {
        let mut d = RenameDialog::new(1, "old");
        d.paste("my\nnew\tname\x07");
        assert_eq!(d.input.text, "my new name");
        assert_eq!(d.on_key(RenameKey::Esc), RenameOutcome::Close);
    }

    #[test]
    fn word_and_clear_editing_shortcuts() {
        let mut d = RenameDialog::new(1, "foo bar baz");
        d.on_key(RenameKey::End);
        d.on_key(RenameKey::DeleteWordBack);
        assert_eq!(d.input.text, "foo bar ");
        d.on_key(RenameKey::Clear);
        assert_eq!(d.input.text, "");
        d.on_key(RenameKey::Char('a'));
        d.on_key(RenameKey::SelectAll);
        d.on_key(RenameKey::DeleteWordBack);
        assert_eq!(d.input.text, "");
    }

    #[test]
    fn render_draws_box_title_text_and_handles_long_or_wide_names() {
        let mut d = RenameDialog::new(1, "日本語-very-long-tab-name-exceeding-the-popup-box-width");
        d.on_key(RenameKey::End);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let screen: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains(" Rename tab "), "{screen}");
        assert!(screen.contains(HINT), "{screen}");
        assert!(screen.contains("box-width"), "scrolled to end:\n{screen}");
    }
}
