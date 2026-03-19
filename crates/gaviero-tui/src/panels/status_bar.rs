use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

use crate::editor::buffer::Buffer as EditorBuffer;
use crate::theme::Theme;

pub struct StatusBar<'a> {
    pub buffer: Option<&'a EditorBuffer>,
    pub theme: &'a Theme,
    pub focus_label: &'a str,
    /// e.g. "sonnet" or "opus|t:25000"
    pub model_info: &'a str,
    /// Panel-specific context or transient message shown in the middle.
    pub context_info: &'a str,
}

impl<'a> StatusBar<'a> {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let style = self.theme.ui_style("status_bar");

        // Fill background
        for x in area.x..area.right() {
            buf[(x, area.y)].set_style(style);
        }

        let (left, center, right) = if let Some(editor_buf) = self.buffer {
            let modified = if editor_buf.modified { " [+]" } else { "" };
            let left = format!(
                " {} │ {}{}",
                self.focus_label,
                editor_buf.display_name(),
                modified
            );

            let cursor = format!(
                "Ln {}, Col {}",
                editor_buf.cursor.line + 1,
                editor_buf.cursor.col + 1
            );
            let lang = editor_buf
                .lang_name
                .as_deref()
                .unwrap_or("Plain Text");
            let right = format!("{} │ {} │ {} ", cursor, lang, self.model_info);

            // Transient message goes in the center
            let center = self.context_info.to_string();

            (left, center, right)
        } else {
            let left = format!(" {} │ {}", self.focus_label, self.context_info);
            let right = format!("{} ", self.model_info);
            (left, String::new(), right)
        };

        // Left side
        let right_len = right.len() as u16;
        let left_width = area.width.saturating_sub(right_len);
        let left_line = Line::from(Span::styled(&left, style));
        let left_area = Rect {
            x: area.x,
            y: area.y,
            width: left_width,
            height: 1,
        };
        Widget::render(left_line, left_area, buf);

        // Center (transient message) — between left text and right text
        if !center.is_empty() {
            let left_end = (area.x + left.len() as u16 + 2).min(area.right());
            let right_start = area.right().saturating_sub(right_len);
            if left_end < right_start {
                let center_width = right_start - left_end;
                let center_text = format!(" {} ", center);
                let center_line = Line::from(Span::styled(
                    center_text,
                    style,
                ));
                let center_area = Rect {
                    x: left_end,
                    y: area.y,
                    width: center_width,
                    height: 1,
                };
                Widget::render(center_line, center_area, buf);
            }
        }

        // Right side
        let right_x = area.right().saturating_sub(right_len);
        let right_line = Line::from(Span::styled(&right, style));
        let right_area = Rect {
            x: right_x,
            y: area.y,
            width: right_len.min(area.width),
            height: 1,
        };
        Widget::render(right_line, right_area, buf);
    }
}
