//! Shared scrollbar rendering for all panels.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};

const TRACK_FG: Color = Color::Rgb(60, 65, 75);
const TRACK_BG: Color = Color::Rgb(36, 40, 47);
const THUMB_FG: Color = Color::Rgb(110, 118, 135);
const THUMB_BG: Color = Color::Rgb(80, 86, 100);
const DIFF_MARKER_FG: Color = Color::Rgb(220, 80, 80);

/// Render a vertical scrollbar on the right edge of `area`.
///
/// - `total_items`: total number of scrollable units (lines, entries, etc.)
/// - `visible_items`: how many units fit in the viewport
/// - `scroll_offset`: index of the first visible unit
///
/// The scrollbar occupies the rightmost column of `area`.
/// If all content is visible (`total_items <= visible_items`), a subtle
/// full-height track is still drawn for visual consistency.
pub fn render_scrollbar(
    area: Rect,
    buf: &mut Buffer,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
) {
    render_scrollbar_inner(area, buf, total_items, visible_items, scroll_offset, &[]);
}

/// Render a vertical scrollbar with red marks at the rows that map to
/// `diff_indices` (item indices that contain a diff change). The marks make it
/// easy to navigate to changes that lie outside the current viewport.
pub fn render_scrollbar_with_diff_markers(
    area: Rect,
    buf: &mut Buffer,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
    diff_indices: &[usize],
) {
    render_scrollbar_inner(
        area,
        buf,
        total_items,
        visible_items,
        scroll_offset,
        diff_indices,
    );
}

fn render_scrollbar_inner(
    area: Rect,
    buf: &mut Buffer,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
    diff_indices: &[usize],
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let x = area.x + area.width.saturating_sub(1);
    let track_height = area.height as usize;

    let track_style = Style::default().fg(TRACK_FG).bg(TRACK_BG);
    let thumb_style = Style::default().fg(THUMB_FG).bg(THUMB_BG);
    let diff_track_style = Style::default().fg(DIFF_MARKER_FG).bg(TRACK_BG);
    let diff_thumb_style = Style::default().fg(DIFF_MARKER_FG).bg(THUMB_BG);

    let (thumb_start, thumb_size) = if total_items <= visible_items {
        (0, track_height)
    } else {
        let size = ((visible_items as f64 / total_items as f64) * track_height as f64)
            .round()
            .max(1.0) as usize;
        let max_scroll = total_items.saturating_sub(visible_items);
        let fraction = scroll_offset as f64 / max_scroll.max(1) as f64;
        let start = (fraction * track_height.saturating_sub(size).max(1) as f64).round() as usize;
        (start, size)
    };

    let thumb_end = thumb_start + thumb_size;

    let mut rows_with_diff = vec![false; track_height];
    if !diff_indices.is_empty() && total_items > 0 && track_height > 0 {
        for &idx in diff_indices {
            if idx < total_items {
                let row = (idx * track_height) / total_items.max(1);
                if row < track_height {
                    rows_with_diff[row] = true;
                }
            }
        }
    }

    for (row, &has_diff) in rows_with_diff.iter().enumerate() {
        let y = area.y + row as u16;
        if x < buf.area.right() && y < buf.area.bottom() {
            let in_thumb = row >= thumb_start && row < thumb_end;

            match (in_thumb, has_diff) {
                (true, true) => {
                    buf[(x, y)].set_char('█').set_style(diff_thumb_style);
                }
                (true, false) => {
                    buf[(x, y)].set_char('█').set_style(thumb_style);
                }
                (false, true) => {
                    buf[(x, y)].set_char('▎').set_style(diff_track_style);
                }
                (false, false) => {
                    buf[(x, y)].set_char('│').set_style(track_style);
                }
            }
        }
    }
}
