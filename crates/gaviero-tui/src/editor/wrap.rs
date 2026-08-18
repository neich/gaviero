//! Visual line layout for word-wrapped editor rendering.

use unicode_width::UnicodeWidthChar;

use super::buffer::Buffer;

/// One display row: a slice of a logical rope line (char indices).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualSegment {
    pub logical_line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// Full wrap map for a buffer at a given content width.
#[derive(Clone, Debug, Default)]
pub struct WrapLayout {
    pub segments: Vec<VisualSegment>,
}

impl WrapLayout {
    /// Build visual segments for every logical line in `buffer`.
    pub fn build(buffer: &Buffer, content_width: usize) -> Self {
        if !buffer.word_wrap || content_width == 0 {
            let segments = (0..buffer.line_count())
                .map(|line| VisualSegment {
                    logical_line: line,
                    start_col: 0,
                    end_col: buffer.line_len(line),
                })
                .collect();
            return Self { segments };
        }

        let tab_width = buffer.tab_width as usize;
        let mut segments = Vec::new();
        for line in 0..buffer.line_count() {
            let text = buffer.text.line(line).to_string();
            for (start, end) in wrap_line_segments(&text, tab_width, content_width) {
                segments.push(VisualSegment {
                    logical_line: line,
                    start_col: start,
                    end_col: end,
                });
            }
        }
        Self { segments }
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Visual line index holding the logical cursor position.
    ///
    /// At a wrap boundary `col` is both the end of one segment and the start of
    /// the next; the later row wins, because that is where the character under
    /// the cursor is drawn (the earlier row has no cell left for it). Positions
    /// past the last segment's `end_col` (end of line) fall back to that row.
    pub fn cursor_segment(&self, line: usize, col: usize) -> usize {
        for (idx, seg) in self.segments.iter().enumerate() {
            if seg.logical_line == line && col >= seg.start_col && col < seg.end_col {
                return idx;
            }
        }
        self.segments
            .iter()
            .rposition(|s| s.logical_line == line)
            .unwrap_or(0)
    }

    pub fn segment_at(&self, visual_line: usize) -> Option<&VisualSegment> {
        self.segments.get(visual_line)
    }

    /// Whether `visual_line` is the final visual row of its logical line.
    ///
    /// Only that row may hold the one-past-the-last-char cursor position; on an
    /// earlier row `end_col` is already drawn on the row below.
    pub fn is_last_of_line(&self, visual_line: usize) -> bool {
        match (
            self.segments.get(visual_line),
            self.segments.get(visual_line + 1),
        ) {
            (Some(seg), Some(next)) => next.logical_line != seg.logical_line,
            _ => true,
        }
    }
}

/// Split one rope line into wrapped (start_col, end_col) char ranges.
pub fn wrap_line_segments(
    text: &str,
    tab_width: usize,
    content_width: usize,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    if content_width == 0 {
        return vec![(0, chars.len())];
    }

    let mut segments = Vec::new();
    let mut seg_start = 0usize;
    while seg_start < chars.len() {
        let mut visual_w = 0usize;
        let mut end = seg_start;

        while end < chars.len() {
            let ch = chars[end];
            let cw = char_display_width(ch, visual_w, tab_width);
            if visual_w + cw > content_width {
                break;
            }
            visual_w += cw;
            end += 1;
        }

        if end == seg_start {
            end = seg_start + 1;
        } else if end < chars.len() && chars[end] != ' ' {
            // Overflow mid-word: break after the last space in this segment.
            if let Some(rel) = chars[seg_start..end].iter().rposition(|c| *c == ' ') {
                let break_at = seg_start + rel + 1;
                if break_at > seg_start {
                    end = break_at;
                }
            }
        }

        segments.push((seg_start, end));
        seg_start = end;
    }
    segments
}

/// Display width of `ch` starting at `visual_col` *within its visual row*.
///
/// Tab stops are measured from the row's left edge, which is how both
/// [`wrap_line_segments`] and the renderer lay wrapped rows out.
pub(crate) fn char_display_width(ch: char, visual_col: usize, tab_width: usize) -> usize {
    if ch == '\t' {
        let next_stop = (visual_col / tab_width + 1) * tab_width;
        next_stop - visual_col
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_space() {
        let segs = wrap_line_segments("hello world foo", 4, 11);
        assert_eq!(segs, vec![(0, 11), (11, 15)]);
    }

    #[test]
    fn wrap_empty_line() {
        assert_eq!(wrap_line_segments("", 4, 10), vec![(0, 0)]);
    }

    #[test]
    fn cursor_segment_prefers_the_row_the_char_is_drawn_on() {
        let layout = WrapLayout {
            segments: vec![
                VisualSegment {
                    logical_line: 0,
                    start_col: 0,
                    end_col: 10,
                },
                VisualSegment {
                    logical_line: 0,
                    start_col: 10,
                    end_col: 19,
                },
            ],
        };
        assert_eq!(layout.cursor_segment(0, 9), 0);
        // Boundary: col 10 is drawn as the first cell of row 1, not past row 0.
        assert_eq!(layout.cursor_segment(0, 10), 1);
        // End of line stays on the last row.
        assert_eq!(layout.cursor_segment(0, 19), 1);
        assert!(!layout.is_last_of_line(0));
        assert!(layout.is_last_of_line(1));
    }

    #[test]
    fn wrap_expands_tabs() {
        let segs = wrap_line_segments("\tabc", 4, 8);
        // tab -> 4 cols, "abc" -> 3, fits in one segment
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], (0, 4));
    }
}
