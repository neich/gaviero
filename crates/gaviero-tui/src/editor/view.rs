use ratatui::{
    buffer::Buffer as RataBuf,
    layout::Rect,
    style::Color,
};
use unicode_width::UnicodeWidthChar;

use crate::theme::{CURRENT_LINE_BG, SELECTION_BG, SEARCH_HIGHLIGHT_BG};

use super::buffer::Buffer;
use super::highlight::{HighlightConfig, StyledSpan, run_highlights};
use crate::theme::Theme;

pub struct EditorView<'a> {
    pub buffer: &'a Buffer,
    pub theme: &'a Theme,
    pub highlight_config: Option<&'a HighlightConfig>,
    pub focused: bool,
}

impl<'a> EditorView<'a> {
    pub fn new(
        buffer: &'a Buffer,
        theme: &'a Theme,
        highlight_config: Option<&'a HighlightConfig>,
        focused: bool,
    ) -> Self {
        Self {
            buffer,
            theme,
            highlight_config,
            focused,
        }
    }

    /// Render the editor into the given area.
    pub fn render(&self, area: Rect, buf: &mut RataBuf) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let line_count = self.buffer.line_count();
        let gutter_width = gutter_width(line_count);
        // Reserve 1 column on the right for the scrollbar
        let scrollbar_width: u16 = 1;
        let code_area = Rect {
            x: area.x + gutter_width,
            y: area.y,
            width: area.width.saturating_sub(gutter_width + scrollbar_width),
            height: area.height,
        };

        // Compute highlights for visible range
        let spans = self.compute_highlights(code_area.height as usize);

        // Render each visible line
        let top = self.buffer.scroll.top_line;
        let default_style = self.theme.default_style();
        for row in 0..area.height as usize {
            let line_idx = top + row;
            let y = area.y + row as u16;

            if line_idx >= line_count {
                // Clear rows beyond end of file (gutter + code area)
                for col in 0..area.width {
                    let cx = area.x + col;
                    if cx < buf.area().right() {
                        buf[(cx, y)].set_char(' ').set_style(default_style);
                    }
                }
                continue;
            }

            // Gutter: line number
            self.render_gutter(line_idx, area.x, y, gutter_width, buf);

            // Code line
            self.render_code_line(line_idx, &spans, code_area.x, y, code_area.width, buf);
        }

        // Render cursor if focused
        if self.focused {
            self.render_cursor(code_area, buf);
        }

        // Render scrollbar on the right edge
        crate::widgets::scrollbar::render_scrollbar(
            area,
            buf,
            line_count,
            area.height as usize,
            self.buffer.scroll.top_line,
        );
    }

    fn render_gutter(
        &self,
        line_idx: usize,
        x: u16,
        y: u16,
        gutter_width: u16,
        buf: &mut RataBuf,
    ) {
        let is_current = line_idx == self.buffer.cursor.line;
        let style = if is_current {
            self.theme.ui_style("line_number.active")
        } else {
            self.theme.ui_style("line_number")
        };

        let num_str = format!("{:>width$} ", line_idx + 1, width = (gutter_width as usize) - 1);
        let x_max = (x + gutter_width).min(buf.area().right());
        for (i, ch) in num_str.chars().enumerate() {
            let cx = x + i as u16;
            if cx < x_max {
                buf[(cx, y)].set_char(ch).set_style(style);
            }
        }
    }

    fn render_code_line(
        &self,
        line_idx: usize,
        spans: &[StyledSpan],
        x: u16,
        y: u16,
        width: u16,
        buf: &mut RataBuf,
    ) {
        let line = self.buffer.text.line(line_idx);
        let left_col = self.buffer.scroll.left_col;
        let is_current = line_idx == self.buffer.cursor.line;

        // Background for current line
        let line_bg = if is_current {
            Some(CURRENT_LINE_BG)
        } else {
            None
        };

        // Determine the effective background for this row
        let row_bg = line_bg.unwrap_or(Color::Reset);

        // Clear the entire row with the base style so old chars don't linger
        let base_style = self.theme.default_style().bg(row_bg);
        for col in 0..width {
            let cell_x = x + col;
            if cell_x < buf.area().right() {
                buf[(cell_x, y)].set_char(' ').set_style(base_style);
            }
        }

        // Compute line byte start for span matching
        let line_byte_start = self.buffer.text.line_to_byte(line_idx);

        let tab_width = self.buffer.tab_width as usize;
        let mut char_idx: usize = 0; // index into rope chars (for selection/byte offset)
        let mut visual_col: usize = 0; // visual column (tabs expand)

        for ch in line.chars() {
            if ch == '\n' || ch == '\r' {
                break;
            }

            let byte_offset = line_byte_start + line.char_to_byte(char_idx);

            // Compute style for this character
            let mut style = self.theme.default_style();
            for span in spans.iter() {
                if span.start_byte <= byte_offset && byte_offset < span.end_byte {
                    if let Some(fg) = span.style.fg {
                        style = style.fg(fg);
                    }
                    if span.style.add_modifier.contains(ratatui::style::Modifier::BOLD) {
                        style = style.add_modifier(ratatui::style::Modifier::BOLD);
                    }
                    if span.style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED) {
                        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    }
                }
            }

            // Search highlight (orange background for matching chars)
            let in_search_match = self.is_in_search_match(line_idx, char_idx);

            if self.is_selected(line_idx, char_idx) {
                style = style.bg(SELECTION_BG);
            } else if in_search_match {
                style = style.bg(SEARCH_HIGHLIGHT_BG);
            } else {
                style = style.bg(row_bg);
            }

            if ch == '\t' {
                // Expand tab to spaces up to next tab stop
                let next_stop = (visual_col / tab_width + 1) * tab_width;
                let spaces = next_stop - visual_col;
                for _ in 0..spaces {
                    if visual_col >= left_col {
                        let display_col = (visual_col - left_col) as u16;
                        if display_col < width {
                            let cell_x = x + display_col;
                            if cell_x < buf.area().right() {
                                buf[(cell_x, y)].set_char(' ').set_style(style);
                            }
                        }
                    }
                    visual_col += 1;
                }
            } else {
                let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                if visual_col >= left_col {
                    let display_col = (visual_col - left_col) as u16;
                    if display_col + (ch_w as u16) <= width {
                        let cell_x = x + display_col;
                        if cell_x < buf.area().right() {
                            buf[(cell_x, y)].set_char(ch).set_style(style);
                        }
                    }
                }
                visual_col += ch_w;
            }

            char_idx += 1;
        }
    }

    fn render_cursor(&self, code_area: Rect, buf: &mut RataBuf) {
        let cursor_line = self.buffer.cursor.line;
        let cursor_col = self.buffer.cursor.col;
        let top = self.buffer.scroll.top_line;
        let left = self.buffer.scroll.left_col;

        if cursor_line >= self.buffer.line_count()
            || cursor_line < top
            || cursor_line >= top + code_area.height as usize
        {
            return;
        }

        let visual_col = self.char_col_to_visual(cursor_line, cursor_col);
        if visual_col < left {
            return;
        }

        let screen_row = (cursor_line - top) as u16;
        let screen_col = (visual_col - left) as u16;

        if screen_col < code_area.width {
            let x = code_area.x + screen_col;
            let y = code_area.y + screen_row;
            if x < buf.area().right() && y < buf.area().bottom() {
                let cursor_style = self.theme.ui_style("cursor");
                let cell = &mut buf[(x, y)];
                cell.set_style(cursor_style);
            }
        }
    }

    /// Convert a char-index column to a visual column (expanding tabs).
    fn char_col_to_visual(&self, line_idx: usize, char_col: usize) -> usize {
        let tab_width = self.buffer.tab_width as usize;
        let line = self.buffer.text.line(line_idx);
        let mut visual = 0;
        for (i, ch) in line.chars().enumerate() {
            if i >= char_col || ch == '\n' || ch == '\r' {
                break;
            }
            if ch == '\t' {
                visual = (visual / tab_width + 1) * tab_width;
            } else {
                visual += UnicodeWidthChar::width(ch).unwrap_or(1);
            }
        }
        visual
    }

    fn compute_highlights(&self, viewport_height: usize) -> Vec<StyledSpan> {
        let top = self.buffer.scroll.top_line;
        let bottom = (top + viewport_height).min(self.buffer.line_count());

        if top >= self.buffer.line_count() {
            return Vec::new();
        }

        let start_byte = self.buffer.text.line_to_byte(top);
        let end_byte = if bottom >= self.buffer.line_count() {
            self.buffer.text.len_bytes()
        } else {
            self.buffer.text.line_to_byte(bottom)
        };

        // Use regex-based markdown highlighter when there's no tree-sitter config
        if self.buffer.lang_name.as_deref() == Some("markdown") && self.highlight_config.is_none() {
            let source = self.buffer.text.to_string();
            return super::markdown::highlight_markdown(&source, self.theme, start_byte..end_byte);
        }

        let config = match self.highlight_config {
            Some(c) => c,
            None => return Vec::new(),
        };
        let tree = match &self.buffer.tree {
            Some(t) => t,
            None => return Vec::new(),
        };

        run_highlights(
            tree,
            &self.buffer.text,
            config,
            self.theme,
            start_byte..end_byte,
        )
    }

    /// Check if a character position is within the selection.
    fn is_selected(&self, line: usize, col: usize) -> bool {
        let anchor = match self.buffer.cursor.anchor {
            Some(a) => a,
            None => return false,
        };

        let head = (self.buffer.cursor.line, self.buffer.cursor.col);
        let (start, end) = if anchor <= head {
            (anchor, head)
        } else {
            (head, anchor)
        };

        if line < start.0 || line > end.0 {
            return false;
        }
        if line == start.0 && line == end.0 {
            return col >= start.1 && col < end.1;
        }
        if line == start.0 {
            return col >= start.1;
        }
        if line == end.0 {
            return col < end.1;
        }
        true
    }

    /// Check if a character position falls within a pre-computed search match.
    fn is_in_search_match(&self, line: usize, col: usize) -> bool {
        if self.buffer.search_highlight.is_none() {
            return false;
        }
        // Binary search or linear scan on pre-computed matches
        self.buffer.search_matches.iter().any(|&(l, start, end)| {
            l == line && col >= start && col < end
        })
    }
}

/// Calculate gutter width based on total line count.
fn gutter_width(line_count: usize) -> u16 {
    let digits = if line_count == 0 {
        1
    } else {
        ((line_count as f64).log10().floor() as u16) + 1
    };
    digits + 2 // digits + space + separator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gutter_width() {
        assert_eq!(gutter_width(1), 3);
        assert_eq!(gutter_width(9), 3);
        assert_eq!(gutter_width(10), 4);
        assert_eq!(gutter_width(99), 4);
        assert_eq!(gutter_width(100), 5);
        assert_eq!(gutter_width(1000), 6);
    }
}
