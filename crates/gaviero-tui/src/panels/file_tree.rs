use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::theme;
use crate::widgets::scroll_state::ScrollState;

#[derive(Debug)]
pub struct FileTreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    pub children_loaded: bool,
}

/// The parts of [`FileTreeState`] the user owns, snapshotted across a rebuild.
#[derive(Debug, Default)]
pub struct FileTreeView {
    /// Paths of the directories that were open.
    pub expanded: Vec<String>,
    /// Path under the cursor, if any — preferred over `selected_index`.
    pub selected_path: Option<PathBuf>,
    /// Cursor row, used only when `selected_path` is gone from the tree.
    pub selected_index: usize,
    /// First visible row.
    pub offset: usize,
    /// Paths marked with `s` for bulk operations.
    pub marked: HashSet<PathBuf>,
}

#[derive(Debug)]
pub struct FileTreeState {
    pub entries: Vec<FileTreeEntry>,
    pub scroll: ScrollState,
    pub exclude_patterns: Vec<String>,
    pub git_allow_list: Vec<String>,
    /// Paths currently marked by the user (via `s` key) for bulk operations.
    pub selected_paths: HashSet<PathBuf>,
}

impl FileTreeState {
    /// Build the file tree from workspace roots.
    pub fn from_roots(
        roots: &[&Path],
        exclude_patterns: &[String],
        git_allow_list: &[String],
    ) -> Self {
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
            scroll: ScrollState::new(),
            exclude_patterns: exclude_patterns.to_vec(),
            git_allow_list: git_allow_list.to_vec(),
            selected_paths: HashSet::new(),
        };

        // Load children for all root entries, last root first: `load_children`
        // inserts into `entries` right after the parent, so walking forwards
        // would push every later root past the index we were about to load —
        // in workspace mode that left roots 1..n expanded but empty, and
        // listed the children of whatever entry had slid into their slot.
        for i in (0..state.entries.len()).rev() {
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
        let is_git_dir = parent_path
            .file_name()
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
                        true
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
                Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
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

    /// Check if entry is allowed inside a `.git` directory.
    /// Only shows config-like files; controlled by `git.treeAllowList` setting.
    fn is_allowed_in_git(&self, name: &str) -> bool {
        self.git_allow_list.contains(&name.to_string())
    }

    /// Navigate down.
    pub fn move_down(&mut self) {
        self.scroll.move_down(self.entries.len());
    }

    /// Navigate up.
    pub fn move_up(&mut self) {
        self.scroll.move_up();
    }

    /// Toggle expand/collapse on the selected entry.
    pub fn toggle_expand(&mut self) {
        let idx = self.scroll.selected;
        if idx >= self.entries.len() {
            tracing::debug!(
                "toggle_expand: idx {} out of range ({})",
                idx,
                self.entries.len()
            );
            return;
        }

        let entry = &self.entries[idx];
        tracing::debug!(
            "toggle_expand: idx={}, name={}, is_dir={}, expanded={}, children_loaded={}",
            idx,
            entry.name,
            entry.is_dir,
            entry.expanded,
            entry.children_loaded
        );

        if self.entries[idx].is_dir {
            if self.entries[idx].expanded {
                self.collapse(idx);
                tracing::debug!(
                    "toggle_expand: collapsed, entries now: {}",
                    self.entries.len()
                );
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
        self.entries
            .get(self.scroll.selected)
            .map(|e| e.path.as_path())
    }

    /// Is the selected entry a file?
    pub fn selected_is_file(&self) -> bool {
        self.entries
            .get(self.scroll.selected)
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
        let idx = self.scroll.offset + row;
        self.scroll.select(idx, self.entries.len());
    }

    /// Scroll up by n entries.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll.scroll_up(n);
    }

    /// Scroll down by n entries.
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll.scroll_down(n, self.entries.len());
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
            // Roots start out expanded, so match on "should be open" rather
            // than "is closed" — an already-expanded entry whose children were
            // never listed still needs the read_dir.
            let entry = &self.entries[idx];
            let should_expand = entry.is_dir
                && (entry.expanded || set.contains(entry.path.to_string_lossy().as_ref()));
            if should_expand {
                self.entries[idx].expanded = true;
                if !self.entries[idx].children_loaded {
                    self.load_children(idx);
                }
            }
            idx += 1;
        }
    }

    /// Capture the user's view of the tree so it can survive a rebuild.
    ///
    /// The tree is rebuilt from disk on every filesystem event
    /// (`Event::FileTreeChanged`), which can fire while the user is looking at
    /// another window. Everything they set by hand — which folders are open,
    /// where the cursor is, how far they scrolled, what they marked — is
    /// carried across so the refresh is invisible.
    pub fn view_state(&self) -> FileTreeView {
        FileTreeView {
            expanded: self.expanded_paths(),
            selected_path: self.selected_path().map(|p| p.to_path_buf()),
            selected_index: self.scroll.selected,
            offset: self.scroll.offset,
            marked: self.selected_paths.clone(),
        }
    }

    /// Re-apply a [`FileTreeView`] captured before the rebuild.
    ///
    /// The cursor follows its path rather than its index — entries created or
    /// deleted above it would otherwise shift the selection.
    pub fn restore_view(&mut self, view: FileTreeView) {
        self.restore_expanded(&view.expanded);
        self.selected_paths = view.marked;

        let selected = view
            .selected_path
            .and_then(|path| self.entries.iter().position(|e| e.path == path))
            .unwrap_or(view.selected_index);
        self.scroll
            .restore_position(selected, view.offset, self.entries.len());
    }

    /// Toggle the selection mark on the currently highlighted entry.
    pub fn toggle_selection(&mut self) {
        let Some(entry) = self.entries.get(self.scroll.selected) else {
            return;
        };
        let path = entry.path.clone();
        if !self.selected_paths.remove(&path) {
            self.selected_paths.insert(path);
        }
    }

    /// Clear all selection marks.
    pub fn clear_selection(&mut self) {
        self.selected_paths.clear();
    }

    /// True when at least one path is selected.
    pub fn has_selection(&self) -> bool {
        !self.selected_paths.is_empty()
    }

    /// Return selected paths sorted for deterministic operation order.
    pub fn selected_paths_sorted(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.selected_paths.iter().cloned().collect();
        v.sort();
        v
    }

    /// Render the file tree into the given area.
    /// NOTE: takes &mut self to auto-scroll the selection into view.
    /// `move_source` highlights the file being moved (SelectingDest / Confirming states).
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        move_source: Option<&std::path::Path>,
    ) {
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

        // Auto-scroll only when the selection has been changed since the last
        // render. Wheel scrolling moves the offset independently, so the
        // selected item may scroll off-screen — that's intentional.
        let viewport = inner.height as usize;
        self.scroll.set_viewport(viewport);
        self.scroll.ensure_visible_on_render();

        let visible_entries = self
            .entries
            .iter()
            .enumerate()
            .skip(self.scroll.offset)
            .take(viewport);

        for (row, (i, entry)) in visible_entries.enumerate() {
            let y = inner.y + row as u16;
            let indent = " ".repeat(entry.depth);
            let icon = if entry.is_dir {
                if entry.expanded { "▾" } else { "▸" }
            } else {
                " "
            };

            let is_cursor = i == self.scroll.selected;
            let is_marked = self.selected_paths.contains(&entry.path);
            let is_move_source = move_source.map(|s| s == entry.path).unwrap_or(false);
            let style = if is_cursor {
                Style::default()
                    .fg(theme::SELECTED_BRIGHT)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme::SELECTION_BG)
            } else if is_move_source {
                Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme::INPUT_BG)
            } else if is_marked {
                Style::default()
                    .fg(theme::NUMERIC_ORANGE)
                    .add_modifier(Modifier::BOLD)
            } else if entry.is_dir {
                Style::default().fg(theme::FOCUS_BORDER)
            } else {
                Style::default().fg(theme::TEXT_FG)
            };

            let mark = if is_marked { "●" } else { " " };
            let text = format!("{}{}{}{}", mark, indent, icon, entry.name);
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
            viewport,
            self.scroll.offset,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `root/{a/, b/, f.txt}` — two sub-dirs keep `compact_single_child` from
    /// merging the root into its only child.
    fn make_root(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("a").join("deep.txt"), "x").unwrap();
        std::fs::write(root.join("a").join("deep2.txt"), "x").unwrap();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        root
    }

    fn names_under(state: &FileTreeState, root: &Path) -> Vec<String> {
        state
            .entries
            .iter()
            .filter(|e| e.path.starts_with(root) && e.path != root)
            .map(|e| e.name.clone())
            .collect()
    }

    #[test]
    fn every_root_loads_its_children() {
        // Regression: `from_roots` used to call `load_children` with the
        // pre-insertion index, so in workspace (multi-root) mode only root 0
        // got its children — every later root rendered expanded but empty.
        let tmp = tempfile::tempdir().unwrap();
        let first = make_root(tmp.path(), "first");
        let second = make_root(tmp.path(), "second");
        let third = make_root(tmp.path(), "third");

        let roots: Vec<&Path> = vec![&first, &second, &third];
        let state = FileTreeState::from_roots(&roots, &[], &[]);

        for root in [&first, &second, &third] {
            let children = names_under(&state, root);
            assert!(
                children.contains(&"a".to_string()) && children.contains(&"f.txt".to_string()),
                "root {} has no children in the tree: {:?}",
                root.display(),
                children
            );
        }
    }

    #[test]
    fn collapsed_dirs_stay_collapsed_on_build() {
        // Same root cause seen from the other side: mis-indexed loads spilled
        // a collapsed directory's children into the flat list.
        let tmp = tempfile::tempdir().unwrap();
        let first = make_root(tmp.path(), "first");
        let second = make_root(tmp.path(), "second");

        let roots: Vec<&Path> = vec![&first, &second];
        let state = FileTreeState::from_roots(&roots, &[], &[]);

        assert!(
            !state.entries.iter().any(|e| e.name == "deep.txt"),
            "children of a collapsed directory must not be listed"
        );
    }

    #[test]
    fn rebuild_restores_expansion_in_every_root() {
        let tmp = tempfile::tempdir().unwrap();
        let first = make_root(tmp.path(), "first");
        let second = make_root(tmp.path(), "second");
        let roots: Vec<&Path> = vec![&first, &second];

        let mut state = FileTreeState::from_roots(&roots, &[], &[]);
        // Expand `second/a` the way the user would.
        let idx = state
            .entries
            .iter()
            .position(|e| e.path == second.join("a"))
            .expect("second/a listed");
        state.scroll.selected = idx;
        state.toggle_expand();
        assert!(state.entries.iter().any(|e| e.name == "deep.txt"));

        // Filesystem event → rebuild from disk.
        let view = state.view_state();
        let mut rebuilt = FileTreeState::from_roots(&roots, &[], &[]);
        rebuilt.restore_view(view);

        assert!(
            rebuilt.entries.iter().any(|e| e.name == "deep.txt"),
            "expansion inside a non-first root must survive a rebuild"
        );
    }

    #[test]
    fn rebuild_keeps_scroll_marks_and_cursor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_root(tmp.path(), "only");
        let roots: Vec<&Path> = vec![&root];

        let mut state = FileTreeState::from_roots(&roots, &[], &[]);
        let cursor = state
            .entries
            .iter()
            .position(|e| e.path == root.join("f.txt"))
            .expect("f.txt listed");
        state.scroll.selected = cursor;
        state.toggle_selection();
        state.scroll.offset = 2;

        let view = state.view_state();
        let mut rebuilt = FileTreeState::from_roots(&roots, &[], &[]);
        rebuilt.restore_view(view);

        assert_eq!(
            rebuilt.selected_path().map(|p| p.to_path_buf()),
            Some(root.join("f.txt")),
            "cursor should follow the path, not the index"
        );
        assert_eq!(rebuilt.scroll.offset, 2, "scroll offset should survive");
        assert!(
            rebuilt.selected_paths.contains(&root.join("f.txt")),
            "marked paths should survive"
        );
    }
}
