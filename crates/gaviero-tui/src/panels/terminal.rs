//! Terminal panel — rendering functions and input encoding.
//!
//! PTY lifecycle is managed by `gaviero_core::terminal::TerminalManager`.
//! This module provides only rendering helpers and key mapping.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer as RataBuf;
use ratatui::layout::Rect;

use crate::theme;

/// State for mouse-based text selection in the terminal panel.
#[derive(Debug, Default, Clone)]
pub struct TerminalSelectionState {
    /// Selection anchor in vt100 screen coordinates (row, col), set on MouseDown.
    pub anchor: Option<(u16, u16)>,
    /// Selection end in vt100 screen coordinates (row, col), updated on Drag.
    pub end: Option<(u16, u16)>,
    /// Whether the mouse is currently dragging to select.
    pub dragging: bool,
}

impl TerminalSelectionState {
    /// Begin a selection at (row, col) in vt100 screen coordinates.
    pub fn start(&mut self, row: u16, col: u16) {
        self.anchor = Some((row, col));
        self.end = Some((row, col));
        self.dragging = true;
    }

    /// Extend the selection to (row, col).
    pub fn extend(&mut self, row: u16, col: u16) {
        self.end = Some((row, col));
    }

    /// Clear the selection entirely.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.end = None;
        self.dragging = false;
    }

    /// Ordered selection range: (start_row, start_col, end_row, end_col).
    fn ordered_range(&self) -> Option<(u16, u16, u16, u16)> {
        let (ar, ac) = self.anchor?;
        let (er, ec) = self.end?;
        if ar < er || (ar == er && ac <= ec) {
            Some((ar, ac, er, ec))
        } else {
            Some((er, ec, ar, ac))
        }
    }

    /// Check if a cell at (row, col) is within the selection.
    pub fn is_selected(&self, row: u16, col: u16) -> bool {
        let Some((sr, sc, er, ec)) = self.ordered_range() else { return false };
        if sr == er && sc == ec { return false; }
        if row < sr || row > er { return false; }
        if row == sr && row == er { return col >= sc && col < ec; }
        if row == sr { return col >= sc; }
        if row == er { return col < ec; }
        true
    }

    /// Extract selected text from a vt100 screen. Each row is trimmed of
    /// trailing whitespace; rows are joined with newlines.
    pub fn extract_text(&self, screen: &vt100::Screen) -> Option<String> {
        let (sr, sc, er, ec) = self.ordered_range()?;
        if sr == er && sc == ec { return None; }

        let mut result = String::new();
        let screen_cols = screen.size().1;
        for row in sr..=er {
            let col_start = if row == sr { sc } else { 0 };
            let col_end = if row == er { ec } else { screen_cols };
            let mut line = String::new();
            for col in col_start..col_end {
                if let Some(cell) = screen.cell(row, col) {
                    let contents = cell.contents();
                    if contents.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(contents);
                    }
                } else {
                    line.push(' ');
                }
            }
            if row > sr {
                result.push('\n');
            }
            result.push_str(line.trim_end());
        }

        if result.is_empty() { None } else { Some(result) }
    }
}

/// Render terminal content (no border) from a vt100 screen into a ratatui buffer.
pub fn render_terminal_screen(
    screen: &vt100::Screen,
    area: Rect,
    buf: &mut RataBuf,
    focused: bool,
    selection: &TerminalSelectionState,
) {
    let sel_style = ratatui::style::Style::default()
        .fg(theme::TAB_BG)
        .bg(theme::FOCUS_BORDER);

    for row in 0..area.height {
        for col in 0..area.width {
            let cx = area.x + col;
            let cy = area.y + row;
            if cx >= buf.area().right() || cy >= buf.area().bottom() {
                continue;
            }
            let cell = screen.cell(row, col);
            let ch = if let Some(cell) = cell {
                cell.contents().chars().next().unwrap_or(' ')
            } else {
                ' '
            };
            let style = if selection.is_selected(row, col) {
                sel_style
            } else if let Some(cell) = cell {
                vt100_style_to_ratatui(cell)
            } else {
                ratatui::style::Style::default()
            };
            buf[(cx, cy)].set_char(ch).set_style(style);
        }
    }

    if focused {
        render_cursor(screen, area, buf, 0);
    }
}

/// Render terminal with a border/title line at the top.
pub fn render_terminal_with_border(
    screen: &vt100::Screen,
    area: Rect,
    buf: &mut RataBuf,
    focused: bool,
    selection: &TerminalSelectionState,
) {
    // Border line at top
    let scrollback = screen.scrollback();
    let border_fg = if focused {
        theme::FOCUS_BORDER
    } else {
        theme::BORDER_DIM
    };
    let border_style = ratatui::style::Style::default().fg(border_fg);
    if area.height > 0 {
        let title = if scrollback > 0 {
            format!(" Terminal [scroll: -{}] ", scrollback)
        } else {
            " Terminal ".to_string()
        };
        let title = &title;
        for col in 0..area.width {
            let cx = area.x + col;
            let ch = if col == 0 {
                '─'
            } else if col as usize == 1 {
                ' '
            } else if (col as usize) < title.len() + 2 {
                title.as_bytes()[col as usize - 2] as char
            } else {
                '─'
            };
            if cx < buf.area().right() && area.y < buf.area().bottom() {
                buf[(cx, area.y)].set_char(ch).set_style(border_style);
            }
        }
    }

    // Terminal content starts below border
    let content_y = area.y + 1;
    let content_height = area.height.saturating_sub(1);

    let sel_style = ratatui::style::Style::default()
        .fg(theme::TAB_BG)
        .bg(theme::FOCUS_BORDER);

    for row in 0..content_height {
        for col in 0..area.width {
            let cx = area.x + col;
            let cy = content_y + row;
            if cx >= buf.area().right() || cy >= buf.area().bottom() {
                continue;
            }

            let cell = screen.cell(row, col);
            let ch = if let Some(cell) = cell {
                cell.contents().chars().next().unwrap_or(' ')
            } else {
                ' '
            };
            let style = if selection.is_selected(row, col) {
                sel_style
            } else if let Some(cell) = cell {
                vt100_style_to_ratatui(cell)
            } else {
                ratatui::style::Style::default()
            };
            buf[(cx, cy)].set_char(ch).set_style(style);
        }
    }

    if focused {
        render_cursor(screen, area, buf, 1);
    }
}

/// Render the cursor block at the correct position.
fn render_cursor(screen: &vt100::Screen, area: Rect, buf: &mut RataBuf, y_offset: u16) {
    let (cursor_row, cursor_col) = screen.cursor_position();
    let cx = area.x + cursor_col;
    let cy = area.y + y_offset + cursor_row;
    if cx < buf.area().right() && cy < buf.area().bottom() {
        let cursor_style = ratatui::style::Style::default()
            .fg(ratatui::style::Color::Black)
            .bg(theme::TEXT_FG);
        buf[(cx, cy)].set_style(cursor_style);
    }
}

/// Convert a crossterm KeyEvent to the byte sequence expected by the terminal.
pub fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) if ctrl => {
            let byte = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
            vec![byte]
        }
        KeyCode::Char(c) if alt => {
            let mut bytes = vec![0x1b];
            let mut char_buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut char_buf).as_bytes());
            bytes
        }
        KeyCode::Char(c) => {
            let mut char_buf = [0u8; 4];
            c.encode_utf8(&mut char_buf).as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::F(n) => f_key_bytes(n),
        _ => vec![],
    }
}

fn f_key_bytes(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => vec![],
    }
}

/// Returns true if this key should escape the terminal and go to the app keymap.
pub fn is_terminal_escape_key(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    matches!(
        (key.code, ctrl, alt, shift),
        (KeyCode::F(4), false, false, false)       // F4 — toggle terminal
        | (KeyCode::Char('\\'), true, false, false) // Ctrl+\ — cycle focus
        | (KeyCode::Char('q'), true, false, false)  // Ctrl+q — quit
        | (KeyCode::Char('w'), true, false, false)  // Ctrl+W — close terminal tab
        | (KeyCode::F(8), false, false, false)       // F8 — new terminal tab
        | (KeyCode::Char('t'), false, true, false)    // Alt+T — new terminal tab
        // Ctrl+arrows — inter-panel navigation (always escapes terminal)
        | (KeyCode::Up, true, false, false)          // Ctrl+Up — focus editor
        | (KeyCode::Down, true, false, false)        // Ctrl+Down — focus terminal
        | (KeyCode::Left, true, false, false)        // Ctrl+Left — focus file tree
        | (KeyCode::Right, true, false, false)       // Ctrl+Right — focus side panel
        // Shift+arrows — intra-panel navigation
        | (KeyCode::Left, false, false, true)        // Shift+Left — prev terminal tab
        | (KeyCode::Right, false, false, true)       // Shift+Right — next terminal tab
        | (KeyCode::Up, false, false, true)          // Shift+Up — scroll back
        | (KeyCode::Down, false, false, true)        // Shift+Down — scroll forward
        // Shift+PageUp/PageDown — page scroll in terminal
        | (KeyCode::PageUp, false, false, true)      // Shift+PageUp — page scroll back
        | (KeyCode::PageDown, false, false, true)    // Shift+PageDown — page scroll forward
        // Alt+Up/Down — terminal resize
        | (KeyCode::Up, false, true, false)          // Alt+Up — grow terminal
        | (KeyCode::Down, false, true, false)        // Alt+Down — shrink terminal
    )
}

/// Convert vt100 cell attributes to ratatui style.
fn vt100_style_to_ratatui(cell: &vt100::Cell) -> ratatui::style::Style {
    use ratatui::style::{Color, Modifier, Style};

    let fg = match cell.fgcolor() {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    };
    let bg = match cell.bgcolor() {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    };

    let mut modifier = Modifier::empty();
    if cell.bold() { modifier |= Modifier::BOLD; }
    if cell.italic() { modifier |= Modifier::ITALIC; }
    if cell.underline() { modifier |= Modifier::UNDERLINED; }
    if cell.inverse() { modifier |= Modifier::REVERSED; }

    Style::default().fg(fg).bg(bg).add_modifier(modifier)
}
