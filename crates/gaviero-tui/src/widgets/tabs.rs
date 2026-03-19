use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

pub struct TabBar<'a> {
    pub titles: &'a [(String, bool)], // (name, modified)
    pub active: usize,
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
        for (i, (title, modified)) in self.titles.iter().enumerate() {
            let is_active = i == self.active;
            let prefix = if *modified { "● " } else { "" };
            let label = format!(" {}{} ", prefix, title);
            let label_len = label.len() as u16;

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

            let style = if is_active {
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
        for (i, (title, modified)) in self.titles.iter().enumerate() {
            let prefix = if *modified { "● " } else { "" };
            let label_len = (format!(" {}{} ", prefix, title).len() as u16) + 1; // +1 for separator
            if click_x >= x && click_x < x + label_len {
                return Some(i);
            }
            x += label_len;
        }
        None
    }
}
