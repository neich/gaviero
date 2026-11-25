//! Host drawing: the session tab bar on row 0, the active session's screen
//! below it.

use super::Host;
use super::session::Activity;
use crate::panels::terminal::{TerminalSelectionState, selection_style, vt100_style_to_ratatui};
use crate::widgets::tabs::{HOST_PALETTE, TabBadge, TabBadges, TabBar, TabKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// Copy a vt100 screen into `area`, cell by cell. Wide characters take
/// their own cell; the continuation cell after them is left alone so the
/// terminal draws the glyph across both.
pub(crate) fn render_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let (rows, cols) = screen.size();
    for row in 0..rows.min(area.height) {
        for col in 0..cols.min(area.width) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let contents = cell.contents();
            buf[(area.x + col, area.y + row)]
                .set_symbol(if contents.is_empty() { " " } else { contents })
                .set_style(vt100_style_to_ratatui(cell));
        }
    }
}

/// Badge palette, shared by both badges: green = working, yellow = waiting
/// for the user, blue = finished (a turn whose review is pending, or work
/// that ended while the tab was out of view). Green and yellow are pure hues
/// at 80% brightness (204/255); blue is lifted towards azure, because a pure
/// blue cell barely stands out on the dark tab backgrounds (on the active
/// tab, almost no contrast). The three tell apart at a glance.
const GREEN: Color = Color::Rgb(0, 204, 0);
const YELLOW: Color = Color::Rgb(204, 204, 0);
const BLUE: Color = Color::Rgb(51, 136, 255);
/// Agent badge (`A`, a plain capital the size of the label's text): an
/// agent or swarm is running / waits for the user (a permission, a
/// question) / finished, its turn review included.
const AGENT_LETTER: char = 'A';
const AGENT_RUNNING: Color = GREEN;
const AGENT_WAITING: Color = YELLOW;
const AGENT_DONE: Color = BLUE;
/// Shell badge (`S`): a command is running / seems to wait for the user (a
/// bell, a notification, or a quiet prompt line) / finished while out of
/// view.
const SHELL_LETTER: char = 'S';
const SHELL_RUNNING: Color = GREEN;
const SHELL_WAITING: Color = YELLOW;
const SHELL_DONE: Color = BLUE;

/// Every tab's label; a shell tab (its label already starts with
/// `shell::SHELL_MARK`) is drawn in its own teal, not the host palette.
pub(crate) fn tab_titles(host: &Host) -> Vec<(String, bool, TabKind)> {
    host.sessions
        .iter()
        .map(|s| {
            let kind = if s.shell.is_some() { TabKind::Shell } else { TabKind::Normal };
            (s.label.clone(), false, kind)
        })
        .collect()
}

/// The colours of a tab's two badges: the agent one, then the shell one.
/// The current tab shows them too; its agent "finished" mark stays until the
/// user leaves it (its shell one clears on arrival, `Host::switch_to`). A
/// pending turn review is finished, not waiting: the turn is
/// over, so it is blue like work that ended out of view (a running agent
/// still wins, the tab being busy).
pub(crate) fn badges(
    activity: Activity,
    agent_done_unseen: bool,
    shell_done_unseen: bool,
) -> [Option<Color>; 2] {
    let agent = if activity.waiting {
        Some(AGENT_WAITING)
    } else if activity.agent {
        Some(AGENT_RUNNING)
    } else if activity.review || agent_done_unseen {
        Some(AGENT_DONE)
    } else {
        None
    };
    let shell = if activity.shell_waiting {
        Some(SHELL_WAITING)
    } else if activity.shell {
        Some(SHELL_RUNNING)
    } else if shell_done_unseen {
        Some(SHELL_DONE)
    } else {
        None
    };
    [agent, shell]
}

pub(crate) fn tab_badges(host: &Host) -> Vec<TabBadges> {
    host.sessions
        .iter()
        .map(|s| {
            let [agent, shell] = badges(s.activity, s.agent_done_unseen, s.shell_done_unseen);
            [
                agent.map(|colour| TabBadge { letter: AGENT_LETTER, colour }),
                shell.map(|colour| TabBadge { letter: SHELL_LETTER, colour }),
            ]
        })
        .collect()
}

/// The host's tab bar: every tab has a close button and two activity
/// badges, a capital on a square of the state colour (`A` agent, `S` shell —
/// a shell tab only ever shows `S`), drawn in
/// the host's own palette so it never looks like the editor's tab row inside
/// the session below it; shell tabs in their own teal.
pub(crate) fn tab_bar<'a>(
    host: &Host,
    titles: &'a [(String, bool, TabKind)],
    badges: &'a [TabBadges],
) -> TabBar<'a> {
    TabBar {
        titles,
        active: host.active,
        closable: true,
        badges: Some(badges),
        palette: HOST_PALETTE,
    }
}

pub(crate) fn draw(host: &Host, frame: &mut Frame) {
    let area = frame.area();
    if area.height == 0 || area.width == 0 {
        return;
    }
    let bar = Rect { height: 1, ..area };
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };

    let titles = tab_titles(host);
    let badges = tab_badges(host);
    tab_bar(host, &titles, &badges).render(bar, frame.buffer_mut());
    if let Some(text) = host.bar_notice() {
        draw_notice(&text, bar, frame.buffer_mut());
    }

    // Quitting: tabs leave one by one and the current one moves along with
    // them, so drawing it would flash whatever another tab shows on its way
    // out (a tab still at its launch prompt, say). Show the quit instead.
    if host.quitting() {
        draw_confirm(
            " Quitting gaviero ",
            &[
                format!("Saving {} tab(s)…", host.sessions.len()),
                "Every tab reopens next start".to_string(),
            ],
            area,
            frame.buffer_mut(),
        );
        return;
    }

    if let Some(session) = host.sessions.get(host.active) {
        let screen = session.parser.screen();
        render_screen(screen, body, frame.buffer_mut());
        if session.shell.is_some() && session.selection.has_selection() {
            draw_selection(screen, &session.selection, body, frame.buffer_mut());
        }
        let scrolled = screen.scrollback();
        if scrolled > 0 {
            // A shell tab's history view: the live cursor is not on it.
            draw_history_mark(scrolled, body, frame.buffer_mut());
        } else if session.disconnected {
            let host_name = session.target.host().unwrap_or_default();
            draw_confirm(
                " Disconnected ",
                &[
                    format!("Connection to {host_name} lost."),
                    String::new(),
                    "Enter reconnect · Alt+Q close".to_string(),
                ],
                area,
                frame.buffer_mut(),
            );
        } else if !host.modal_open() && !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            if row < body.height && col < body.width {
                frame.set_cursor_position((body.x + col, body.y + row));
            }
        }
    }

    if let Some(picker) = &host.picker {
        picker.render(area, frame.buffer_mut());
    }
    if let Some((title, lines)) = host.confirm_popup() {
        draw_confirm(&title, &lines, area, frame.buffer_mut());
    }
}

/// The close guard: a centred box, the last line (the keys) dimmed.
fn draw_confirm(title: &str, lines: &[String], area: Rect, buf: &mut Buffer) {
    use ratatui::widgets::{Block, Borders, Clear, Widget};
    use unicode_width::UnicodeWidthStr;
    let widest = lines
        .iter()
        .map(|l| l.as_str().width())
        .chain(std::iter::once(title.width()))
        .max()
        .unwrap_or(0) as u16;
    let width = (widest + 4).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    if width < 10 || height < 3 {
        return;
    }
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    Clear.render(rect, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
        .border_style(Style::default().fg(Color::Rgb(229, 192, 123)))
        .style(Style::default().fg(Color::White).bg(Color::Rgb(35, 39, 46)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let last = lines.len().saturating_sub(1);
    for (row, line) in lines.iter().enumerate().take(inner.height as usize) {
        let style = if row == last {
            Style::default().fg(Color::Rgb(157, 165, 180))
        } else {
            Style::default()
        };
        buf.set_stringn(
            inner.x + 1,
            inner.y + row as u16,
            line,
            inner.width.saturating_sub(1) as usize,
            style,
        );
    }
}

/// A shell tab's mouse selection, over its screen: the selected cells take
/// the embedded terminal's selection colours; their text stays.
fn draw_selection(
    screen: &vt100::Screen,
    selection: &TerminalSelectionState,
    area: Rect,
    buf: &mut Buffer,
) {
    let style = selection_style();
    let (rows, cols) = screen.size();
    for row in 0..rows.min(area.height) {
        for col in 0..cols.min(area.width) {
            if selection.is_selected(row, col, screen) {
                buf[(area.x + col, area.y + row)].set_style(style);
            }
        }
    }
}

/// How far back a shell tab's history view is, at the top right of the
/// screen, and how to leave it.
fn draw_history_mark(lines: usize, body: Rect, buf: &mut Buffer) {
    use unicode_width::UnicodeWidthStr;
    if body.height == 0 {
        return;
    }
    let text = format!(" ↑ {lines} lines · type or wheel down to return ");
    let width = (text.as_str().width() as u16).min(body.width);
    let style = Style::default()
        .fg(Color::Rgb(210, 245, 238))
        .bg(Color::Rgb(26, 82, 86));
    buf.set_stringn(body.x + body.width - width, body.y, &text, width as usize, style);
}

/// A message or question right-aligned on the tab bar row.
fn draw_notice(text: &str, bar: Rect, buf: &mut Buffer) {
    use unicode_width::UnicodeWidthStr;
    let text = format!(" {text} ");
    let width = (text.as_str().width() as u16).min(bar.width);
    let x = bar.x + bar.width - width;
    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(229, 192, 123))
        .add_modifier(Modifier::BOLD);
    buf.set_stringn(x, bar.y, &text, width as usize, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_characters_take_two_columns() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process("a\u{4e2d}b".as_bytes());
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(1, 0)].symbol(), "\u{4e2d}");
        assert_eq!(buf[(3, 0)].symbol(), "b");
    }

    #[test]
    fn badge_letters_take_one_cell() {
        // The tab bar keeps fixed 3-cell slots (half block, letter, half
        // block); `widgets::tabs` checks the half blocks.
        use unicode_width::UnicodeWidthChar;
        for letter in [AGENT_LETTER, SHELL_LETTER] {
            assert_eq!(letter.width(), Some(1), "{letter:?}");
        }
    }

    #[test]
    fn badges_show_agent_and_shell_state() {
        let none = Activity::default();
        let running = Activity { agent: true, shell: true, ..none };
        let asking = Activity { waiting: true, ..running };
        assert_eq!(badges(none, false, false), [None, None]);
        assert_eq!(
            badges(running, false, false),
            [Some(AGENT_RUNNING), Some(SHELL_RUNNING)]
        );
        // Waiting for the user wins over running.
        assert_eq!(badges(asking, false, false)[0], Some(AGENT_WAITING));
        // A turn review pending after the turn: finished, so blue.
        let review = Activity { review: true, ..none };
        assert_eq!(badges(review, false, false), [Some(AGENT_DONE), None]);
        // A running agent still wins over a pending review.
        assert_eq!(
            badges(Activity { review: true, ..running }, false, false)[0],
            Some(AGENT_RUNNING)
        );
        // Finished while out of view.
        assert_eq!(badges(none, true, true), [Some(AGENT_DONE), Some(SHELL_DONE)]);
        // Running again wins over an earlier finish.
        assert_eq!(
            badges(running, true, true),
            [Some(AGENT_RUNNING), Some(SHELL_RUNNING)]
        );
        // A shell command waiting for the user wins over running.
        let prompting = Activity { shell_waiting: true, ..running };
        assert_eq!(badges(prompting, false, true)[1], Some(SHELL_WAITING));
    }
}
