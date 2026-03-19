use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use std::path::{Path, PathBuf};

use crate::theme;

#[derive(Debug)]
pub struct FileTreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    pub children_loaded: bool,
}

#[derive(Debug)]
pub struct FileTreeState {
    pub entries: Vec<FileTreeEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub exclude_patterns: Vec<String>,
    pub git_allow_list: Vec<String>,
}

impl FileTreeState {
    /// Build the file tree from workspace roots.
    pub fn from_roots(roots: &[&Path], exclude_patterns: &[String], git_allow_list: &[String]) -> Self {
        let mut entries = Vec::new();

        for root in roots {
            let name = root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            entries.push(FileTreeEntry {
                path: root.to_path_buf(),
                name,
                is_dir: true,
                depth: 0,
                expanded: true,
                children_loaded: false,
            });
        }

        let mut state = Self {
            entries,
            selected: 0,
            scroll_offset: 0,
            exclude_patterns: exclude_patterns.to_vec(),
            git_allow_list: git_allow_list.to_vec(),
        };

        // Load children for all root entries
        let root_count = state.entries.len();
        for i in 0..root_count {
            state.load_children(i);
        }

        state
    }

    /// Load children of a directory entry.
    fn load_children(&mut self, index: usize) {
        if !self.entries[index].is_dir || self.entries[index].children_loaded {
            return;
        }

        self.entries[index].children_loaded = true;
        let parent_path = self.entries[index].path.clone();
        let depth = self.entries[index].depth + 1;

        // Check if this is a .git directory (apply allowlist instead of denylist)
        let is_git_dir = parent_path.file_name()
            .map(|n| n == ".git")
            .unwrap_or(false);

        let mut children: Vec<FileTreeEntry> = match std::fs::read_dir(&parent_path) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if is_git_dir && !self.git_allow_list.is_empty() {
                        self.is_allowed_in_git(&name)
                    } else {
                        !self.is_excluded(&name)
                    }
                })
                .map(|e| {
                    let path = e.path();
                    let is_dir = path.is_dir();
                    let name = e.file_name().to_string_lossy().to_string();
                    FileTreeEntry {
                        path,
                        name,
                        is_dir,
                        depth,
                        expanded: false,
                        children_loaded: false,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        // Sort: directories first, then alphabetically
        children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        // Compact single-child directory chains (like VS Code)
        for child in &mut children {
            if child.is_dir {
                self.compact_single_child(child);
            }
        }

        // Insert children after the parent
        let insert_pos = index + 1;
        for (i, child) in children.into_iter().enumerate() {
            self.entries.insert(insert_pos + i, child);
        }
    }

    /// Compact a directory entry if it has exactly one child that is also a directory.
    /// Merges names like "src/editor" into a single entry. Max 10 levels to avoid runaway.
    fn compact_single_child(&self, entry: &mut FileTreeEntry) {
        for _ in 0..10 {
            let sub_entries: Vec<_> = match std::fs::read_dir(&entry.path) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        !self.is_excluded(&name)
                    })
                    .collect(),
                Err(_) => break,
            };
            if sub_entries.len() == 1 && sub_entries[0].path().is_dir() {
                let child_name = sub_entries[0].file_name().to_string_lossy().to_string();
                entry.name = format!("{}/{}", entry.name, child_name);
                entry.path = sub_entries[0].path();
            } else {
                break;
            }
        }
    }

    /// Check if a filename matches any exclude pattern.
    /// All patterns come from user settings — no hardcoded defaults.
    fn is_excluded(&self, name: &str) -> bool {
        for pattern in &self.exclude_patterns {
            let pat = pattern.trim_start_matches("**/");
            if name == pat {
                return true;
            }
            if name.starts_with('.') && pattern == ".*" {
                return true;
            }
        }
        false
    }

    /// Check if entry is allowed inside a `.git` directory.
    /// Only shows config-like files; controlled by `git.treeAllowList` setting.
    fn is_allowed_in_git(&self, name: &str) -> bool {
        self.git_allow_list.contains(&name.to_string())
    }

    /// Navigate down.
    pub fn move_down(&mut self) {
        if self.selected < self.visible_count().saturating_sub(1) {
            self.selected += 1;
        }
    }

    /// Navigate up.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Toggle expand/collapse on the selected entry.
    pub fn toggle_expand(&mut self) {
        let idx = self.selected;
        if idx >= self.entries.len() {
            tracing::debug!("toggle_expand: idx {} out of range ({})", idx, self.entries.len());
            return;
        }

        let entry = &self.entries[idx];
        tracing::debug!(
            "toggle_expand: idx={}, name={}, is_dir={}, expanded={}, children_loaded={}",
            idx, entry.name, entry.is_dir, entry.expanded, entry.children_loaded
        );

        if self.entries[idx].is_dir {
            if self.entries[idx].expanded {
                self.collapse(idx);
                tracing::debug!("toggle_expand: collapsed, entries now: {}", self.entries.len());
            } else {
                self.entries[idx].expanded = true;
                if !self.entries[idx].children_loaded {
                    let before = self.entries.len();
                    self.load_children(idx);
                    tracing::debug!(
                        "toggle_expand: expanded, loaded {} children",
                        self.entries.len() - before
                    );
                }
            }
        }
    }

    /// Get the path of the selected entry (for opening files).
    pub fn selected_path(&self) -> Option<&Path> {
        self.entries.get(self.selected).map(|e| e.path.as_path())
    }

    /// Is the selected entry a file?
    pub fn selected_is_file(&self) -> bool {
        self.entries
            .get(self.selected)
            .map(|e| !e.is_dir)
            .unwrap_or(false)
    }

    /// Collapse a directory entry (remove its children from the flat list).
    fn collapse(&mut self, idx: usize) {
        self.entries[idx].expanded = false;
        let depth = self.entries[idx].depth;

        // Remove all entries with greater depth following this one
        let mut remove_end = idx + 1;
        while remove_end < self.entries.len() && self.entries[remove_end].depth > depth {
            remove_end += 1;
        }
        self.entries.drain(idx + 1..remove_end);
        self.entries[idx].children_loaded = false;
    }

    /// Click on a row (relative to the panel top, accounting for scroll).
    pub fn click_row(&mut self, row: usize) {
        // Account for block border (1 row for top border if present, but we use RIGHT border only)
        let idx = self.scroll_offset + row;
        if idx < self.entries.len() {
            self.selected = idx;
        }
    }

    /// Scroll up by n entries.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll down by n entries.
    pub fn scroll_down(&mut self, n: usize) {
        let max = self.entries.len().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    /// Ensure the selected entry is visible in the viewport.
    fn ensure_selected_visible(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
        if self.selected >= self.scroll_offset + viewport_height {
            self.scroll_offset = self.selected - viewport_height + 1;
        }
    }

    /// Return paths of all currently expanded directories (for state persistence).
    pub fn expanded_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.is_dir && e.expanded)
            .map(|e| e.path.to_string_lossy().to_string())
            .collect()
    }

    /// Restore expanded state from saved paths.
    /// Expands directories matching the given paths.
    pub fn restore_expanded(&mut self, paths: &[String]) {
        use std::collections::HashSet;
        let set: HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();

        let mut idx = 0;
        while idx < self.entries.len() {
            if self.entries[idx].is_dir
                && !self.entries[idx].expanded
                && set.contains(self.entries[idx].path.to_string_lossy().as_ref())
            {
                self.entries[idx].expanded = true;
                if !self.entries[idx].children_loaded {
                    self.load_children(idx);
                }
            }
            idx += 1;
        }
    }

    fn visible_count(&self) -> usize {
        self.entries.len()
    }

    /// Render the file tree into the given area.
    /// NOTE: takes &mut self to auto-scroll the selection into view.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let border_style = if focused {
            Style::default().fg(theme::FOCUS_BORDER)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, buf);

        // Auto-scroll to keep selection visible
        self.ensure_selected_visible(inner.height as usize);

        let visible_entries = self.entries.iter().enumerate()
            .skip(self.scroll_offset)
            .take(inner.height as usize);

        for (row, (i, entry)) in visible_entries.enumerate() {
            let y = inner.y + row as u16;
            let indent = " ".repeat(entry.depth);
            let icon = if entry.is_dir {
                if entry.expanded { "▾" } else { "▸" }
            } else {
                " "
            };

            let is_selected = i == self.selected;
            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme::SELECTION_BG)
            } else if entry.is_dir {
                Style::default().fg(theme::FOCUS_BORDER)
            } else {
                Style::default().fg(theme::TEXT_FG)
            };

            let text = format!("{}{}{}", indent, icon, entry.name);
            let line = Line::from(Span::styled(text, style));

            let line_area = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };
            Widget::render(line, line_area, buf);
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(
            inner,
            buf,
            self.entries.len(),
            inner.height as usize,
            self.scroll_offset,
        );
    }
}
