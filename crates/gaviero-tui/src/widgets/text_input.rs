//! Shared single-line/multi-line text input with char-indexed cursor,
//! selection, undo/redo, and word movement.
//!
//! Used by agent_chat (chat input) and git_panel (commit message input).

/// A text buffer with cursor, selection, and undo/redo support.
///
/// The cursor is always a **char index** (not byte offset). All public
/// methods maintain this invariant.
#[derive(Debug, Clone)]
pub struct TextInput {
    /// The text content.
    pub text: String,
    /// Cursor position as a char index (0 = before first char).
    pub cursor: usize,
    /// Selection anchor (char index). When `Some`, the selection spans
    /// from `anchor` to `cursor`.
    pub sel_anchor: Option<usize>,
    /// Undo stack: (text_snapshot, cursor_pos) before each edit.
    undo_stack: Vec<(String, usize)>,
    /// Redo stack: (text_snapshot, cursor_pos) for undone edits.
    redo_stack: Vec<(String, usize)>,
}

const MAX_UNDO: usize = 50;

impl TextInput {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            sel_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Create with initial text (cursor at end).
    #[allow(dead_code)]
    pub fn with_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            sel_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    // ── Cursor helpers ──────────────────────────────────────────

    /// Convert the char-index cursor to a byte offset.
    pub fn cursor_byte_offset(&self) -> usize {
        self.text
            .char_indices()
            .nth(self.cursor)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len())
    }

    /// Total char count.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Convert a char position to a byte offset.
    pub fn char_to_byte(&self, char_pos: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_pos)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len())
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Clear text, cursor, selection, and undo history.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.sel_anchor = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    // ── Editing ─────────────────────────────────────────────────

    /// Insert a character at the cursor, replacing any selection.
    pub fn insert_char(&mut self, ch: char) {
        self.push_undo();
        self.delete_selection_inner();
        let byte_pos = self.cursor_byte_offset();
        self.text.insert(byte_pos, ch);
        self.cursor += 1;
    }

    /// Insert a string at the cursor, replacing any selection.
    pub fn insert_str(&mut self, s: &str) {
        self.push_undo();
        self.delete_selection_inner();
        let byte_pos = self.cursor_byte_offset();
        self.text.insert_str(byte_pos, s);
        self.cursor += s.chars().count();
    }

    /// Delete the character before the cursor (or the selection).
    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selection_inner();
        } else if self.cursor > 0 {
            self.push_undo();
            self.cursor -= 1;
            let byte_pos = self.cursor_byte_offset();
            let ch_len = self.text[byte_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.text.drain(byte_pos..byte_pos + ch_len);
        }
    }

    /// Delete the character after the cursor (or the selection).
    pub fn delete(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selection_inner();
        } else if self.cursor < self.char_count() {
            self.push_undo();
            let byte_pos = self.cursor_byte_offset();
            let ch_len = self.text[byte_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.text.drain(byte_pos..byte_pos + ch_len);
        }
    }

    // ── Cursor movement ─────────────────────────────────────────

    /// Move cursor left by one char, clearing selection.
    pub fn move_left(&mut self) {
        self.sel_anchor = None;
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right by one char, clearing selection.
    pub fn move_right(&mut self) {
        self.sel_anchor = None;
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Move cursor to the beginning of the text.
    pub fn move_home(&mut self) {
        self.sel_anchor = None;
        self.cursor = 0;
    }

    /// Move cursor to the end of the text.
    pub fn move_end(&mut self) {
        self.sel_anchor = None;
        self.cursor = self.char_count();
    }

    // ── Word movement ───────────────────────────────────────────

    /// Move cursor left by one word.
    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor;
        // Skip non-word chars
        while pos > 0 && !chars[pos - 1].is_alphanumeric() && chars[pos - 1] != '_' {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && (chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_') {
            pos -= 1;
        }
        self.cursor = pos;
    }

    /// Move cursor right by one word.
    pub fn move_word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor;
        // Skip word chars
        while pos < len && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
            pos += 1;
        }
        // Skip non-word chars
        while pos < len && !chars[pos].is_alphanumeric() && chars[pos] != '_' {
            pos += 1;
        }
        self.cursor = pos;
    }

    /// Delete the word before the cursor.
    pub fn delete_word_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let old_cursor = self.cursor;
        self.move_word_left();
        let byte_start = self.cursor_byte_offset();
        let byte_end = self.char_to_byte(old_cursor);
        self.text.drain(byte_start..byte_end);
    }

    // ── Selection extension ────────────────────────────────────

    /// Extend selection one char to the left.
    pub fn select_left(&mut self) {
        self.ensure_anchor();
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Extend selection one char to the right.
    pub fn select_right(&mut self) {
        self.ensure_anchor();
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Extend selection one word to the left.
    pub fn select_word_left(&mut self) {
        self.ensure_anchor();
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor;
        while pos > 0 && !chars[pos - 1].is_alphanumeric() && chars[pos - 1] != '_' {
            pos -= 1;
        }
        while pos > 0 && (chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_') {
            pos -= 1;
        }
        self.cursor = pos;
    }

    /// Extend selection one word to the right.
    pub fn select_word_right(&mut self) {
        self.ensure_anchor();
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor;
        while pos < len && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
            pos += 1;
        }
        while pos < len && !chars[pos].is_alphanumeric() && chars[pos] != '_' {
            pos += 1;
        }
        self.cursor = pos;
    }

    // ── Selection ───────────────────────────────────────────────

    /// Whether the input has an active selection.
    pub fn has_selection(&self) -> bool {
        self.sel_anchor.is_some() && self.sel_anchor != Some(self.cursor)
    }

    /// Get the selection range as (start_char, end_char), normalized.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.sel_anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some(if anchor < self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        })
    }

    /// Delete the selected text. Returns true if something was deleted.
    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.push_undo();
        self.delete_selection_inner()
    }

    /// Select all text.
    pub fn select_all(&mut self) {
        self.sel_anchor = Some(0);
        self.cursor = self.char_count();
    }

    /// Clear selection without moving cursor.
    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
    }

    /// Ensure selection anchor is set (for extending selection).
    pub fn ensure_anchor(&mut self) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = Some(self.cursor);
        }
    }

    /// Get the selected text, if any.
    #[allow(dead_code)]
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection_range()?;
        let byte_start = self.char_to_byte(start);
        let byte_end = self.char_to_byte(end);
        Some(&self.text[byte_start..byte_end])
    }

    // ── Undo / Redo ─────────────────────────────────────────────

    /// Save current state to undo stack.
    pub fn push_undo(&mut self) {
        self.undo_stack
            .push((self.text.clone(), self.cursor));
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Undo the last edit.
    #[allow(dead_code)]
    pub fn undo(&mut self) {
        if let Some((text, cursor)) = self.undo_stack.pop() {
            self.redo_stack
                .push((self.text.clone(), self.cursor));
            self.text = text;
            self.cursor = cursor;
            self.sel_anchor = None;
        }
    }

    /// Redo the last undone edit.
    #[allow(dead_code)]
    pub fn redo(&mut self) {
        if let Some((text, cursor)) = self.redo_stack.pop() {
            self.undo_stack
                .push((self.text.clone(), self.cursor));
            self.text = text;
            self.cursor = cursor;
            self.sel_anchor = None;
        }
    }

    // ── Internal ────────────────────────────────────────────────

    /// Delete selection without pushing to undo (caller must push first).
    fn delete_selection_inner(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        let byte_start = self.char_to_byte(start);
        let byte_end = self.char_to_byte(end);
        self.text.drain(byte_start..byte_end);
        self.cursor = start;
        self.sel_anchor = None;
        true
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace() {
        let mut ti = TextInput::new();
        ti.insert_char('h');
        ti.insert_char('i');
        assert_eq!(ti.text, "hi");
        assert_eq!(ti.cursor, 2);
        ti.backspace();
        assert_eq!(ti.text, "h");
        assert_eq!(ti.cursor, 1);
    }

    #[test]
    fn insert_unicode() {
        let mut ti = TextInput::new();
        ti.insert_char('é');
        ti.insert_char('ñ');
        assert_eq!(ti.cursor, 2); // char count, not bytes
        assert_eq!(ti.char_count(), 2);
        ti.move_left();
        assert_eq!(ti.cursor, 1);
        ti.backspace();
        assert_eq!(ti.text, "ñ");
        assert_eq!(ti.cursor, 0);
    }

    #[test]
    fn selection_delete() {
        let mut ti = TextInput::new();
        ti.insert_str("hello world");
        ti.sel_anchor = Some(5);
        ti.cursor = 11;
        assert!(ti.has_selection());
        assert_eq!(ti.selection_range(), Some((5, 11)));
        ti.delete_selection();
        assert_eq!(ti.text, "hello");
        assert_eq!(ti.cursor, 5);
    }

    #[test]
    fn undo_redo() {
        let mut ti = TextInput::new();
        ti.insert_char('a');
        ti.insert_char('b');
        assert_eq!(ti.text, "ab");
        ti.undo();
        assert_eq!(ti.text, "a");
        ti.undo();
        assert_eq!(ti.text, "");
        ti.redo();
        assert_eq!(ti.text, "a");
    }

    #[test]
    fn word_movement() {
        let mut ti = TextInput::new();
        ti.insert_str("hello world foo");
        assert_eq!(ti.cursor, 15);
        ti.move_word_left();
        assert_eq!(ti.cursor, 12); // before "foo"
        ti.move_word_left();
        assert_eq!(ti.cursor, 6); // before "world"
        ti.move_word_left();
        assert_eq!(ti.cursor, 0);
    }

    #[test]
    fn delete_word_back() {
        let mut ti = TextInput::new();
        ti.insert_str("hello world");
        ti.delete_word_back();
        assert_eq!(ti.text, "hello ");
    }

    #[test]
    fn select_all_and_replace() {
        let mut ti = TextInput::new();
        ti.insert_str("old text");
        ti.select_all();
        ti.insert_char('x');
        assert_eq!(ti.text, "x");
        assert_eq!(ti.cursor, 1);
    }

    #[test]
    fn insert_str_replaces_selection() {
        let mut ti = TextInput::new();
        ti.insert_str("abcdef");
        ti.sel_anchor = Some(1);
        ti.cursor = 4;
        ti.insert_str("XY");
        assert_eq!(ti.text, "aXYef");
    }

    #[test]
    fn move_home_end() {
        let mut ti = TextInput::new();
        ti.insert_str("test");
        ti.move_home();
        assert_eq!(ti.cursor, 0);
        ti.move_end();
        assert_eq!(ti.cursor, 4);
    }
}
