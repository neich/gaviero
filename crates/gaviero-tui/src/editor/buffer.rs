use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use gaviero_core::{InputEdit, Language, Parser, Point, Tree};
use ropey::Rope;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug)]
pub struct Cursor {
    pub line: usize,   // 0-indexed line in rope
    pub col: usize,    // 0-indexed grapheme offset within line
    pub anchor: Option<(usize, usize)>, // Selection start (line, col), None if no selection
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            line: 0,
            col: 0,
            anchor: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scroll {
    pub top_line: usize,
    pub left_col: usize,
}

/// All positions are **char indices** into the Rope (not byte offsets).
#[derive(Clone, Debug)]
pub enum Change {
    Insert { pos: usize, text: String },
    Delete { pos: usize, len: usize, deleted: String },
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub changes: Vec<Change>,
    pub cursor_before: Cursor,
}

/// Format compactness level — controls how aggressively F5 reformats.
/// Cycled with F6. Think of it as a density dial: 0 = densest, 2 = most expanded.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FormatLevel {
    /// Fix indent only, never add or remove lines. Maximum density.
    /// Single-line lists, compact objects, manual alignment — all preserved.
    #[default]
    Compact,
    /// Fix indent + normalize spacing. Keeps single-line constructs that fit
    /// within a reasonable width, but may break overly long lines.
    Normal,
    /// Full reformat via external tools (rustfmt, etc.) or built-in
    /// pretty-printers. One element per line, everything expanded.
    Expanded,
}

impl FormatLevel {
    pub fn next(self) -> Self {
        match self {
            Self::Compact => Self::Normal,
            Self::Normal => Self::Expanded,
            Self::Expanded => Self::Compact,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Normal => "normal",
            Self::Expanded => "expanded",
        }
    }
}

pub struct Buffer {
    pub text: Rope,
    pub path: Option<PathBuf>,
    pub language: Option<Language>,
    pub lang_name: Option<String>,
    pub tree: Option<Tree>,
    pub modified: bool,
    pub cursor: Cursor,
    pub scroll: Scroll,
    pub undo_stack: Vec<Transaction>,
    pub redo_stack: Vec<Transaction>,
    parser: Option<Parser>,
    /// Tab display width (from settings, default 4).
    pub tab_width: u8,
    /// String used for one indent level (from settings, default "    ").
    pub indent_unit: String,
    /// Compiled indent query for this buffer's language (None = bracket fallback).
    pub indent_query: Option<Arc<gaviero_core::Query>>,
    /// Current format strictness level (cycled with F5).
    pub format_level: FormatLevel,
    /// Active search query to highlight in the editor (None = no highlights).
    pub search_highlight: Option<String>,
    /// Pre-computed search match ranges: Vec<(line, start_col, end_col)>.
    /// Recomputed when search_highlight changes.
    pub search_matches: Vec<(usize, usize, usize)>,
}

impl Buffer {
    /// Create an empty buffer with no file.
    pub fn empty() -> Self {
        Self {
            text: Rope::new(),
            path: None,
            language: None,
            lang_name: None,
            tree: None,
            modified: false,
            cursor: Cursor::default(),
            scroll: Scroll::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            parser: None,
            tab_width: 4,
            indent_unit: "    ".to_string(),
            indent_query: None,
            format_level: FormatLevel::default(),
            search_highlight: None,
            search_matches: Vec::new(),
        }
    }

    /// Open a file from disk.
    pub fn open(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let text = Rope::from_str(&content);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let language = gaviero_core::tree_sitter::language_for_extension(ext);
        let lang_name = gaviero_core::tree_sitter::language_name_for_extension(ext)
            .map(|s| s.to_string());

        let (tree, parser) = if let Some(ref lang) = language {
            let mut parser = Parser::new();
            parser
                .set_language(lang)
                .context("setting tree-sitter language")?;

            let tree = parser.parse(&content, None);
            (tree, Some(parser))
        } else {
            (None, None)
        };

        Ok(Self {
            text,
            path: Some(path.to_path_buf()),
            language,
            lang_name,
            tree,
            modified: false,
            cursor: Cursor::default(),
            scroll: Scroll::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            parser,
            tab_width: 4,
            indent_unit: "    ".to_string(),
            indent_query: None,
            format_level: FormatLevel::default(),
            search_highlight: None,
            search_matches: Vec::new(),
        })
    }

    /// Display name for the tab bar.
    pub fn display_name(&self) -> &str {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("[untitled]")
    }

    /// Apply a transaction: modify rope, update tree, push undo.
    pub fn apply(&mut self, transaction: Transaction) {
        for change in &transaction.changes {
            self.notify_tree_edit(change);
            match change {
                Change::Insert { pos, text } => {
                    self.text.insert(*pos, text);
                }
                Change::Delete { pos, len, .. } => {
                    self.text.remove(*pos..*pos + *len);
                }
            }
        }

        self.modified = true;
        self.redo_stack.clear();
        self.undo_stack.push(transaction);
        self.search_highlight = None; // Clear search highlight on any edit
        self.reparse();
    }

    /// Undo the last transaction.
    pub fn undo(&mut self) -> bool {
        let Some(transaction) = self.undo_stack.pop() else {
            return false;
        };

        // Apply inverse changes in reverse order
        for change in transaction.changes.iter().rev() {
            // Build the inverse change to notify tree-sitter
            let inverse = match change {
                Change::Insert { pos, text } => {
                    let char_len = text.chars().count();
                    Change::Delete { pos: *pos, len: char_len, deleted: text.clone() }
                }
                Change::Delete { pos, deleted, .. } => {
                    Change::Insert { pos: *pos, text: deleted.clone() }
                }
            };
            self.notify_tree_edit(&inverse);
            match change {
                Change::Insert { pos, text } => {
                    let char_len = text.chars().count();
                    self.text.remove(*pos..*pos + char_len);
                }
                Change::Delete { pos, deleted, .. } => {
                    self.text.insert(*pos, deleted);
                }
            }
        }

        let cursor_before = self.cursor.clone();
        self.cursor = transaction.cursor_before.clone();
        self.redo_stack.push(Transaction {
            changes: transaction.changes,
            cursor_before: cursor_before,
        });
        self.reparse();
        true
    }

    /// Redo the last undone transaction.
    pub fn redo(&mut self) -> bool {
        let Some(transaction) = self.redo_stack.pop() else {
            return false;
        };

        for change in &transaction.changes {
            self.notify_tree_edit(change);
            match change {
                Change::Insert { pos, text } => {
                    self.text.insert(*pos, text);
                }
                Change::Delete { pos, len, .. } => {
                    self.text.remove(*pos..*pos + *len);
                }
            }
        }

        let cursor_before = self.cursor.clone();
        self.cursor = transaction.cursor_before.clone();
        self.undo_stack.push(Transaction {
            changes: transaction.changes,
            cursor_before: cursor_before,
        });
        self.reparse();
        true
    }

    /// Insert a character at the current cursor position.
    /// If there is a selection, replace it.
    pub fn insert_char(&mut self, ch: char) {
        // Never insert carriage returns
        if ch == '\r' {
            return;
        }
        // Replace selection if any
        if self.cursor.anchor.is_some() {
            self.delete_selection();
        }

        let pos = self.cursor_char_pos();
        let text = ch.to_string();
        let transaction = Transaction {
            changes: vec![Change::Insert {
                pos,
                text: text.clone(),
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Advance cursor
        if ch == '\n' {
            self.cursor.line += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col += 1;
        }
        self.cursor.anchor = None;
    }

    /// Insert a newline with syntax-aware indentation.
    ///
    /// Uses `gaviero_core::indent::compute_indent` to determine the correct
    /// indentation for the new line based on the buffer's tree-sitter parse
    /// tree (when available) or bracket counting as fallback.
    pub fn insert_newline(&mut self) {
        if self.cursor.anchor.is_some() {
            self.delete_selection();
        }

        let cursor_byte = self.text.char_to_byte(self.cursor_char_pos());
        let result = gaviero_core::indent::compute_indent(
            &self.text,
            self.tree.as_ref(),
            self.indent_query.as_deref(),
            cursor_byte,
            true, // new_line_below
            self.tab_width,
            &self.indent_unit,
            gaviero_core::indent::IndentHeuristic::Hybrid,
        );

        let pos = self.cursor_char_pos();
        let text = format!("\n{}", result.whitespace);
        let indent_len = result.whitespace.len();

        let transaction = Transaction {
            changes: vec![Change::Insert {
                pos,
                text,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        self.cursor.line += 1;
        self.cursor.col = indent_len;
        self.cursor.anchor = None;
    }

    /// Insert a tab using the buffer's indent unit.
    pub fn insert_tab(&mut self) {
        if self.cursor.anchor.is_some() {
            self.delete_selection();
        }

        let unit = self.indent_unit.clone();
        let unit_len = unit.len();
        let pos = self.cursor_char_pos();
        let transaction = Transaction {
            changes: vec![Change::Insert {
                pos,
                text: unit,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
        self.cursor.col += unit_len;
        self.cursor.anchor = None;
    }

    /// Convert a visual column to a char-index column for a given line.
    /// Accounts for tab expansion.
    pub fn visual_to_char_col(&self, line: usize, visual_col: usize) -> usize {
        let tab_width = self.tab_width as usize;
        let rope_line = self.text.line(line);
        let mut visual = 0;
        let mut char_idx = 0;
        for ch in rope_line.chars() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            if visual >= visual_col {
                return char_idx;
            }
            if ch == '\t' {
                visual = (visual / tab_width + 1) * tab_width;
            } else {
                visual += UnicodeWidthChar::width(ch).unwrap_or(1);
            }
            char_idx += 1;
        }
        char_idx
    }

    /// Get the leading whitespace of the current line.
    fn current_line_indent(&self) -> String {
        let line = self.text.line(self.cursor.line);
        let mut indent = String::new();
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' {
                indent.push(ch);
            } else {
                break;
            }
        }
        indent
    }

    /// Delete the character before the cursor (Backspace).
    /// If there is a selection, deletes the selection instead.
    pub fn backspace(&mut self) -> bool {
        // Delete selection if any
        if self.cursor.anchor.is_some() {
            let deleted = self.delete_selection();
            return !deleted.is_empty();
        }

        if self.cursor.line == 0 && self.cursor.col == 0 {
            return false;
        }

        let pos = self.cursor_char_pos();
        if pos == 0 {
            return false;
        }

        let prev_char = self.text.char(pos - 1);

        // For CRLF: delete both \r and \n together
        let (del_pos, del_len, del_text) = if prev_char == '\n' && pos >= 2 && self.text.char(pos - 2) == '\r' {
            (pos - 2, 2, "\r\n".to_string())
        } else {
            (pos - 1, 1, prev_char.to_string())
        };

        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos: del_pos,
                len: del_len,
                deleted: del_text,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Move cursor back
        if prev_char == '\n' {
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
        } else {
            self.cursor.col -= 1;
        }
        self.cursor.anchor = None;
        true
    }

    /// Delete the character at the cursor (Delete key).
    /// If there is a selection, deletes the selection instead.
    pub fn delete(&mut self) -> bool {
        // Delete selection if any
        if self.cursor.anchor.is_some() {
            let deleted = self.delete_selection();
            return !deleted.is_empty();
        }

        let pos = self.cursor_char_pos();
        if pos >= self.text.len_chars() {
            return false;
        }

        let ch = self.text.char(pos);

        // Don't delete line endings — prevents unexpected line joining with Delete key
        if ch == '\n' || ch == '\r' {
            return false;
        }

        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos,
                len: 1,
                deleted: ch.to_string(),
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
        self.cursor.anchor = None;
        true
    }

    /// Move cursor in a direction.
    pub fn move_cursor_up(&mut self) {
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        }
        self.cursor.anchor = None;
    }

    pub fn move_cursor_down(&mut self) {
        if self.cursor.line < self.text.len_lines().saturating_sub(1) {
            self.cursor.line += 1;
            self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        }
        self.cursor.anchor = None;
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
        }
        self.cursor.anchor = None;
    }

    pub fn move_cursor_right(&mut self) {
        let line_len = self.line_len(self.cursor.line);
        if self.cursor.col < line_len {
            self.cursor.col += 1;
        } else if self.cursor.line < self.text.len_lines().saturating_sub(1) {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
        self.cursor.anchor = None;
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor.col = 0;
        self.cursor.anchor = None;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor.col = self.line_len(self.cursor.line);
        self.cursor.anchor = None;
    }

    /// Select (shift+arrow) variants — set anchor then move.
    pub fn select_up(&mut self) {
        self.ensure_anchor();
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        }
    }

    pub fn select_down(&mut self) {
        self.ensure_anchor();
        if self.cursor.line < self.text.len_lines().saturating_sub(1) {
            self.cursor.line += 1;
            self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        }
    }

    pub fn select_left(&mut self) {
        self.ensure_anchor();
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
        }
    }

    pub fn select_right(&mut self) {
        self.ensure_anchor();
        let line_len = self.line_len(self.cursor.line);
        if self.cursor.col < line_len {
            self.cursor.col += 1;
        } else if self.cursor.line < self.text.len_lines().saturating_sub(1) {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
    }

    /// Page up/down — move cursor and viewport by viewport_height lines.
    pub fn page_up(&mut self, viewport_height: usize) {
        self.cursor.line = self.cursor.line.saturating_sub(viewport_height);
        self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        self.cursor.anchor = None;
        self.scroll.top_line = self.scroll.top_line.saturating_sub(viewport_height);
    }

    pub fn page_down(&mut self, viewport_height: usize) {
        let max_line = self.text.len_lines().saturating_sub(1);
        self.cursor.line = (self.cursor.line + viewport_height).min(max_line);
        self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        self.cursor.anchor = None;
        self.scroll.top_line = (self.scroll.top_line + viewport_height).min(max_line);
    }

    /// Ensure the cursor is visible in the viewport.
    pub fn ensure_cursor_visible(&mut self, viewport_height: usize, viewport_width: usize) {
        // Vertical
        if self.cursor.line < self.scroll.top_line {
            self.scroll.top_line = self.cursor.line;
        }
        if self.cursor.line >= self.scroll.top_line + viewport_height {
            self.scroll.top_line = self.cursor.line - viewport_height + 1;
        }

        // Horizontal scroll margin — keep cursor this far from the edge
        const HORIZONTAL_SCROLL_MARGIN: usize = 8;
        let margin = HORIZONTAL_SCROLL_MARGIN;
        if self.cursor.col < self.scroll.left_col {
            self.scroll.left_col = self.cursor.col;
        }
        if self.cursor.col >= self.scroll.left_col + viewport_width.saturating_sub(margin) {
            self.scroll.left_col = self.cursor.col.saturating_sub(viewport_width - margin - 1);
        }
    }

    /// Clamp cursor and scroll positions to be within the current content bounds.
    /// Must be called after any operation that may shrink the buffer (reload, external edit).
    fn clamp_cursor(&mut self) {
        let max_line = self.text.len_lines().saturating_sub(1);
        self.cursor.line = self.cursor.line.min(max_line);
        self.cursor.col = self.cursor.col.min(self.line_len(self.cursor.line));
        self.cursor.anchor = None;
        self.scroll.top_line = self.scroll.top_line.min(max_line);
    }

    /// Reload content from disk (for auto-reload).
    pub fn reload(&mut self) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .context("cannot reload buffer without path")?;
        let content = std::fs::read_to_string(path)?;
        self.text = Rope::from_str(&content);
        self.modified = false;
        self.undo_stack.clear();
        self.redo_stack.clear();
        // Full reparse from scratch (not incremental) since the content was replaced entirely
        if let Some(parser) = &mut self.parser {
            self.tree = parser.parse(&content, None);
        }
        // Clamp cursor/scroll to new content bounds (file may have shrunk)
        self.clamp_cursor();
        Ok(())
    }

    /// Save buffer to its file path.
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .context("cannot save buffer without path")?;
        let content = self.text.to_string();
        std::fs::write(path, &content)?;
        self.modified = false;
        Ok(())
    }

    /// Format the buffer content at the current format level.
    ///
    /// F5 applies the current level. F6 cycles to the next level.
    /// - **Soft**: Only fix wrong indent depth. Preserves formatting choices.
    /// - **Standard**: Fix indent + use built-in formatters (JSON/TOML pretty-print).
    /// - **Strict**: Full reformat via external tools (rustfmt, clang-format, etc.).
    pub fn format(&mut self) -> String {
        let level = self.format_level;
        let lang = self.lang_name.clone().unwrap_or_default();
        let lang = lang.as_str();
        let content = self.text.to_string();

        let result = match level {
            FormatLevel::Compact => self.format_compact(lang, &content),
            FormatLevel::Normal => self.format_normal(lang, &content),
            FormatLevel::Expanded => self.format_expanded(lang, &content),
        };

        format!("{} [{}]", result, level.label())
    }

    /// Cycle to the next format level. Returns a label for the status bar.
    pub fn cycle_format_level(&mut self) -> String {
        self.format_level = self.format_level.next();
        format!("Format level: {}", self.format_level.label())
    }

    /// Compact (0): Maximum density. Collapse simple constructs to single lines.
    /// JSON: inline everything possible. Code: collapse + reindent.
    fn format_compact(&mut self, lang: &str, content: &str) -> String {
        match lang {
            "json" => {
                if let Some(formatted) = format_json_smart(content, &self.indent_unit, JsonCompactness::Compact) {
                    return self.apply_formatted(content, &formatted, "compact");
                }
            }
            "toml" => {
                if let Some(formatted) = format_toml(content) {
                    return self.apply_formatted(content, &formatted, "compact");
                }
            }
            _ => {}
        }
        // Code: collapse brackets then reindent
        let collapsed = collapse_multiline_constructs(content);
        if let (Some(_), Some(query_ref)) = (&self.tree, &self.indent_query) {
            if let Some(parser) = &mut self.parser {
                if let Some(new_tree) = parser.parse(&collapsed, None) {
                    let reindented = treesitter_reindent(&collapsed, &new_tree, query_ref, &self.indent_unit);
                    return self.apply_formatted(content, &reindented, "compact");
                }
            }
        }
        let reindented = gaviero_core::indent::bracket::reindent_document(&collapsed, &self.indent_unit);
        self.apply_formatted(content, &reindented, "compact")
    }

    /// Normal (1): Expand objects/dicts to one key per line, but keep
    /// short arrays/lists inline. Best balance of readability and density.
    fn format_normal(&mut self, lang: &str, content: &str) -> String {
        match lang {
            "json" => {
                if let Some(formatted) = format_json_smart(content, &self.indent_unit, JsonCompactness::Normal) {
                    return self.apply_formatted(content, &formatted, "normal");
                }
            }
            _ => {}
        }
        // Code: reindent only (preserve line structure)
        if let (Some(tree), Some(query)) = (&self.tree, &self.indent_query) {
            let reindented = treesitter_reindent(content, tree, query, &self.indent_unit);
            self.apply_formatted(content, &reindented, "re-indent")
        } else {
            let reindented = gaviero_core::indent::bracket::reindent_document(content, &self.indent_unit);
            self.apply_formatted(content, &reindented, "re-indent")
        }
    }

    /// Expanded (2): One element per line, everything expanded.
    /// Full reformat via external tools or built-in pretty-printers.
    fn format_expanded(&mut self, lang: &str, content: &str) -> String {
        // Try external tools first
        let external_result = match lang {
            "rust" => self.try_external_format("rustfmt", &["--edition".into(), "2024".into()], content),
            "python" => self.try_external_format("black", &["-q".into(), "-".into()], content),
            "c" | "cpp" => self.try_external_format("clang-format", &[], content),
            "java" => self.try_external_format("clang-format", &["--style=Google".into()], content),
            _ => None,
        };
        if let Some(result) = external_result {
            return result;
        }
        // JSON/TOML: standard pretty-print (one element per line)
        match lang {
            "json" => {
                if let Some(formatted) = format_json_smart(content, &self.indent_unit, JsonCompactness::Expanded) {
                    return self.apply_formatted(content, &formatted, "expanded");
                }
            }
            "toml" => {
                if let Some(formatted) = format_toml(content) {
                    return self.apply_formatted(content, &formatted, "expanded");
                }
            }
            _ => {}
        }
        // Fall back to normal
        self.format_normal(lang, content)
    }

    /// Try formatting with an external tool. Returns None if tool not found.
    fn try_external_format(&mut self, cmd: &str, args: &[String], content: &str) -> Option<String> {
        // Check if tool exists
        if std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            return None; // Not found — let caller try fallback
        }

        let mut child = match std::process::Command::new(cmd)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return None,
        };

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(content.as_bytes());
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return Some(format!("{} failed: {}", cmd, e)),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first_line = stderr.lines().next().unwrap_or("unknown error");
            return Some(format!("{} error: {}", cmd, first_line));
        }

        let formatted = match String::from_utf8(output.stdout) {
            Ok(s) => s,
            Err(_) => return Some(format!("{} produced invalid UTF-8", cmd)),
        };

        Some(self.apply_formatted(content, &formatted, cmd))
    }

    /// Apply formatted content to the buffer. Returns a status message.
    fn apply_formatted(&mut self, original: &str, formatted: &str, method: &str) -> String {
        if formatted == original {
            return "Already formatted".to_string();
        }

        let old_len = self.text.len_chars();
        let transaction = Transaction {
            changes: vec![
                Change::Delete {
                    pos: 0,
                    len: old_len,
                    deleted: original.to_string(),
                },
                Change::Insert {
                    pos: 0,
                    text: formatted.to_string(),
                },
            ],
            cursor_before: self.cursor.clone(),
        };

        // Use apply_full_replace for complete content replacement
        self.apply_full_replace(transaction);

        format!("Formatted ({})", method)
    }

    /// Reindent only the selected lines (expanded to whole lines).
    ///
    /// If the selection partially covers a line, the entire line is included.
    /// Only lines with wrong bracket-depth indentation are changed.
    pub fn format_selection(&mut self) -> String {
        let Some((sel_start, sel_end)) = self.selection_range() else {
            return "No selection".to_string();
        };

        if sel_start == sel_end {
            return "Empty selection".to_string();
        }

        // Expand to whole lines
        let start_line = self.text.char_to_line(sel_start);
        let end_line = self.text.char_to_line(sel_end.saturating_sub(1).max(sel_start));

        let content = self.text.to_string();
        let reindented = gaviero_core::indent::bracket::reindent_line_range(
            &content,
            start_line,
            end_line,
            &self.indent_unit,
        );

        if reindented == content {
            return "Already formatted".to_string();
        }

        self.cursor.anchor = None;
        self.apply_formatted(&content, &reindented, &format!("re-indent lines {}-{}", start_line + 1, end_line + 1))
    }

    // --- Internal helpers ---

    /// Get the char-index position of the cursor in the rope.
    fn cursor_char_pos(&self) -> usize {
        let line_start = self.text.line_to_char(self.cursor.line);
        let line_chars = self.text.line(self.cursor.line).len_chars();
        line_start + self.cursor.col.min(line_chars)
    }

    /// Length of a line in characters (excluding trailing line endings).
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.text.len_lines() {
            return 0;
        }
        let line_text = self.text.line(line);
        let mut len = line_text.len_chars();
        // Exclude trailing \n and \r\n
        if len > 0 && line_text.char(len - 1) == '\n' {
            len -= 1;
        }
        if len > 0 && line_text.char(len - 1) == '\r' {
            len -= 1;
        }
        len
    }

    /// Set anchor at current cursor position if not already set.
    fn ensure_anchor(&mut self) {
        if self.cursor.anchor.is_none() {
            self.cursor.anchor = Some((self.cursor.line, self.cursor.col));
        }
    }

    /// Select all text in the buffer.
    /// Select the word (identifier) at the cursor position.
    /// A word is a contiguous sequence of alphanumeric chars or underscores.
    /// Returns the selected word text, or empty string if cursor is not on a word.
    pub fn select_word_at_cursor(&mut self) -> String {
        let line = self.text.line(self.cursor.line);
        let line_str: String = line.into();
        let col = self.cursor.col;

        if col >= line_str.len() || line_str.is_empty() {
            return String::new();
        }

        let chars: Vec<char> = line_str.chars().collect();
        if !is_word_char(chars[col]) {
            return String::new();
        }

        // Find word start
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }

        // Find word end
        let mut end = col;
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }

        self.cursor.anchor = Some((self.cursor.line, start));
        self.cursor.col = end;

        chars[start..end].iter().collect()
    }

    pub fn select_all(&mut self) {
        self.cursor.anchor = Some((0, 0));
        let last_line = self.text.len_lines().saturating_sub(1);
        self.cursor.line = last_line;
        self.cursor.col = self.line_len(last_line);
    }

    /// Get the selection range as (start_char_pos, end_char_pos), ordered.
    /// Returns None if no selection.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let (anchor_line, anchor_col) = self.cursor.anchor?;
        let a_pos = {
            let line_start = self.text.line_to_char(anchor_line);
            let line_chars = self.text.line(anchor_line).len_chars();
            line_start + anchor_col.min(line_chars)
        };
        let b_pos = self.cursor_char_pos();
        if a_pos <= b_pos {
            Some((a_pos, b_pos))
        } else {
            Some((b_pos, a_pos))
        }
    }

    /// Get the selected text. Returns empty string if no selection.
    pub fn selected_text(&self) -> String {
        match self.selection_range() {
            Some((start, end)) if start < end => {
                self.text.slice(start..end).to_string()
            }
            _ => String::new(),
        }
    }

    /// Delete the selected text. Returns the deleted text (for cut).
    /// Moves cursor to start of selection. Returns empty string if no selection.
    pub fn delete_selection(&mut self) -> String {
        let Some((start, end)) = self.selection_range() else {
            return String::new();
        };
        if start == end {
            self.cursor.anchor = None;
            return String::new();
        }

        let deleted = self.text.slice(start..end).to_string();

        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos: start,
                len: end - start,
                deleted: deleted.clone(),
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Position cursor at selection start
        self.cursor.line = self.text.char_to_line(start);
        let line_start = self.text.line_to_char(self.cursor.line);
        self.cursor.col = start - line_start;
        self.cursor.anchor = None;

        deleted
    }

    /// Insert text at the cursor position, replacing any selection.
    /// Used for paste operations.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        // Normalize line endings
        let text = &text.replace('\r', "");
        if text.is_empty() {
            return;
        }

        // Delete selection first if any
        self.delete_selection();

        let pos = self.cursor_char_pos();
        let transaction = Transaction {
            changes: vec![Change::Insert {
                pos,
                text: text.to_string(),
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Advance cursor past inserted text
        let char_count = text.chars().count();
        let new_pos = pos + char_count;
        self.cursor.line = self.text.char_to_line(new_pos);
        let line_start = self.text.line_to_char(self.cursor.line);
        self.cursor.col = new_pos - line_start;
        self.cursor.anchor = None;
    }

    /// Paste text with indentation adjustment.
    ///
    /// The first line is inserted at the cursor. Subsequent lines are
    /// re-indented: the pasted text's base indent is replaced with
    /// the current line's indentation.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        // Normalize line endings: strip \r
        let text = &text.replace('\r', "");

        self.delete_selection();

        let lines: Vec<&str> = text.lines().collect();
        let trailing_newline = text.ends_with('\n');

        if lines.len() <= 1 {
            // Single line — insert as-is (trimming trailing newline)
            self.insert_text(lines.first().copied().unwrap_or(""));
            return;
        }

        // Determine the base indent of the pasted text (from first non-empty line)
        let paste_base_indent = lines.iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| {
                let trimmed = l.trim_start();
                &l[..l.len() - trimmed.len()]
            })
            .unwrap_or("");

        // The indent context at the cursor
        let target_indent = self.current_line_indent();

        // Build the adjusted text
        let mut result = String::new();

        // First line: insert at cursor position (no indent adjustment)
        result.push_str(lines[0]);

        // Subsequent lines: replace base indent with target indent
        for line in &lines[1..] {
            result.push('\n');
            if line.trim().is_empty() {
                // Blank line — just add newline
            } else if line.starts_with(paste_base_indent) {
                // Replace the pasted base indent with target indent
                result.push_str(&target_indent);
                result.push_str(&line[paste_base_indent.len()..]);
            } else {
                // Line has less indent than base — insert as-is
                result.push_str(line);
            }
        }

        if trailing_newline {
            result.push('\n');
        }

        self.insert_text(&result);
    }

    /// Delete the entire current line (Ctrl+K).
    pub fn delete_line(&mut self) {
        self.cursor.anchor = None;
        let line = self.cursor.line;
        let total = self.text.len_lines();
        if total == 0 {
            return;
        }

        let start = self.text.line_to_char(line);
        let end = if line + 1 < total {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };

        if start == end {
            return;
        }

        let deleted: String = self.text.slice(start..end).into();
        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos: start,
                len: end - start,
                deleted,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Adjust cursor
        let new_total = self.text.len_lines();
        if self.cursor.line >= new_total {
            self.cursor.line = new_total.saturating_sub(1);
        }
        let max_col = self.line_len(self.cursor.line);
        if self.cursor.col > max_col {
            self.cursor.col = max_col;
        }
    }

    /// Duplicate the current line below (Ctrl+D).
    pub fn duplicate_line(&mut self) {
        self.cursor.anchor = None;
        let line = self.cursor.line;
        let line_text: String = self.text.line(line).into();
        // Ensure we insert a full line (with newline)
        let insert_text = if line_text.ends_with('\n') {
            line_text
        } else {
            format!("\n{}", line_text)
        };

        let end_of_line = if line + 1 < self.text.len_lines() {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };

        let transaction = Transaction {
            changes: vec![Change::Insert {
                pos: end_of_line,
                text: insert_text,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
        self.cursor.line += 1;
    }

    /// Move the current line up (Alt+Up).
    pub fn move_line_up(&mut self) {
        if self.cursor.line == 0 {
            return;
        }
        self.cursor.anchor = None;
        let line = self.cursor.line;

        // Get current line text and previous line text
        let cur_start = self.text.line_to_char(line);
        let cur_end = if line + 1 < self.text.len_lines() {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };
        let prev_start = self.text.line_to_char(line - 1);

        let prev_text: String = self.text.slice(prev_start..cur_start).into();
        let cur_text: String = self.text.slice(cur_start..cur_end).into();

        // Swap: delete both lines, insert cur then prev
        let deleted: String = self.text.slice(prev_start..cur_end).into();
        let mut replacement = cur_text;
        if !replacement.ends_with('\n') {
            replacement.push('\n');
        }
        replacement.push_str(&prev_text);
        // Trim trailing newline if original didn't have one
        if cur_end == self.text.len_chars() && replacement.ends_with('\n') && !deleted.ends_with('\n') {
            replacement.pop();
        }

        let transaction = Transaction {
            changes: vec![
                Change::Delete {
                    pos: prev_start,
                    len: cur_end - prev_start,
                    deleted,
                },
                Change::Insert {
                    pos: prev_start,
                    text: replacement,
                },
            ],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
        self.cursor.line -= 1;
    }

    /// Move the current line down (Alt+Down).
    pub fn move_line_down(&mut self) {
        let line = self.cursor.line;
        if line + 1 >= self.text.len_lines() {
            return;
        }
        self.cursor.anchor = None;

        let cur_start = self.text.line_to_char(line);
        let next_start = self.text.line_to_char(line + 1);
        let next_end = if line + 2 < self.text.len_lines() {
            self.text.line_to_char(line + 2)
        } else {
            self.text.len_chars()
        };

        let cur_text: String = self.text.slice(cur_start..next_start).into();
        let next_text: String = self.text.slice(next_start..next_end).into();

        let deleted: String = self.text.slice(cur_start..next_end).into();
        let mut replacement = next_text;
        if !replacement.ends_with('\n') {
            replacement.push('\n');
        }
        replacement.push_str(&cur_text);
        if next_end == self.text.len_chars() && replacement.ends_with('\n') && !deleted.ends_with('\n') {
            replacement.pop();
        }

        let transaction = Transaction {
            changes: vec![
                Change::Delete {
                    pos: cur_start,
                    len: next_end - cur_start,
                    deleted,
                },
                Change::Insert {
                    pos: cur_start,
                    text: replacement,
                },
            ],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
        self.cursor.line += 1;
    }

    /// Delete from cursor to end of line (Ctrl+Delete).
    pub fn delete_to_line_end(&mut self) {
        self.cursor.anchor = None;
        let pos = self.cursor_char_pos();
        let line = self.cursor.line;
        let line_start = self.text.line_to_char(line);
        let line_chars = self.text.line(line).len_chars();
        // End position is end of line content (before newline)
        let end = line_start + if line_chars > 0 && self.text.line(line).as_str().map_or(false, |s| s.ends_with('\n')) {
            line_chars - 1
        } else {
            line_chars
        };

        if pos >= end {
            return;
        }

        let deleted: String = self.text.slice(pos..end).into();
        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos,
                len: end - pos,
                deleted,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);
    }

    /// Delete the word before the cursor (Ctrl+Backspace / Ctrl+H).
    pub fn delete_word_back(&mut self) {
        if self.cursor.anchor.is_some() {
            self.delete_selection();
            return;
        }

        let pos = self.cursor_char_pos();
        if pos == 0 {
            return;
        }

        // Walk backwards: skip whitespace, then skip word chars
        let mut start = pos;
        while start > 0 && self.text.char(start - 1) == ' ' {
            start -= 1;
        }
        while start > 0 {
            let ch = self.text.char(start - 1);
            if ch.is_alphanumeric() || ch == '_' {
                start -= 1;
            } else {
                break;
            }
        }
        // If we didn't move at all (cursor after a symbol), delete one char
        if start == pos {
            start = pos - 1;
        }

        let deleted: String = self.text.slice(start..pos).into();
        let transaction = Transaction {
            changes: vec![Change::Delete {
                pos: start,
                len: pos - start,
                deleted,
            }],
            cursor_before: self.cursor.clone(),
        };
        self.apply(transaction);

        // Recompute cursor position from char offset
        self.cursor.line = self.text.char_to_line(start);
        let line_start = self.text.line_to_char(self.cursor.line);
        self.cursor.col = start - line_start;
    }

    /// Apply a transaction that replaces the entire buffer content.
    /// Uses a full reparse (not incremental) since the old tree is invalid.
    fn apply_full_replace(&mut self, transaction: Transaction) {
        for change in &transaction.changes {
            match change {
                Change::Insert { pos, text } => self.text.insert(*pos, text),
                Change::Delete { pos, len, .. } => self.text.remove(*pos..*pos + *len),
            }
        }
        self.modified = true;
        self.redo_stack.clear();
        self.undo_stack.push(transaction);
        self.search_highlight = None;

        // Full reparse from scratch
        if let Some(parser) = &mut self.parser {
            let source = self.text.to_string();
            self.tree = parser.parse(&source, None);
        }

        // Clamp cursor
        let max_line = self.text.len_lines().saturating_sub(1);
        if self.cursor.line > max_line {
            self.cursor.line = max_line;
        }
        let max_col = self.line_len(self.cursor.line);
        if self.cursor.col > max_col {
            self.cursor.col = max_col;
        }
    }

    /// Set the search highlight query and pre-compute all match positions.
    pub fn set_search_highlight(&mut self, query: Option<String>) {
        self.search_highlight = query;
        self.search_matches.clear();

        let Some(ref q) = self.search_highlight else { return };
        if q.is_empty() { return; }

        let query_lower = q.to_lowercase();
        for line_idx in 0..self.text.len_lines() {
            let line: String = self.text.line(line_idx).into();
            let line_lower = line.to_lowercase();
            let mut from = 0;
            while let Some(pos) = line_lower[from..].find(&query_lower) {
                let start = from + pos;
                let end = start + q.len();
                self.search_matches.push((line_idx, start, end));
                from = start + 1;
            }
        }
    }

    /// Notify the tree-sitter tree about an edit so incremental parsing
    /// produces correct byte offsets. Must be called *before* modifying the rope.
    fn notify_tree_edit(&mut self, change: &Change) {
        let tree = match &mut self.tree {
            Some(t) => t,
            None => return,
        };

        let edit = match change {
            Change::Insert { pos, text } => {
                let start_byte = self.text.char_to_byte(*pos);
                let start_line = self.text.char_to_line(*pos);
                let line_start_byte = self.text.line_to_byte(start_line);
                let start_col = start_byte - line_start_byte;
                let start_position = Point::new(start_line, start_col);

                let new_end_byte = start_byte + text.len();
                let newline_count = text.matches('\n').count();
                let new_end_position = if newline_count == 0 {
                    Point::new(start_line, start_col + text.len())
                } else {
                    let after_last_newline = text.len() - text.rfind('\n').unwrap() - 1;
                    Point::new(start_line + newline_count, after_last_newline)
                };

                InputEdit {
                    start_byte,
                    old_end_byte: start_byte,
                    new_end_byte,
                    start_position,
                    old_end_position: start_position,
                    new_end_position,
                }
            }
            Change::Delete { pos, len, deleted } => {
                let start_byte = self.text.char_to_byte(*pos);
                let old_end_byte = self.text.char_to_byte(*pos + *len);
                let start_line = self.text.char_to_line(*pos);
                let line_start_byte = self.text.line_to_byte(start_line);
                let start_col = start_byte - line_start_byte;
                let start_position = Point::new(start_line, start_col);

                let newline_count = deleted.matches('\n').count();
                let old_end_position = if newline_count == 0 {
                    Point::new(start_line, start_col + (old_end_byte - start_byte))
                } else {
                    let after_last_newline = deleted.len() - deleted.rfind('\n').unwrap() - 1;
                    Point::new(start_line + newline_count, after_last_newline)
                };

                InputEdit {
                    start_byte,
                    old_end_byte,
                    new_end_byte: start_byte,
                    start_position,
                    old_end_position,
                    new_end_position: start_position,
                }
            }
        };

        tree.edit(&edit);
    }

    /// Re-parse the tree-sitter tree after an edit.
    fn reparse(&mut self) {
        if let Some(parser) = &mut self.parser {
            let source = self.text.to_string();
            self.tree = parser.parse(&source, self.tree.as_ref());
        }
    }

    /// Total line count.
    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }
}

// ── Built-in formatters ─────────────────────────────────────────

/// Pretty-print JSON using serde_json.
/// Reindent a document using tree-sitter indent queries.
///
/// Computes the expected indent for each line using `compute_indent` in
/// pure TreeSitter mode (not hybrid), and replaces wrong indentation.
/// Lines with correct indentation are left unchanged.
fn treesitter_reindent(
    content: &str,
    tree: &gaviero_core::Tree,
    query: &gaviero_core::Query,
    indent_unit: &str,
) -> String {
    let rope = ropey::Rope::from_str(content);
    if indent_unit.is_empty() {
        return content.to_string();
    }

    let mut result = String::with_capacity(content.len());

    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        // Compute expected indent for this line by placing cursor at end of previous line
        let expected_level = if i > 0 {
            let prev_line_text: String = rope.line(i - 1).into();
            let prev_trimmed_len = prev_line_text.trim_end_matches('\n').len();
            let cursor_byte = rope.line_to_byte(i - 1) + prev_trimmed_len;
            let r = gaviero_core::indent::treesitter::compute_treesitter_indent(
                &rope, tree, query, cursor_byte, true, 4, indent_unit,
            );
            r.level.max(0) as usize
        } else {
            0
        };

        // Compare actual whitespace against expected
        let expected_ws = indent_unit.repeat(expected_level);
        let actual_ws: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();

        if actual_ws == expected_ws {
            // Correct — keep the original line exactly as-is
            result.push_str(line);
        } else {
            // Wrong — reindent
            result.push_str(&expected_ws);
            result.push_str(trimmed);
        }
        result.push('\n');
    }

    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Collapse multi-line bracket constructs back to single lines.
///
/// Scans for `[...]\n...\n]`, `(...)\n...\n)`, `{...}\n...\n}` patterns
/// where the content is "simple" (no nested multi-line blocks) and collapses
/// them to a single line. Leaves complex/nested constructs alone.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn collapse_multiline_constructs(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = String::with_capacity(content.len());
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        let trailing = trimmed.chars().last();

        // Check if this line ends with an opening bracket
        let opener = match trailing {
            Some(c @ '[') | Some(c @ '(') => Some(c),
            // Only collapse { for data formats (when content looks like key-value pairs)
            // Skip for code blocks (functions, if-statements, etc.)
            _ => None,
        };

        if let Some(open_char) = opener {
            let close_char = match open_char {
                '[' => ']',
                '(' => ')',
                '{' => '}',
                _ => unreachable!(),
            };

            // Look ahead for the matching close bracket
            if let Some((close_line, inner)) = find_collapsible_block(&lines, i, close_char) {
                // Collapse: opening line content + inner items + close
                let opening = lines[i].trim_end();
                // Remove trailing open bracket from opening line
                let base = &opening[..opening.len() - 1];
                let collapsed = format!("{}{}{}{}", base, open_char, inner, close_char);

                // Only collapse if the result is reasonably short (< 100 chars)
                if collapsed.trim().len() <= 100 {
                    let indent: String = lines[i].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                    result.push_str(&indent);
                    result.push_str(collapsed.trim());
                    result.push('\n');
                    i = close_line + 1;
                    continue;
                }
            }
        }

        result.push_str(lines[i]);
        result.push('\n');
        i += 1;
    }

    // Match trailing newline of original
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Find a collapsible block starting at `start_line`.
/// Returns `(close_line_index, inner_content_as_single_line)` or None.
///
/// A block is collapsible if:
/// - All inner lines are simple (no nested multi-line blocks)
/// - The close bracket is on its own line
/// - No line comments in the inner content
fn find_collapsible_block(lines: &[&str], start_line: usize, close_char: char) -> Option<(usize, String)> {
    let mut depth = 1i32;
    let mut inner_parts: Vec<String> = Vec::new();

    for j in (start_line + 1)..lines.len() {
        let trimmed = lines[j].trim();

        if trimmed.is_empty() {
            continue;
        }

        // Count brackets on this line
        for ch in trimmed.chars() {
            match ch {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth -= 1,
                _ => {}
            }
        }

        // Check if this is the closing line
        if depth == 0 {
            // The close bracket should be alone or the start of the trimmed line
            if trimmed.starts_with(close_char) && trimmed.len() <= 2 {
                // Anything after the close char (like a comma)
                let suffix = &trimmed[1..];
                let inner = if inner_parts.is_empty() {
                    String::new()
                } else {
                    format!(" {} ", inner_parts.join(", "))
                };
                // Re-add suffix (comma, semicolon) if present
                return Some((j, format!("{}{}", inner.trim_end(), suffix)));
            }
            return None; // Close bracket is part of a more complex line
        }

        // If depth went above 1, this is a nested block — don't collapse
        if depth > 1 {
            return None;
        }

        // Skip lines with comments
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            return None;
        }

        // Collect the inner content (strip trailing comma for joining)
        let part = trimmed.trim_end_matches(',').to_string();
        if !part.is_empty() {
            inner_parts.push(part);
        }
    }

    None // No matching close found
}

/// JSON compactness mode for the smart formatter.
enum JsonCompactness {
    /// Everything on as few lines as possible.
    Compact,
    /// Objects expanded (one key per line), but short arrays stay inline.
    Normal,
    /// Everything expanded (one element per line) — standard pretty-print.
    Expanded,
}

/// Format JSON with configurable compactness.
fn format_json_smart(content: &str, indent_unit: &str, compactness: JsonCompactness) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let mut result = String::new();
    json_write_value(&value, &mut result, 0, indent_unit, &compactness);
    if !result.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

/// Recursively serialize a JSON value with compactness control.
fn json_write_value(
    value: &serde_json::Value,
    out: &mut String,
    depth: usize,
    indent_unit: &str,
    compactness: &JsonCompactness,
) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            out.push('"');
            // Escape the string properly
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                out.push_str("[]");
                return;
            }

            let inline = match compactness {
                JsonCompactness::Compact => true,
                JsonCompactness::Normal => {
                    // Inline if all elements are simple (not objects/arrays)
                    // and the result would be short
                    let all_simple = arr.iter().all(|v| !v.is_object() && !v.is_array());
                    let est_len: usize = arr.iter().map(|v| estimate_json_len(v)).sum::<usize>() + arr.len() * 2 + 2;
                    all_simple && est_len <= 80
                }
                JsonCompactness::Expanded => false,
            };

            if inline {
                out.push('[');
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    json_write_value(v, out, depth + 1, indent_unit, compactness);
                }
                out.push(']');
            } else {
                out.push_str("[\n");
                for (i, v) in arr.iter().enumerate() {
                    push_indent(out, depth + 1, indent_unit);
                    json_write_value(v, out, depth + 1, indent_unit, compactness);
                    if i + 1 < arr.len() { out.push(','); }
                    out.push('\n');
                }
                push_indent(out, depth, indent_unit);
                out.push(']');
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                out.push_str("{}");
                return;
            }

            let inline = match compactness {
                JsonCompactness::Compact => {
                    // Inline if all values are simple and result is short
                    let all_simple = obj.values().all(|v| !v.is_object() && !v.is_array());
                    let est_len: usize = obj.iter().map(|(k, v)| k.len() + 4 + estimate_json_len(v)).sum::<usize>() + 2;
                    all_simple && est_len <= 80
                }
                JsonCompactness::Normal | JsonCompactness::Expanded => false,
            };

            if inline {
                out.push('{');
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    out.push('"');
                    out.push_str(k);
                    out.push_str("\": ");
                    json_write_value(v, out, depth + 1, indent_unit, compactness);
                }
                out.push('}');
            } else {
                out.push_str("{\n");
                let len = obj.len();
                for (i, (k, v)) in obj.iter().enumerate() {
                    push_indent(out, depth + 1, indent_unit);
                    out.push('"');
                    out.push_str(k);
                    out.push_str("\": ");
                    json_write_value(v, out, depth + 1, indent_unit, compactness);
                    if i + 1 < len { out.push(','); }
                    out.push('\n');
                }
                push_indent(out, depth, indent_unit);
                out.push('}');
            }
        }
    }
}

/// Estimate the inline length of a JSON value.
fn estimate_json_len(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(b) => if *b { 4 } else { 5 },
        serde_json::Value::Number(n) => n.to_string().len(),
        serde_json::Value::String(s) => s.len() + 2,
        serde_json::Value::Array(a) => a.iter().map(|v| estimate_json_len(v) + 2).sum::<usize>() + 2,
        serde_json::Value::Object(o) => o.iter().map(|(k, v)| k.len() + 4 + estimate_json_len(v)).sum::<usize>() + 2,
    }
}

fn push_indent(out: &mut String, depth: usize, indent_unit: &str) {
    for _ in 0..depth {
        out.push_str(indent_unit);
    }
}

/// Pretty-print TOML using the toml crate.
fn format_toml(content: &str) -> Option<String> {
    let value: toml::Value = content.parse().ok()?;
    let mut formatted = toml::to_string_pretty(&value).ok()?;
    if !formatted.ends_with('\n') {
        formatted.push('\n');
    }
    Some(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_buffer() {
        let buf = Buffer::empty();
        assert_eq!(buf.line_count(), 1);
        assert!(!buf.modified);
    }

    #[test]
    fn test_insert_and_undo() {
        let mut buf = Buffer::empty();
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.text.to_string(), "hi");
        assert!(buf.modified);

        buf.undo();
        assert_eq!(buf.text.to_string(), "h");

        buf.undo();
        assert_eq!(buf.text.to_string(), "");
    }

    #[test]
    fn test_redo() {
        let mut buf = Buffer::empty();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.undo();
        assert_eq!(buf.text.to_string(), "a");
        buf.redo();
        assert_eq!(buf.text.to_string(), "ab");
    }

    #[test]
    fn test_backspace() {
        let mut buf = Buffer::empty();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.backspace();
        assert_eq!(buf.text.to_string(), "a");
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_newline_and_cursor() {
        let mut buf = Buffer::empty();
        buf.insert_char('a');
        buf.insert_newline();
        buf.insert_char('b');
        assert_eq!(buf.text.to_string(), "a\nb");
        assert_eq!(buf.cursor.line, 1);
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_cursor_movement() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello\nworld\n");
        buf.move_cursor_right();
        assert_eq!(buf.cursor.col, 1);
        buf.move_cursor_down();
        assert_eq!(buf.cursor.line, 1);
        buf.move_cursor_end();
        assert_eq!(buf.cursor.col, 5);
        buf.move_cursor_home();
        assert_eq!(buf.cursor.col, 0);
    }

    #[test]
    fn test_selection() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello");
        buf.select_right();
        assert!(buf.cursor.anchor.is_some());
        assert_eq!(buf.cursor.anchor, Some((0, 0)));
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_ensure_cursor_visible() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str(&"line\n".repeat(100));
        buf.cursor.line = 50;
        buf.ensure_cursor_visible(20, 80);
        assert!(buf.scroll.top_line <= 50);
        assert!(buf.scroll.top_line + 20 > 50);
    }

    #[test]
    fn test_backspace_then_insert_no_corruption() {
        // Regression: byte-vs-char confusion caused garbage after delete+insert
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello\n");
        buf.cursor.line = 0;
        buf.cursor.col = 5; // end of "hello"

        buf.backspace(); // delete 'o' → "hell\n"
        assert_eq!(buf.text.to_string(), "hell\n");
        assert_eq!(buf.cursor.col, 4);

        buf.backspace(); // delete 'l' → "hel\n"
        assert_eq!(buf.text.to_string(), "hel\n");
        assert_eq!(buf.cursor.col, 3);

        buf.insert_char(' '); // insert space → "hel \n"
        assert_eq!(buf.text.to_string(), "hel \n");
        assert_eq!(buf.cursor.col, 4);

        buf.insert_char('x'); // → "hel x\n"
        assert_eq!(buf.text.to_string(), "hel x\n");
    }

    #[test]
    fn test_utf8_editing() {
        // Ensure char-based positions work with multi-byte chars
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("café\n");
        buf.cursor.line = 0;
        buf.cursor.col = 4; // after 'é'

        buf.backspace(); // delete 'é' → "caf\n"
        assert_eq!(buf.text.to_string(), "caf\n");
        assert_eq!(buf.cursor.col, 3);

        buf.insert_char('!'); // → "caf!\n"
        assert_eq!(buf.text.to_string(), "caf!\n");
    }

    #[test]
    fn test_open_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let buf = Buffer::open(&path).unwrap();
        assert_eq!(buf.display_name(), "test.rs");
        assert!(buf.language.is_some());
        assert!(buf.tree.is_some());
        assert!(!buf.modified);
    }

    #[test]
    fn test_selected_text() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello world\n");
        // Select "hello" (anchor at 0,0, cursor at 0,5)
        buf.cursor.anchor = Some((0, 0));
        buf.cursor.col = 5;
        assert_eq!(buf.selected_text(), "hello");
    }

    #[test]
    fn test_selected_text_multiline() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("line one\nline two\n");
        buf.cursor.anchor = Some((0, 5));
        buf.cursor.line = 1;
        buf.cursor.col = 4;
        assert_eq!(buf.selected_text(), "one\nline");
    }

    #[test]
    fn test_delete_selection() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello world\n");
        buf.cursor.anchor = Some((0, 0));
        buf.cursor.col = 5;
        let deleted = buf.delete_selection();
        assert_eq!(deleted, "hello");
        assert_eq!(buf.text.to_string(), " world\n");
        assert_eq!(buf.cursor.col, 0);
        assert!(buf.cursor.anchor.is_none());
    }

    #[test]
    fn test_insert_text_replaces_selection() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello world\n");
        buf.cursor.anchor = Some((0, 0));
        buf.cursor.col = 5;
        buf.insert_text("goodbye");
        assert_eq!(buf.text.to_string(), "goodbye world\n");
        assert_eq!(buf.cursor.col, 7);
    }

    #[test]
    fn test_select_all() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("abc\ndef\n");
        buf.select_all();
        assert_eq!(buf.cursor.anchor, Some((0, 0)));
        assert_eq!(buf.selected_text(), "abc\ndef\n");
    }

    #[test]
    fn test_backspace_deletes_selection() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello world\n");
        buf.cursor.anchor = Some((0, 6));
        buf.cursor.col = 11;
        buf.backspace();
        assert_eq!(buf.text.to_string(), "hello \n");
    }

    #[test]
    fn test_typing_replaces_selection() {
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str("hello world\n");
        buf.cursor.anchor = Some((0, 0));
        buf.cursor.col = 11;
        buf.insert_char('X');
        assert_eq!(buf.text.to_string(), "X\n");
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_incremental_parse_json_number_edit() {
        // Regression: editing a number in JSON (e.g. "50" → "5") corrupted
        // nearby highlights because tree.edit() was not called before reparse.
        let json = r#"{"a": [50, 100, 200]}"#;
        let lang = gaviero_core::tree_sitter::language_for_extension("json").unwrap();
        let mut buf = Buffer::empty();
        buf.text = Rope::from_str(json);
        buf.parser = Some({
            let mut p = Parser::new();
            p.set_language(&lang).unwrap();
            p
        });
        buf.language = Some(lang);
        buf.reparse();
        assert!(buf.tree.is_some());

        // Place cursor after '0' in "50" and delete it → "5"
        buf.cursor.line = 0;
        buf.cursor.col = 9; // after the '0' in "50"
        buf.backspace();
        assert_eq!(buf.text.to_string(), r#"{"a": [5, 100, 200]}"#);

        // The tree should still be valid — all number nodes must have
        // correct byte ranges matching actual digit positions.
        let tree = buf.tree.as_ref().unwrap();
        let root = tree.root_node();
        assert!(!root.has_error(), "tree should have no errors after edit");

        // Verify each number node's byte ranges point to correct text
        let source = buf.text.to_string();
        let source_bytes = source.as_bytes();
        let mut numbers = Vec::new();
        collect_numbers(root, source_bytes, &mut numbers);
        assert_eq!(numbers, vec!["5", "100", "200"]);
    }
}

/// Collect text of all "number" nodes in the tree via DFS.
#[allow(dead_code)]
fn collect_numbers(
    node: gaviero_core::Node,
    source: &[u8],
    out: &mut Vec<String>,
) {
    if node.kind() == "number" {
        if let Ok(text) = node.utf8_text(source) {
            out.push(text.to_string());
        }
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_numbers(child, source, out);
        }
    }
}
