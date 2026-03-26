use std::path::{Path, PathBuf};

use crate::theme;
use crate::widgets::scroll_state::ScrollState;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

/// A single search result: file path + line number + matching line text.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: PathBuf,
    pub line_number: usize,
    pub line_text: String,
}

/// State for the workspace search panel.
pub struct SearchPanelState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub scroll: ScrollState,
    pub searching: bool,
}

impl SearchPanelState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            scroll: ScrollState::new(),
            searching: false,
        }
    }

    /// Start a new search. Clears previous results.
    pub fn search(&mut self, query: &str, roots: &[&Path], excludes: &[String]) {
        self.query = query.to_string();
        self.results.clear();
        self.scroll.reset();
        self.searching = true;

        // Search synchronously through workspace files
        for root in roots {
            self.search_dir(root, root, excludes);
        }
        self.searching = false;
    }

    fn search_dir(&mut self, root: &Path, dir: &Path, excludes: &[String]) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };

        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy();

            // Skip excluded patterns
            if excludes.iter().any(|ex| {
                if ex.ends_with('/') {
                    rel_str.starts_with(ex) || rel_str.starts_with(ex.trim_end_matches('/'))
                } else {
                    rel_str == *ex
                }
            }) {
                continue;
            }

            // Skip hidden dirs and common build dirs
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "target" || name_str == "node_modules" {
                continue;
            }

            if path.is_dir() {
                self.search_dir(root, &path, excludes);
            } else if path.is_file() {
                self.search_file(&path, rel);
            }
        }
    }

    fn search_file(&mut self, path: &Path, rel_path: &Path) {
        // Skip binary files (check first 512 bytes)
        let Ok(content) = std::fs::read_to_string(path) else { return };

        let query_lower = self.query.to_lowercase();
        for (i, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&query_lower) {
                self.results.push(SearchResult {
                    path: rel_path.to_path_buf(),
                    line_number: i + 1,
                    line_text: line.trim().to_string(),
                });
            }
        }
    }

    /// Get the selected result.
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.scroll.selected)
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let fg = theme::TEXT_FG;
        let sel_bg = if focused {
            theme::FOCUSED_SELECTION_BG
        } else {
            theme::DARK_BG
        };

        // Clear area
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        if self.results.is_empty() {
            let msg = if self.query.is_empty() {
                "No search query"
            } else if self.searching {
                "Searching..."
            } else {
                "No results"
            };
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            let y = area.y;
            for (i, ch) in msg.chars().enumerate() {
                let x = area.x + 1 + i as u16;
                if x < area.right() {
                    buf[(x, y)].set_char(ch).set_style(style);
                }
            }
            return;
        }

        // Header: "N results for 'query'"
        let header = format!(" {} results for '{}'", self.results.len(), self.query);
        let header_style = Style::default()
            .fg(theme::WARNING)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        for (i, ch) in header.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() && area.y < area.bottom() {
                buf[(x, area.y)].set_char(ch).set_style(header_style);
            }
        }

        // Results
        let results_start = area.y + 1;
        let viewport = (area.height as usize).saturating_sub(1);
        self.scroll.set_viewport(viewport);
        self.scroll.ensure_visible();

        for idx in self.scroll.visible_range(self.results.len(), viewport) {
            let row = idx - self.scroll.offset;
            let y = results_start + row as u16;
            if y >= area.bottom() {
                break;
            }

            let result = &self.results[idx];
            let is_selected = idx == self.scroll.selected;

            let line_bg = if is_selected { sel_bg } else { bg };

            // File path + line number
            let path_str = format!(
                " {}:{}",
                result.path.display(),
                result.line_number
            );
            let path_style = Style::default()
                .fg(theme::FOCUS_BORDER)
                .bg(line_bg);
            let text_style = Style::default().fg(fg).bg(line_bg);

            // Clear row
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(line_bg));
            }

            // Render path
            let mut x = area.x;
            for ch in path_str.chars() {
                if x < area.right() {
                    buf[(x, y)].set_char(ch).set_style(path_style);
                    x += 1;
                }
            }

            // Separator
            if x + 2 < area.right() {
                buf[(x, y)].set_char(' ').set_style(text_style);
                x += 1;
            }

            // Render line text (truncated)
            for ch in result.line_text.chars() {
                if x >= area.right() {
                    break;
                }
                buf[(x, y)].set_char(ch).set_style(text_style);
                x += 1;
            }
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(
            area,
            buf,
            self.results.len(),
            viewport,
            self.scroll.offset,
        );
    }
}
