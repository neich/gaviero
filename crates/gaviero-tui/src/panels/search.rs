use std::path::{Path, PathBuf};

use crate::theme;
use crate::widgets::scroll_state::ScrollState;
use crate::widgets::text_input::TextInput;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};

/// A single search result: file path + line number + matching line text.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: PathBuf,
    pub line_number: usize,
    pub line_text: String,
}

/// State for the workspace search panel.
pub struct SearchPanelState {
    /// The interactive input field at the top of the panel.
    pub input: TextInput,
    /// The query that produced the current result set (may lag behind `input`
    /// while a debounce timer is pending).
    pub query: String,
    pub results: Vec<SearchResult>,
    pub scroll: ScrollState,
    pub searching: bool,
    /// When true the text input has keyboard focus; when false the results
    /// list does (arrow keys navigate results).
    pub editing: bool,
}

impl SearchPanelState {
    pub fn new() -> Self {
        Self {
            input: TextInput::new(),
            query: String::new(),
            results: Vec::new(),
            scroll: ScrollState::new(),
            searching: false,
            editing: true,
        }
    }

    /// Focus the input field (called when switching to the search panel).
    pub fn focus_input(&mut self) {
        self.editing = true;
    }

    /// Start a new search. Clears previous results.
    pub fn search(&mut self, query: &str, roots: &[&Path], excludes: &[String]) {
        self.query = query.to_string();
        self.results.clear();
        self.scroll.reset();
        self.searching = true;

        if !query.trim().is_empty() {
            for root in roots {
                self.search_dir(root, root, excludes);
            }
        }
        self.searching = false;
    }

    /// Run a search using the current input text. Called on every keystroke.
    pub fn search_from_input(&mut self, roots: &[&Path], excludes: &[String]) {
        let query = self.input.text.clone();
        self.search(&query, roots, excludes);
    }

    fn search_dir(&mut self, root: &Path, dir: &Path, excludes: &[String]) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

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
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };

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
        let sel_bg = if focused && !self.editing {
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

        // ── Input field (row 0) ─────────────────────────────────
        let input_y = area.y;
        let input_bg = if focused && self.editing {
            theme::INPUT_BG
        } else {
            bg
        };
        let prompt = " \u{1F50D} "; // 🔍 magnifying glass + space
        let prompt_style = Style::default().fg(theme::TEXT_DIM).bg(input_bg);

        // Clear input row
        for x in area.x..area.right() {
            if input_y < buf.area().bottom() {
                buf[(x, input_y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(input_bg));
            }
        }

        // Draw prompt
        let mut x = area.x;
        for ch in prompt.chars() {
            if x < area.right() && input_y < buf.area().bottom() {
                buf[(x, input_y)].set_char(ch).set_style(prompt_style);
                x += ch.len_utf8() as u16; // emoji takes 1 cell in our buffer but let's advance properly
            }
        }
        // The emoji is wide; use a simpler prompt for reliable column math
        let prompt_cols: u16 = 3; // " > "
        x = area.x;
        // Re-render with a simple text prompt for reliable positioning
        let prompt = " > ";
        for ch in prompt.chars() {
            if x < area.right() && input_y < buf.area().bottom() {
                buf[(x, input_y)].set_char(ch).set_style(prompt_style);
            }
            x += 1;
        }
        let text_x = area.x + prompt_cols;

        // Draw input text or placeholder
        if self.input.is_empty() && !(focused && self.editing) {
            let hint = "type to search...";
            let hint_style = Style::default().fg(theme::TEXT_DIM).bg(input_bg);
            let mut hx = text_x;
            for ch in hint.chars() {
                if hx >= area.right() {
                    break;
                }
                if input_y < buf.area().bottom() {
                    buf[(hx, input_y)].set_char(ch).set_style(hint_style);
                }
                hx += 1;
            }
        } else {
            let input_style = Style::default().fg(fg).bg(input_bg);
            let mut ix = text_x;
            for ch in self.input.text.chars() {
                if ix >= area.right() {
                    break;
                }
                if input_y < buf.area().bottom() {
                    buf[(ix, input_y)].set_char(ch).set_style(input_style);
                }
                ix += 1;
            }
        }

        // Cursor
        if focused && self.editing {
            let cursor_x = text_x + self.input.cursor as u16;
            if cursor_x < area.right() && input_y < buf.area().bottom() {
                let cursor_style = Style::default().fg(input_bg).bg(theme::TEXT_FG);
                buf[(cursor_x, input_y)].set_style(cursor_style);
            }
        }

        // ── Summary line (row 1) ────────────────────────────────
        let summary_y = area.y + 1;
        if summary_y < area.bottom() {
            let summary = if self.query.is_empty() {
                String::new()
            } else if self.results.is_empty() {
                format!(" No results for '{}'", self.query)
            } else {
                format!(" {} results", self.results.len())
            };
            let summary_style = Style::default()
                .fg(if self.results.is_empty() {
                    theme::TEXT_DIM
                } else {
                    theme::WARNING
                })
                .bg(bg);
            for (i, ch) in summary.chars().enumerate() {
                let sx = area.x + i as u16;
                if sx < area.right() && summary_y < buf.area().bottom() {
                    buf[(sx, summary_y)].set_char(ch).set_style(summary_style);
                }
            }
        }

        // ── Results list (row 2+) ───────────────────────────────
        let results_start = area.y + 2;
        let viewport = (area.height as usize).saturating_sub(2);
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
            let path_str = format!(" {}:{}", result.path.display(), result.line_number);
            let path_style = Style::default().fg(theme::FOCUS_BORDER).bg(line_bg);
            let text_style = Style::default().fg(fg).bg(line_bg);

            // Clear row
            for rx in area.x..area.right() {
                buf[(rx, y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(line_bg));
            }

            // Render path
            let mut rx = area.x;
            for ch in path_str.chars() {
                if rx < area.right() {
                    buf[(rx, y)].set_char(ch).set_style(path_style);
                    rx += 1;
                }
            }

            // Separator
            if rx + 2 < area.right() {
                buf[(rx, y)].set_char(' ').set_style(text_style);
                rx += 1;
            }

            // Render line text (truncated)
            for ch in result.line_text.chars() {
                if rx >= area.right() {
                    break;
                }
                buf[(rx, y)].set_char(ch).set_style(text_style);
                rx += 1;
            }
        }

        // Scrollbar
        if viewport > 0 {
            let scrollbar_area = Rect {
                x: area.x,
                y: results_start,
                width: area.width,
                height: viewport as u16,
            };
            crate::widgets::scrollbar::render_scrollbar(
                scrollbar_area,
                buf,
                self.results.len(),
                viewport,
                self.scroll.offset,
            );
        }
    }
}
