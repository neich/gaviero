//! Host drawing: the session tab bar on row 0, the active session's screen
//! below it.

use super::Host;
use super::session::Activity;
use crate::panels::terminal::vt100_style_to_ratatui;
use crate::widgets::tabs::{TabBadges, TabBar, TabKind};
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

/// Circle palette, shared by both circles: green = working, yellow = waiting
/// for the user, blue = finished while out of view.
const GREEN: Color = Color::Rgb(152, 195, 121);
const YELLOW: Color = Color::Rgb(229, 192, 123);
const BLUE: Color = Color::Rgb(97, 175, 239);
/// Agent circle: an agent or swarm is running / waits for the user (a
/// permission, a question, a turn review) / finished while out of view.
const AGENT_RUNNING: Color = GREEN;
const AGENT_WAITING: Color = YELLOW;
const AGENT_DONE: Color = BLUE;
/// Shell circle: a command is running / one finished while out of view.
const SHELL_RUNNING: Color = GREEN;
const SHELL_DONE: Color = BLUE;

pub(crate) fn tab_titles(host: &Host) -> Vec<(String, bool, TabKind)> {
    host.sessions
        .iter()
        .map(|s| (s.label.clone(), false, TabKind::Normal))
        .collect()
}

/// A tab's two circles: the agent one, then the shell one. The current tab
/// shows them too; its "finished" marks stay until the user leaves it.
pub(crate) fn badges(
    activity: Activity,
    agent_done_unseen: bool,
    shell_done_unseen: bool,
) -> TabBadges {
    let agent = if activity.waiting {
        Some(AGENT_WAITING)
    } else if activity.agent {
        Some(AGENT_RUNNING)
    } else if agent_done_unseen {
        Some(AGENT_DONE)
    } else {
        None
    };
    let shell = if activity.shell {
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
        .map(|s| badges(s.activity, s.agent_done_unseen, s.shell_done_unseen))
        .collect()
}

/// The host's tab bar: every tab has a close button and two activity
/// circles.
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

    if let Some(session) = host.sessions.get(host.active) {
        let screen = session.parser.screen();
        render_screen(screen, body, frame.buffer_mut());
        if !host.modal_open() && !screen.hide_cursor() {
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
    fn badges_show_agent_and_shell_state() {
        let none = Activity::default();
        let running = Activity { waiting: false, agent: true, shell: true };
        let asking = Activity { waiting: true, ..running };
        assert_eq!(badges(none, false, false), [None, None]);
        assert_eq!(
            badges(running, false, false),
            [Some(AGENT_RUNNING), Some(SHELL_RUNNING)]
        );
        // Waiting for the user wins over running.
        assert_eq!(badges(asking, false, false)[0], Some(AGENT_WAITING));
        // A turn review pending after the turn: waiting, not finished.
        let review = Activity { waiting: true, ..none };
        assert_eq!(badges(review, true, false), [Some(AGENT_WAITING), None]);
        // Finished while out of view.
        assert_eq!(badges(none, true, true), [Some(AGENT_DONE), Some(SHELL_DONE)]);
        // Running again wins over an earlier finish.
        assert_eq!(
            badges(running, true, true),
            [Some(AGENT_RUNNING), Some(SHELL_RUNNING)]
        );
    }
}
