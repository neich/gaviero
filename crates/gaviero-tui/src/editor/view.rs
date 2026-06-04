use ratatui::{buffer::Buffer as RataBuf, layout::Rect, style::Color};
use unicode_width::UnicodeWidthChar;

use crate::theme::{CURRENT_LINE_BG, SEARCH_HIGHLIGHT_BG, SELECTION_BG};

use super::buffer::Buffer;
use super::diff::DiffKind;
use super::highlight::{HighlightConfig, StyledSpan, run_highlights};
use crate::theme::Theme;

/// Per-line backgrounds used to tint a diff-view buffer's rendered lines.
const DIFF_ADD_BG: Color = Color::Rgb(40, 65, 42);
const DIFF_REM_BG: Color = Color::Rgb(65, 40, 40);
const DIFF_ADD_GUTTER_FG: Color = Color::Rgb(80, 200, 80);
const DIFF_REM_GUTTER_FG: Color = Color::Rgb(220, 80, 80);

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

        let content_width = code_area.width as usize;
        let layout = self.buffer.wrap_layout(content_width);
        let scroll_lines = layout.len();

        // Compute highlights for visible range
        let spans = self.compute_highlights(code_area.height as usize, content_width);

        // Render each visible line (logical or wrapped visual row)
        let top = self.buffer.scroll.top_line;
        let default_style = self.theme.default_style();
        for row in 0..area.height as usize {
            let visual_idx = top + row;
            let y = area.y + row as u16;

            if visual_idx >= scroll_lines {
                for col in 0..area.width {
                    let cx = area.x + col;
                    if cx < buf.area().right() {
                        buf[(cx, y)].set_char(' ').set_style(default_style);
                    }
                }
                continue;
            }

            let seg = &layout.segments[visual_idx];
            self.render_gutter(
                seg.logical_line,
                seg.start_col == 0,
                area.x,
                y,
                gutter_width,
                buf,
            );
            self.render_code_line(
                seg.logical_line,
                seg.start_col,
                seg.end_col,
                &spans,
                code_area.x,
                y,
                code_area.width,
                buf,
            );
        }

        // Render cursor if focused
        if self.focused {
            self.render_cursor(code_area, &layout, buf);
        }

        // Render scrollbar on the right edge. In diff-view mode, mark the rows
        // that contain Added/Removed lines in red so the user can navigate to
        // changes that lie outside the current viewport.
        if let Some(dv) = self.buffer.diff_view.as_ref() {
            let diff_indices: Vec<usize> = dv
                .kinds
                .iter()
                .enumerate()
                .filter(|(_, k)| !matches!(k, DiffKind::Context))
                .map(|(i, _)| i)
                .collect();
            crate::widgets::scrollbar::render_scrollbar_with_diff_markers(
                area,
                buf,
                scroll_lines,
                area.height as usize,
                self.buffer.scroll.top_line,
                &diff_indices,
            );
        } else {
            crate::widgets::scrollbar::render_scrollbar(
                area,
                buf,
                scroll_lines,
                area.height as usize,
                self.buffer.scroll.top_line,
            );
        }
    }

    fn render_gutter(
        &self,
        line_idx: usize,
        show_line_number: bool,
        x: u16,
        y: u16,
        gutter_width: u16,
        buf: &mut RataBuf,
    ) {
        let is_current = line_idx == self.buffer.cursor.line;
        let diff_kind = self
            .buffer
            .diff_view
            .as_ref()
            .and_then(|dv| dv.kinds.get(line_idx).copied());

        // In diff view, recolor the gutter and replace the trailing space
        // with a `+` / `-` / ` ` indicator. Regular gutter otherwise.
        let style = match diff_kind {
            Some(DiffKind::Added) => ratatui::style::Style::default().fg(DIFF_ADD_GUTTER_FG),
            Some(DiffKind::Removed) => ratatui::style::Style::default().fg(DIFF_REM_GUTTER_FG),
            Some(DiffKind::Context) | None if is_current => self.theme.ui_style("line_number.active"),
            _ => self.theme.ui_style("line_number"),
        };

        let num_str = if !show_line_number {
            " ".repeat(gutter_width as usize)
        } else {
            match diff_kind {
                Some(DiffKind::Added) => format!(
                    "{:>width$}+",
                    line_idx + 1,
                    width = (gutter_width as usize) - 1
                ),
                Some(DiffKind::Removed) => format!(
                    "{:>width$}-",
                    line_idx + 1,
                    width = (gutter_width as usize) - 1
                ),
                _ => format!(
                    "{:>width$} ",
                    line_idx + 1,
                    width = (gutter_width as usize) - 1
                ),
            }
        };
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
        segment_start: usize,
        segment_end: usize,
        spans: &[StyledSpan],
        x: u16,
        y: u16,
        width: u16,
        buf: &mut RataBuf,
    ) {
        let line = self.buffer.text.line(line_idx);
        let left_col = if self.buffer.word_wrap {
            0
        } else {
            self.buffer.scroll.left_col
        };
        let is_current = line_idx == self.buffer.cursor.line;

        // Diff-view tint takes precedence over the current-line highlight.
        let diff_kind = self
            .buffer
            .diff_view
            .as_ref()
            .and_then(|dv| dv.kinds.get(line_idx).copied());

        // Background for current line / diff line
        let line_bg = match diff_kind {
            Some(DiffKind::Added) => Some(DIFF_ADD_BG),
            Some(DiffKind::Removed) => Some(DIFF_REM_BG),
            _ if is_current => Some(CURRENT_LINE_BG),
            _ => None,
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
        // Wrapped segments always start at display column 0; non-wrap uses absolute
        // visual columns so horizontal scroll (left_col) can offset the line.
        let mut visual_col: usize = 0;

        for ch in line.chars() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            if char_idx < segment_start {
                if !self.buffer.word_wrap {
                    if ch == '\t' {
                        visual_col = (visual_col / tab_width + 1) * tab_width;
                    } else {
                        visual_col += UnicodeWidthChar::width(ch).unwrap_or(1);
                    }
                }
                char_idx += 1;
                continue;
            }
            if char_idx >= segment_end {
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
                    if span
                        .style
                        .add_modifier
                        .contains(ratatui::style::Modifier::BOLD)
                    {
                        style = style.add_modifier(ratatui::style::Modifier::BOLD);
                    }
                    if span
                        .style
                        .add_modifier
                        .contains(ratatui::style::Modifier::UNDERLINED)
                    {
                        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    }
                }
            }

            // Search highlight (orange background for matching chars)
            let in_search_match = self.is_in_search_match(line_idx, char_idx);
            let in_conflict = self.buffer.is_line_in_conflict(line_idx);

            if self.is_selected(line_idx, char_idx) {
                style = style.bg(SELECTION_BG);
            } else if in_search_match {
                style = style.bg(SEARCH_HIGHLIGHT_BG);
            } else if in_conflict {
                style = style.bg(crate::theme::CONFLICT_MARKER_BG);
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

    fn render_cursor(
        &self,
        code_area: Rect,
        layout: &super::wrap::WrapLayout,
        buf: &mut RataBuf,
    ) {
        let cursor_line = self.buffer.cursor.line;
        let cursor_col = self.buffer.cursor.col;
        let top = self.buffer.scroll.top_line;
        let left = if self.buffer.word_wrap {
            0
        } else {
            self.buffer.scroll.left_col
        };

        let (vline, _) = layout.cursor_segment(cursor_line, cursor_col);
        if vline < top || vline >= top + code_area.height as usize {
            return;
        }

        let seg = match layout.segment_at(vline) {
            Some(s) => s,
            None => return,
        };
        let visual_col = self.buffer.char_col_to_visual(cursor_line, cursor_col);
        let base_visual = self
            .buffer
            .char_col_to_visual(seg.logical_line, seg.start_col);
        let rel_visual = visual_col.saturating_sub(base_visual);
        if rel_visual < left {
            return;
        }

        let screen_row = (vline - top) as u16;
        let screen_col = (rel_visual - left) as u16;

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

    fn compute_highlights(&self, viewport_height: usize, content_width: usize) -> Vec<StyledSpan> {
        let Some((start_byte, end_byte)) =
            self.buffer
                .highlight_viewport_byte_range(viewport_height, content_width)
        else {
            return Vec::new();
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

        // Diff-view buffers concatenate old + new lines so tree-sitter
        // (correctly) emits ERROR nodes throughout — suppress the parse-error
        // underline overlay there.
        let with_errors = self.buffer.diff_view.is_none();
        run_highlights(
            tree,
            &self.buffer.text,
            config,
            self.theme,
            start_byte..end_byte,
            with_errors,
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
        self.buffer
            .search_matches
            .iter()
            .any(|&(l, start, end)| l == line && col >= start && col < end)
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
