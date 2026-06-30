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
    /// When set, the next render must scroll the selection into view. Selection-
    /// changing operations set this; wheel scrolling does not, so the offset is
    /// free to move past the selected item.
    pending_focus: bool,
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            selected: 0,
            offset: 0,
            viewport: usize::MAX, // large default — ensure_visible is a no-op until first render
            pending_focus: true,
        }
    }

    /// Reset to the top.
    pub fn reset(&mut self) {
        self.selected = 0;
        self.offset = 0;
        self.pending_focus = true;
    }

    /// Update the cached viewport height. Call at the start of each render.
    pub fn set_viewport(&mut self, viewport: usize) {
        self.viewport = viewport;
    }



    /// Move selection up by one. Adjusts scroll to keep selection visible.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.pending_focus = true;
        self.ensure_visible();
    }

    /// Move selection down by one. Adjusts scroll to keep selection visible.
    pub fn move_down(&mut self, item_count: usize) {
        if self.selected < item_count.saturating_sub(1) {
            self.selected += 1;
        }
        self.pending_focus = true;
        self.ensure_visible();
    }

    /// Move selection down by one viewport page. Adjusts scroll to keep selection visible.
    #[allow(dead_code)] // exercised by tests; reserved for list-panel paging
    pub fn page_down(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            self.offset = 0;
            self.pending_focus = true;
            return;
        }
        let page = self.viewport.max(1);
        self.selected = (self.selected + page).min(item_count - 1);
        self.pending_focus = true;
        self.ensure_visible();
    }

    /// Scroll the viewport up by `n` lines without moving selection.
    /// Does not pin the offset to the selected item — the selection may go
    /// off-screen, and the caller can keep scrolling freely.
    pub fn scroll_up(&mut self, n: usize) {
        self.offset = self.offset.saturating_sub(n);
    }

    /// Scroll the viewport down by `n` lines without moving selection.
    /// Same independence property as `scroll_up`.
    pub fn scroll_down(&mut self, n: usize, item_count: usize) {
        let max = item_count.saturating_sub(1);
        self.offset = (self.offset + n).min(max);
    }

    /// Select a specific item (e.g. from a mouse click). Clamps to valid range
    /// and requests visibility on next render.
    pub fn select(&mut self, index: usize, item_count: usize) {
        self.selected = index.min(item_count.saturating_sub(1));
        self.pending_focus = true;
        self.ensure_visible();
    }

    /// Set the selected index without forcing immediate visibility (viewport
    /// may be unknown, e.g. during session restore). The next render will
    /// scroll the selection into view.
    pub fn set_selected(&mut self, index: usize, item_count: usize) {
        self.selected = index.min(item_count.saturating_sub(1));
        self.pending_focus = true;
    }

    /// Clamp `selected` and `offset` to a list of `item_count` items.
    #[allow(dead_code)] // exercised by tests; reserved for list-panel paging
    pub fn clamp(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            self.offset = 0;
            self.pending_focus = true;
            return;
        }
        let last = item_count - 1;
        self.selected = self.selected.min(last);
        self.offset = self.offset.min(last);
        self.pending_focus = true;
        self.ensure_visible();
    }

    /// Ensure the selected item is within the visible viewport.
    pub fn ensure_visible(&mut self) {
        if self.viewport == 0 || self.viewport == usize::MAX {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.viewport {
            self.offset = self.selected - self.viewport + 1;
        }
    }

    /// Render-time visibility check. Only re-pins the offset to the selection
    /// when a selection-changing operation has occurred since the last render.
    /// Wheel scrolling does not trigger this, so the offset can scroll past
    /// the selected item.
    pub fn ensure_visible_on_render(&mut self) {
        if self.pending_focus {
            self.ensure_visible();
            self.pending_focus = false;
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
        let mut s = with_viewport(10);
        s.selected = 2;
        s.offset = 5;
        s.ensure_visible();
        assert_eq!(s.offset, 2);
    }

    #[test]
    fn visible_range_basic() {
        let mut s = with_viewport(5);
        s.offset = 3;
        assert_eq!(s.visible_range(20, 5), 3..8);
    }

    #[test]
    fn visible_range_clamps_to_item_count() {
        let mut s = with_viewport(5);
        s.offset = 18;
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
        let mut s = with_viewport(10);
        s.selected = 10;
        s.offset = 5;
        s.clamp(3);
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn scroll_without_moving_selection() {
        let mut s = with_viewport(10);
        s.selected = 5;
        s.offset = 0;
        s.scroll_down(3, 20);
        assert_eq!(s.offset, 3);
        assert_eq!(s.selected, 5); // unchanged
        s.scroll_up(2);
        assert_eq!(s.offset, 1);
        assert_eq!(s.selected, 5); // unchanged
    }

    #[test]
    fn wheel_scroll_can_push_selection_off_screen_after_render() {
        // Repro for the explorer-panel scroll bug: wheel scrolling stopped
        // when the selected item hit the viewport edge because render() was
        // re-pinning the offset to the selection on every frame.
        let mut s = with_viewport(5);
        s.selected = 0;
        s.offset = 0;
        s.ensure_visible_on_render(); // initial render clears pending focus

        s.scroll_down(3, 20); // wheel scroll past the (still-at-0) selection
        s.ensure_visible_on_render(); // simulate next render

        assert_eq!(s.offset, 3, "render should not snap offset back to selection");
        assert_eq!(s.selected, 0, "selection unchanged by wheel scroll");
    }

    #[test]
    fn move_down_re_pins_offset_after_wheel_scroll() {
        // Counterpart: keyboard navigation must still bring the selection
        // back into view, even after the user wheel-scrolled it off-screen.
        let mut s = with_viewport(5);
        s.selected = 0;
        s.offset = 0;
        s.ensure_visible_on_render(); // initial render clears pending focus

        s.scroll_down(10, 20);
        s.ensure_visible_on_render(); // render after wheel scroll: no re-pin
        assert_eq!(s.offset, 10);

        s.move_down(20); // keyboard arrow re-pins
        s.ensure_visible_on_render();
        assert_eq!(s.selected, 1);
        assert!(
            s.offset <= s.selected,
            "offset {} should not exceed selected {}",
            s.offset,
            s.selected
        );
    }
}
