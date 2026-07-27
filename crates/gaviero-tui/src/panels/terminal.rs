//! Terminal panel — rendering functions and input encoding.
//!
//! PTY lifecycle is managed by `gaviero_core::terminal::TerminalManager`.
//! This module provides only rendering helpers and key mapping.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer as RataBuf;
use ratatui::layout::Rect;

use crate::keymap::{Action, Keymap};
use crate::theme;

/// State for mouse-based text selection in the terminal panel.
#[derive(Debug, Default, Clone)]
pub struct TerminalSelectionState {
    /// Selection anchor in vt100 screen coordinates (row, col), set on MouseDown.
    pub anchor: Option<(u16, u16)>,
    /// Selection end in vt100 screen coordinates (row, col), updated on Drag.
    pub end: Option<(u16, u16)>,
    /// Scrollback position when anchor was set (for tracking across scrolls).
    pub anchor_scrollback: Option<usize>,
    /// Scrollback position when end was last updated (for tracking across scrolls).
    pub end_scrollback: Option<usize>,
    /// Whether the mouse is currently dragging to select.
    pub dragging: bool,
    /// Keyboard cursor (row, col) for Shift+Arrow selection.
    pub kb_cursor: Option<(u16, u16)>,
}

impl TerminalSelectionState {
    /// Begin a selection at (row, col) in vt100 screen coordinates.
    /// Takes screen reference to record scrollback position for accurate extraction during scrolling.
    pub fn start(&mut self, row: u16, col: u16, screen: &vt100::Screen) {
        let scrollback = screen.scrollback();
        self.anchor = Some((row, col));
        self.anchor_scrollback = Some(scrollback);
        self.end = Some((row, col));
        self.end_scrollback = Some(scrollback);
        self.dragging = true;
    }

    /// Extend the selection to (row, col).
    /// Takes screen reference to record scrollback position for accurate extraction during scrolling.
    pub fn extend(&mut self, row: u16, col: u16, screen: &vt100::Screen) {
        let scrollback = screen.scrollback();
        self.end = Some((row, col));
        self.end_scrollback = Some(scrollback);
    }

    /// Clear the selection entirely.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.end = None;
        self.anchor_scrollback = None;
        self.end_scrollback = None;
        self.dragging = false;
        self.kb_cursor = None;
    }

    /// Check if there is an active text selection.
    pub fn has_selection(&self) -> bool {
        matches!((self.anchor, self.end), (Some(a), Some(e)) if a != e)
    }

    /// Check if a cell at (row, col) is within the selection.
    /// Accounts for scrollback changes to correctly highlight selections that span scrolled content.
    pub fn is_selected(&self, row: u16, col: u16, screen: &vt100::Screen) -> bool {
        let (ar, ac) = match self.anchor {
            Some(a) => a,
            None => return false,
        };
        let (er, ec) = match self.end {
            Some(e) => e,
            None => return false,
        };

        if ar == er && ac == ec {
            return false;
        }

        let anchor_sb = self.anchor_scrollback.unwrap_or(0) as i32;
        let end_sb = self.end_scrollback.unwrap_or(0) as i32;
        let current_sb = screen.scrollback() as i32;

        // Adjust each endpoint's screen row for scrollback change since it was recorded.
        // When scrollback increases (scrolling back), older content appears at top,
        // pushing existing content DOWN: new_row = old_row + (current_sb - original_sb).
        let ar_current = (ar as i32 + (current_sb - anchor_sb)).max(0);
        let er_current = (er as i32 + (current_sb - end_sb)).max(0);

        // Normalize to (start_row, start_col, end_row, end_col) order.
        let (sr, sc, er_norm, ec) =
            if ar_current < er_current || (ar_current == er_current && ac <= ec) {
                (ar_current, ac, er_current, ec)
            } else {
                (er_current, ec, ar_current, ac)
            };

        let row_i = row as i32;
        if row_i < sr || row_i > er_norm {
            return false;
        }
        if row_i == sr && row_i == er_norm {
            return col >= sc && col < ec;
        }
        if row_i == sr {
            return col >= sc;
        }
        if row_i == er_norm {
            return col < ec;
        }
        true
    }

    /// Extract selected text from a vt100 screen. Each row is trimmed of
    /// trailing whitespace; rows are joined with newlines.
    /// Temporarily adjusts scrollback to access rows that are currently off-screen
    /// due to scrolling during selection. Restores original scrollback before returning.
    pub fn extract_text(&self, screen: &mut vt100::Screen) -> Option<String> {
        let (ar, ac) = self.anchor?;
        let (er, ec) = self.end?;
        let anchor_sb = self.anchor_scrollback.unwrap_or(0) as i32;
        let end_sb = self.end_scrollback.unwrap_or(0) as i32;

        if ar == er && ac == ec && anchor_sb == end_sb {
            return None;
        }

        let original_sb = screen.scrollback();
        let screen_rows = screen.size().0 as i32;
        let screen_cols = screen.size().1;

        // Absolute position: lines back from present bottom. Higher value = older content.
        // abs(row, sb) = sb + (screen_rows - 1 - row)
        //   At sb=0, row=screen_rows-1 (bottom): abs=0 (the "present")
        //   At sb=0, row=0 (top): abs=screen_rows-1
        //   At sb=S, row=R: abs = S + (screen_rows - 1 - R)
        let anchor_abs = anchor_sb + (screen_rows - 1 - ar as i32);
        let end_abs = end_sb + (screen_rows - 1 - er as i32);

        // Determine selection order: higher abs = top of selection (older content).
        let (start_abs, start_col, end_abs_pos, end_col) =
            if anchor_abs > end_abs || (anchor_abs == end_abs && ac <= ec) {
                (anchor_abs, ac, end_abs, ec)
            } else {
                (end_abs, ec, anchor_abs, ac)
            };

        // Iterate from start (top, older) to end (bottom, newer), varying scrollback as needed.
        let mut result = String::new();
        let mut abs_pos = start_abs;
        while abs_pos >= end_abs_pos {
            let is_first = abs_pos == start_abs;
            let is_last = abs_pos == end_abs_pos;
            let col_start = if is_first { start_col } else { 0 };
            let col_end = if is_last { end_col } else { screen_cols };

            // Pick a scrollback value that brings abs_pos into the visible viewport.
            // row_in_view = screen_rows - 1 + target_sb - abs_pos  (must be in [0, screen_rows))
            // Use target_sb = max(0, abs_pos - (screen_rows - 1)) to place the row at the top
            // when it's deep in history, or at natural position otherwise.
            let target_sb = (abs_pos - (screen_rows - 1)).max(0);
            let row_in_view = screen_rows - 1 + target_sb - abs_pos;
            if row_in_view < 0 || row_in_view >= screen_rows {
                abs_pos -= 1;
                continue;
            }
            screen.set_scrollback(target_sb as usize);

            let mut line = String::new();
            for col in col_start..col_end {
                if let Some(cell) = screen.cell(row_in_view as u16, col) {
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

            if abs_pos < start_abs {
                result.push('\n');
            }
            result.push_str(line.trim_end());

            if abs_pos == 0 && end_abs_pos == 0 {
                break;
            }
            abs_pos -= 1;
        }

        // Restore original scrollback position.
        screen.set_scrollback(original_sb);

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Extend keyboard selection in a given direction.
    pub fn select_kb(&mut self, dir: (i16, i16), screen_rows: u16, screen_cols: u16) {
        // Initialize cursor at bottom-left of screen if not yet active.
        let cursor = self
            .kb_cursor
            .get_or_insert((screen_rows.saturating_sub(1), 0));
        if self.anchor.is_none() {
            self.anchor = Some(*cursor);
            self.end = Some(*cursor);
        }
        let new_row =
            (cursor.0 as i16 + dir.0).clamp(0, screen_rows.saturating_sub(1) as i16) as u16;
        let new_col =
            (cursor.1 as i16 + dir.1).clamp(0, screen_cols.saturating_sub(1) as i16) as u16;
        *cursor = (new_row, new_col);
        self.end = Some(*cursor);
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
            let style = if selection.is_selected(row, col, screen) {
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
        render_cursor(screen, area, buf);
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

    // Terminal content starts below the border row.
    let content = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    render_terminal_screen(screen, content, buf, focused, selection);
}

/// Render the cursor block at the correct position.
fn render_cursor(screen: &vt100::Screen, area: Rect, buf: &mut RataBuf) {
    let (cursor_row, cursor_col) = screen.cursor_position();
    let cx = area.x + cursor_col;
    let cy = area.y + cursor_row;
    if cx < buf.area().right() && cy < buf.area().bottom() {
        let cursor_style = ratatui::style::Style::default()
            .fg(theme::CURSOR_INVERT_FG)
            .bg(theme::TEXT_FG);
        buf[(cx, cy)].set_style(cursor_style);
    }
}

/// Convert a crossterm KeyEvent to the byte sequence expected by the terminal.
///
/// `application_cursor` is the DECCKM mode the foreground app negotiated
/// (`screen().application_cursor()`): full-screen apps like vim/less expect
/// SS3 (`ESC O _`) cursor sequences in that mode, CSI (`ESC [ _`) otherwise.
pub fn key_event_to_bytes(key: &KeyEvent, application_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // CSI vs SS3 introducer for unmodified cursor keys (DECCKM).
    let cursor_intro = if application_cursor { b'O' } else { b'[' };

    // AltGr chords carry text, not a control chord (see
    // `platform::altgr_char`) — send the char verbatim, before the Ctrl arm
    // can mangle it into a control byte.
    if let Some(c) = crate::platform::altgr_char(key) {
        let mut char_buf = [0u8; 4];
        return c.encode_utf8(&mut char_buf).as_bytes().to_vec();
    }

    match key.code {
        KeyCode::Char(c) if ctrl => {
            let byte = (c.to_ascii_lowercase() as u8)
                .wrapping_sub(b'a')
                .wrapping_add(1);
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
        KeyCode::Up => vec![0x1b, cursor_intro, b'A'],
        KeyCode::Down => vec![0x1b, cursor_intro, b'B'],
        // Ctrl+Left / Ctrl+Right → xterm modifier-5 arrow sequences so the
        // shell's line editor (readline / PSReadLine) performs backward-word /
        // forward-word instead of single-char motion. Must precede the plain
        // arrow arms; always CSI-form regardless of DECCKM (xterm behavior
        // for modified cursor keys). (Ctrl+Up/Down never reach here —
        // is_terminal_escape_key routes them to panel resize.)
        KeyCode::Right if ctrl => vec![0x1b, b'[', b'1', b';', b'5', b'C'],
        KeyCode::Left if ctrl => vec![0x1b, b'[', b'1', b';', b'5', b'D'],
        KeyCode::Right => vec![0x1b, cursor_intro, b'C'],
        KeyCode::Left => vec![0x1b, cursor_intro, b'D'],
        KeyCode::Home => vec![0x1b, cursor_intro, b'H'],
        KeyCode::End => vec![0x1b, cursor_intro, b'F'],
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

/// Encode pasted text for the PTY the way the foreground application expects.
///
/// If the application enabled bracketed-paste mode (vt100 tracks DECSET 2004
/// via `screen().bracketed_paste()`), wrap the raw text in the paste markers
/// so vim/fzf/readline treat it as one unit. Otherwise convert newlines to CR,
/// matching the Enter key path (`key_event_to_bytes` maps Enter → `\r`): a raw
/// `\n` is ^J, which PSReadLine inserts as a soft line break (">>"
/// continuation) instead of executing the command.
pub fn paste_bytes(bracketed: bool, text: &str) -> Vec<u8> {
    if bracketed {
        let mut payload = b"\x1b[200~".to_vec();
        payload.extend_from_slice(text.as_bytes());
        payload.extend_from_slice(b"\x1b[201~");
        payload
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Whether an [`Action`] operates on the app frame rather than the focused
/// pane — these escape the embedded terminal instead of reaching the PTY.
///
/// Everything else deliberately stays with the PTY: Ctrl+C (SIGINT), Ctrl+Z
/// (SIGTSTP), Ctrl+D (EOF), Ctrl+A/E/K (readline), Ctrl+Left/Right
/// (shell word-nav — user-confirmed), Tab completion, F-keys, plain text.
fn action_escapes_terminal(action: &Action) -> bool {
    use Action::*;
    matches!(
        action,
        Quit | ToggleFileTree
            | ToggleSidePanel
            | ToggleTerminal
            | NewTab
            | CloseTab
            | FocusLeftPanel
            | FocusEditor
            | FocusSidePanel
            | FocusTerminal
            | SetLeftModeExplorer
            | SetLeftModeFind
            | SetLeftModeChanges
            | SetSideModeChat
            | SetSideModeSwarm
            | SetSideModeGit
            | SetSideModeMemory
            | ToggleAutoApprove
            | SwitchLayout(_)
            | CycleTabForward
            | CycleTabBack
            // Alt+Up/Down and the Ctrl+Up/Down fallback (Windows Terminal
            // steals Alt+arrows for pane navigation) both resize the split.
            // Ctrl+Alt+Left/Right resize explorer/editor/side widths
            // (Alt+Left/Right are reserved for tmux/psmux window nav).
            | MoveLineUp
            | MoveLineDown
            | ResizePanelUp
            | ResizePanelDown
            | ResizePanelLeft
            | ResizePanelRight
            // Keyboard text selection in the terminal (TUI-side, not PTY).
            | SelectUp
            | SelectDown
            | SelectLeft
            | SelectRight
            // Ctrl+V — paste is handled by the app, not forwarded raw.
            | Paste
    )
}

/// Returns true if this key should escape the terminal and go to the app
/// keymap. Derived from `Keymap::resolve` so new global bindings escape
/// automatically instead of desyncing from a second hand-written chord table.
pub fn is_terminal_escape_key(key: &KeyEvent) -> bool {
    // Shift+PageUp/PageDown page-scroll the terminal viewport. Explicit check:
    // Keymap::resolve maps PageUp/PageDown to the same action with or without
    // SHIFT, but the plain keys must reach the PTY.
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if shift && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        return true;
    }
    action_escapes_terminal(&Keymap::resolve(key))
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
    if cell.bold() {
        modifier |= Modifier::BOLD;
    }
    if cell.dim() {
        modifier |= Modifier::DIM;
    }
    if cell.italic() {
        modifier |= Modifier::ITALIC;
    }
    if cell.underline() {
        modifier |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifier |= Modifier::REVERSED;
    }

    Style::default().fg(fg).bg(bg).add_modifier(modifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn altgr_char_passes_through_verbatim() {
        // AltGr = CONTROL|ALT on Windows; Spanish AltGr+2 = '@' must
        // reach the PTY as the character, not a control byte.
        let key = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(key_event_to_bytes(&key, false), b"@".to_vec());
    }

    #[test]
    fn ctrl_char_maps_to_control_byte() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(&key, false), vec![0x03]);
    }

    #[test]
    fn ctrl_arrow_maps_to_word_motion() {
        // Ctrl+Left / Ctrl+Right must reach the shell as xterm modifier-5
        // arrow sequences so readline / PSReadLine do backward/forward-word,
        // not a bare arrow (single-char motion) — in both DECCKM modes.
        for app_cursor in [false, true] {
            let left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
            assert_eq!(key_event_to_bytes(&left, app_cursor), b"\x1b[1;5D".to_vec());
            let right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
            assert_eq!(key_event_to_bytes(&right, app_cursor), b"\x1b[1;5C".to_vec());
        }
    }

    #[test]
    fn cursor_keys_honor_application_mode() {
        // Normal mode: CSI; DECCKM application mode (vim/less): SS3.
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&up, false), b"\x1b[A".to_vec());
        assert_eq!(key_event_to_bytes(&up, true), b"\x1bOA".to_vec());
        let end = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&end, false), b"\x1b[F".to_vec());
        assert_eq!(key_event_to_bytes(&end, true), b"\x1bOF".to_vec());
    }

    #[test]
    fn paste_bytes_brackets_only_when_mode_enabled() {
        // App enabled DECSET 2004: raw text between markers, no CR munge.
        assert_eq!(
            paste_bytes(true, "a\nb"),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        // Mode off: newlines become CR so the shell executes lines.
        assert_eq!(paste_bytes(false, "a\r\nb\nc"), b"a\rb\rc".to_vec());
    }

    #[test]
    fn escape_keys_track_the_keymap() {
        let escapes = |code, mods| is_terminal_escape_key(&KeyEvent::new(code, mods));

        // Global chords escape — including the ones the old hand-written
        // table had desynced from the keymap (Alt+M, Alt+Y, Alt+digits).
        assert!(escapes(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(escapes(KeyCode::Char('m'), KeyModifiers::ALT));
        assert!(escapes(KeyCode::Char('y'), KeyModifiers::ALT));
        assert!(escapes(KeyCode::Char('5'), KeyModifiers::ALT));
        assert!(escapes(KeyCode::Up, KeyModifiers::CONTROL));
        assert!(escapes(KeyCode::PageUp, KeyModifiers::SHIFT));
        assert!(escapes(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(escapes(
            KeyCode::Left,
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ));
        assert!(escapes(
            KeyCode::Right,
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ));

        // PTY-bound keys stay with the shell.
        assert!(!escapes(KeyCode::Char('c'), KeyModifiers::CONTROL)); // SIGINT
        assert!(!escapes(KeyCode::Char('d'), KeyModifiers::CONTROL)); // EOF
        assert!(!escapes(KeyCode::Char('s'), KeyModifiers::CONTROL)); // XOFF
        assert!(!escapes(KeyCode::Left, KeyModifiers::CONTROL)); // word-nav
        assert!(!escapes(KeyCode::Char('a'), KeyModifiers::NONE)); // text
        assert!(!escapes(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(!escapes(KeyCode::Tab, KeyModifiers::NONE));
        assert!(!escapes(KeyCode::Enter, KeyModifiers::NONE));
    }
}
