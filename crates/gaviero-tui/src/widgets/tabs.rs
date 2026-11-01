use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

/// How a tab is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Normal,
    /// A read-only diff overlay: orange, to signal it cannot be edited.
    DiffView,
    /// The global `$GAVIERO_ROOT/.gaviero/settings.json`, shared by every
    /// gaviero instance: violet, so an edit there is never mistaken for a
    /// workspace one.
    Global,
}

pub struct TabBar<'a> {
    /// (name, modified, kind).
    pub titles: &'a [(String, bool, TabKind)],
    pub active: usize,
}

/// Orange used for read-only diff-view tabs (e.g. git panel diff preview).
const DIFF_VIEW_FG: Color = Color::Rgb(214, 134, 50);
/// Violet for the global settings tab, with a tinted background.
const GLOBAL_FG: Color = Color::Rgb(198, 146, 233);
const GLOBAL_BG: Color = Color::Rgb(52, 40, 66);
const GLOBAL_ACTIVE_BG: Color = Color::Rgb(78, 56, 102);

/// The rendered label for a tab and its display width in terminal cells.
/// Width must be unicode-aware (`'●'` is 3 bytes / 1 cell, CJK chars are
/// 2 cells) or rendering and mouse hit-testing desync.
fn tab_label(title: &str, modified: bool) -> (String, u16) {
    use unicode_width::UnicodeWidthStr;
    let prefix = if modified { "● " } else { "" };
    let label = format!(" {}{} ", prefix, title);
    let width = label.as_str().width() as u16;
    (label, width)
}

impl<'a> TabBar<'a> {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Background — slightly darker than terminal bg to separate from editor
        let bg_style = Style::default().bg(Color::Rgb(35, 39, 46));
        for x in area.x..area.right() {
            buf[(x, area.y)].set_style(bg_style);
        }

        let mut x = area.x;
        for (i, (title, modified, kind)) in self.titles.iter().enumerate() {
            let is_active = i == self.active;
            let (label, label_len) = tab_label(title, *modified);

            if x + label_len > area.right() {
                // Show "..." if there are more tabs
                if x + 3 <= area.right() {
                    let dots = Line::from(Span::styled(
                        "...",
                        Style::default().fg(Color::Rgb(99, 109, 131)),
                    ));
                    let dots_area = Rect {
                        x,
                        y: area.y,
                        width: 3,
                        height: 1,
                    };
                    Widget::render(dots, dots_area, buf);
                }
                break;
            }

            let style = if *kind == TabKind::DiffView {
                let bg = if is_active {
                    Color::Rgb(55, 60, 70)
                } else {
                    Color::Rgb(35, 39, 46)
                };
                let mut s = Style::default().fg(DIFF_VIEW_FG).bg(bg);
                if is_active {
                    s = s.add_modifier(Modifier::BOLD);
                }
                s
            } else if *kind == TabKind::Global {
                let bg = if is_active { GLOBAL_ACTIVE_BG } else { GLOBAL_BG };
                let mut s = Style::default().fg(GLOBAL_FG).bg(bg);
                if is_active {
                    s = s.add_modifier(Modifier::BOLD);
                }
                s
            } else if is_active {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(55, 60, 70))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Rgb(157, 165, 180))
                    .bg(Color::Rgb(35, 39, 46))
            };

            let tab = Line::from(Span::styled(label, style));
            let tab_area = Rect {
                x,
                y: area.y,
                width: label_len,
                height: 1,
            };
            Widget::render(tab, tab_area, buf);

            // Separator
            x += label_len;
            if x < area.right() {
                buf[(x, area.y)]
                    .set_char('│')
                    .set_style(Style::default().fg(Color::Rgb(99, 109, 131)));
                x += 1;
            }
        }
    }

    /// Return the tab index at the given x coordinate, for mouse click handling.
    pub fn tab_at_x(&self, click_x: u16, area_x: u16) -> Option<usize> {
        let mut x = area_x;
        for (i, (title, modified, _)) in self.titles.iter().enumerate() {
            let label_len = tab_label(title, *modified).1 + 1; // +1 for separator
            if click_x >= x && click_x < x + label_len {
                return Some(i);
            }
            x += label_len;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_label_width_is_cells_not_bytes() {
        // '●' is 3 bytes but 1 cell; " ● a " = 5 cells.
        assert_eq!(tab_label("a", true).1, 5);
        // Plain ASCII: " a " = 3 cells.
        assert_eq!(tab_label("a", false).1, 3);
        // CJK chars occupy 2 cells each: " 日本 " = 2 + 4 = 6.
        assert_eq!(tab_label("日本", false).1, 6);
    }
}
