//! Shared scroll + selection state for list-based panels.
//!
//! Panels that display a scrollable list of items (file tree, search results,
//! git panel, swarm dashboard) all need the same offset/selection/viewport
//! logic. This module provides a single implementation.

/// Scroll offset and single-item selection for a list of `item_count` items
/// rendered into a viewport of a given height.
///
/// The `viewport` is cached from the most recent render pass so that event
/// handlers (which don't know the current terminal size) can call `move_up()`
/// / `move_down()` without passing dimensions.
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Index of the currently selected item (0-based).
    pub selected: usize,
    /// First visible item index.
    pub offset: usize,
    /// Last-known viewport height (set during render via `set_viewport`).
    viewport: usize,
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            selected: 0,
            offset: 0,
            viewport: usize::MAX, // large default — ensure_visible is a no-op until first render
        }
    }

    /// Reset to the top.
    pub fn reset(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    /// Update the cached viewport height. Call at the start of each render.
    pub fn set_viewport(&mut self, viewport: usize) {
        self.viewport = viewport;
    }

    /// Current cached viewport.
    #[allow(dead_code)]
    pub fn viewport(&self) -> usize {
        self.viewport
    }

    /// Move selection up by one. Adjusts scroll to keep selection visible.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.ensure_visible();
    }

    /// Move selection down by one. Adjusts scroll to keep selection visible.
    pub fn move_down(&mut self, item_count: usize) {
        if self.selected < item_count.saturating_sub(1) {
            self.selected += 1;
        }
        self.ensure_visible();
    }

    /// Jump selection up by a page.
    #[allow(dead_code)]
    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(self.viewport);
        self.ensure_visible();
    }

    /// Jump selection down by a page.
    #[allow(dead_code)]
    pub fn page_down(&mut self, item_count: usize) {
        self.selected = (self.selected + self.viewport).min(item_count.saturating_sub(1));
        self.ensure_visible();
    }

    /// Scroll the viewport up by `n` lines without moving selection.
    pub fn scroll_up(&mut self, n: usize) {
        self.offset = self.offset.saturating_sub(n);
    }

    /// Scroll the viewport down by `n` lines without moving selection.
    pub fn scroll_down(&mut self, n: usize, item_count: usize) {
        let max = item_count.saturating_sub(1);
        self.offset = (self.offset + n).min(max);
    }

    /// Select a specific item (e.g. from a mouse click). Clamps to valid range.
    #[allow(dead_code)]
    pub fn select(&mut self, index: usize, item_count: usize) {
        self.selected = index.min(item_count.saturating_sub(1));
    }

    /// Ensure the selected item is within the visible viewport.
    pub fn ensure_visible(&mut self) {
        if self.viewport == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.viewport {
            self.offset = self.selected - self.viewport + 1;
        }
    }

    /// Clamp selection to valid range (call after item_count changes).
    #[allow(dead_code)]
    pub fn clamp(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            self.offset = 0;
        } else if self.selected >= item_count {
            self.selected = item_count - 1;
        }
    }

    /// Iterator range of visible item indices for the given viewport height.
    pub fn visible_range(&self, item_count: usize, viewport: usize) -> std::ops::Range<usize> {
        let start = self.offset;
        let end = (self.offset + viewport).min(item_count);
        start..end
    }
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_viewport(viewport: usize) -> ScrollState {
        let mut s = ScrollState::new();
        s.set_viewport(viewport);
        s
    }

    #[test]
    fn move_down_clamps_to_last() {
        let mut s = with_viewport(10);
        s.move_down(3); // 3 items
        s.move_down(3);
        s.move_down(3);
        s.move_down(3); // should stay at 2
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn move_up_clamps_to_zero() {
        let mut s = with_viewport(10);
        s.move_up();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn ensure_visible_scrolls_down() {
        let mut s = with_viewport(10);
        s.selected = 15;
        s.ensure_visible();
        assert_eq!(s.offset, 6); // 15 - 10 + 1
    }

    #[test]
    fn ensure_visible_scrolls_up() {
        let mut s = ScrollState { selected: 2, offset: 5, viewport: 10 };
        s.ensure_visible();
        assert_eq!(s.offset, 2);
    }

    #[test]
    fn visible_range_basic() {
        let s = ScrollState { selected: 0, offset: 3, viewport: 5 };
        assert_eq!(s.visible_range(20, 5), 3..8);
    }

    #[test]
    fn visible_range_clamps_to_item_count() {
        let s = ScrollState { selected: 0, offset: 18, viewport: 5 };
        assert_eq!(s.visible_range(20, 5), 18..20);
    }

    #[test]
    fn page_down_clamps() {
        let mut s = with_viewport(10);
        s.page_down(5); // 5 items
        assert_eq!(s.selected, 4); // last item
    }

    #[test]
    fn clamp_adjusts_selection() {
        let mut s = ScrollState { selected: 10, offset: 5, viewport: 10 };
        s.clamp(3);
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn scroll_without_moving_selection() {
        let mut s = ScrollState { selected: 5, offset: 0, viewport: 10 };
        s.scroll_down(3, 20);
        assert_eq!(s.offset, 3);
        assert_eq!(s.selected, 5); // unchanged
        s.scroll_up(2);
        assert_eq!(s.offset, 1);
        assert_eq!(s.selected, 5); // unchanged
    }
}
