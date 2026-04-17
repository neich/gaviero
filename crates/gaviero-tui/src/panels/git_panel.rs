//! Git panel — staging, committing, and branch management.

use ratatui::{
    buffer::Buffer as RataBuf,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use gaviero_core::git::{BranchEntry, FileStatus, FileStatusEntry, GitRepo};

use crate::theme;
use crate::theme::Theme;
use crate::widgets::text_input::TextInput;

// ── Types ──────────────────────────────────────────────────

/// Which region of the git panel has focus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GitRegion {
    Unstaged,
    Staged,
    CommitInput,
}

// ── State ──────────────────────────────────────────────────

pub struct GitPanelState {
    pub unstaged: Vec<FileStatusEntry>,
    pub staged: Vec<FileStatusEntry>,
    pub branches: Vec<BranchEntry>,
    pub current_branch: String,

    pub region: GitRegion,
    pub unstaged_selected: usize,
    pub staged_selected: usize,
    pub commit_input: TextInput,
    pub error_message: Option<String>,

    // Branch picker
    pub branch_picker_open: bool,
    pub branch_filter: String,
    pub branch_selected: usize,
}

impl GitPanelState {
    pub fn new() -> Self {
        Self {
            unstaged: Vec::new(),
            staged: Vec::new(),
            branches: Vec::new(),
            current_branch: String::new(),
            region: GitRegion::Unstaged,
            unstaged_selected: 0,
            staged_selected: 0,
            commit_input: TextInput::new(),
            error_message: None,
            branch_picker_open: false,
            branch_filter: String::new(),
            branch_selected: 0,
        }
    }

    /// Reload file status and branch info from the repository.
    pub fn refresh(&mut self, repo: &GitRepo) {
        self.error_message = None;

        match repo.file_status() {
            Ok(entries) => {
                self.unstaged = entries.iter().filter(|e| !e.staged).cloned().collect();
                self.staged = entries.iter().filter(|e| e.staged).cloned().collect();
            }
            Err(e) => {
                self.error_message = Some(format!("git status: {}", e));
                self.unstaged.clear();
                self.staged.clear();
            }
        }

        match repo.current_branch() {
            Ok(b) => self.current_branch = b,
            Err(_) => self.current_branch = "HEAD".to_string(),
        }

        if let Ok(branches) = repo.branches() {
            self.branches = branches;
        }

        // Clamp selections
        if self.unstaged_selected >= self.unstaged.len() {
            self.unstaged_selected = self.unstaged.len().saturating_sub(1);
        }
        if self.staged_selected >= self.staged.len() {
            self.staged_selected = self.staged.len().saturating_sub(1);
        }
    }

    /// Get the path of the currently selected file, if any.
    pub fn selected_path(&self) -> Option<&str> {
        match self.region {
            GitRegion::Unstaged => self
                .unstaged
                .get(self.unstaged_selected)
                .map(|e| e.path.as_str()),
            GitRegion::Staged => self
                .staged
                .get(self.staged_selected)
                .map(|e| e.path.as_str()),
            GitRegion::CommitInput => None,
        }
    }

    /// Cycle to the next region.
    pub fn cycle_region(&mut self) {
        self.region = match self.region {
            GitRegion::Unstaged => GitRegion::Staged,
            GitRegion::Staged => GitRegion::CommitInput,
            GitRegion::CommitInput => GitRegion::Unstaged,
        };
    }

    /// Move selection up in the current region.
    pub fn move_up(&mut self) {
        match self.region {
            GitRegion::Unstaged => {
                self.unstaged_selected = self.unstaged_selected.saturating_sub(1);
            }
            GitRegion::Staged => {
                self.staged_selected = self.staged_selected.saturating_sub(1);
            }
            GitRegion::CommitInput => {}
        }
    }

    /// Move selection down in the current region.
    pub fn move_down(&mut self) {
        match self.region {
            GitRegion::Unstaged => {
                if self.unstaged_selected + 1 < self.unstaged.len() {
                    self.unstaged_selected += 1;
                }
            }
            GitRegion::Staged => {
                if self.staged_selected + 1 < self.staged.len() {
                    self.staged_selected += 1;
                }
            }
            GitRegion::CommitInput => {}
        }
    }

    // ── Branch picker ───────────────────────────────────────

    /// Filtered branches matching the current filter.
    pub fn filtered_branches(&self) -> Vec<&BranchEntry> {
        let filter = self.branch_filter.to_lowercase();
        self.branches
            .iter()
            .filter(|b| !b.is_remote)
            .filter(|b| filter.is_empty() || b.name.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn toggle_branch_picker(&mut self) {
        self.branch_picker_open = !self.branch_picker_open;
        if self.branch_picker_open {
            self.branch_filter.clear();
            self.branch_selected = 0;
        }
    }

    pub fn close_branch_picker(&mut self) {
        self.branch_picker_open = false;
        self.branch_filter.clear();
    }

    pub fn branch_picker_up(&mut self) {
        self.branch_selected = self.branch_selected.saturating_sub(1);
    }

    pub fn branch_picker_down(&mut self) {
        let count = self.filtered_branches().len();
        if self.branch_selected + 1 < count {
            self.branch_selected += 1;
        }
    }

    pub fn branch_picker_insert(&mut self, ch: char) {
        self.branch_filter.push(ch);
        self.branch_selected = 0;
    }

    pub fn branch_picker_backspace(&mut self) {
        self.branch_filter.pop();
        self.branch_selected = 0;
    }

    /// Get the name of the selected branch in the picker.
    pub fn selected_branch_name(&self) -> Option<String> {
        self.filtered_branches()
            .get(self.branch_selected)
            .map(|b| b.name.clone())
    }

    // ── Rendering ──────────────────────────────────────────

    pub fn render(&self, area: Rect, buf: &mut RataBuf, focused: bool, _theme: &Theme) {
        if area.width < 4 || area.height < 6 {
            return;
        }

        let bg = theme::PANEL_BG;
        let fg = theme::TEXT_FG;
        let header_fg = theme::FOCUS_BORDER;
        let selected_bg = if focused {
            theme::BROWSE_BG
        } else {
            theme::INPUT_BG
        };
        let dim_fg = theme::TEXT_DIM;

        // Clear area
        let default_style = Style::default().fg(fg).bg(bg);
        for row in 0..area.height {
            for col in 0..area.width {
                let cx = area.x + col;
                let cy = area.y + row;
                if cx < buf.area().right() && cy < buf.area().bottom() {
                    buf[(cx, cy)].set_char(' ').set_style(default_style);
                }
            }
        }

        // Layout: branch line (1) + unstaged header (1) + unstaged files + staged header (1)
        //         + staged files + separator (1) + commit input (2)
        let commit_height: u16 = 3; // input + hint + separator
        let available = area.height.saturating_sub(commit_height + 1); // -1 for branch line
        let unstaged_files = (available / 2).max(2).saturating_sub(1); // -1 for header
        let staged_files = available.saturating_sub(unstaged_files + 2); // -2 for headers

        let mut y = area.y;

        // ── Branch line ──
        let branch_text = format!(" {} {}", '\u{E0A0}', self.current_branch); // git branch icon
        let branch_style = Style::default().fg(theme::CODE_GREEN).bg(bg);
        self.render_line(buf, area.x, y, area.width, &branch_text, branch_style);
        y += 1;

        // ── Unstaged Changes header ──
        let unstaged_header = format!(" Unstaged Changes ({})", self.unstaged.len());
        let h_style = if self.region == GitRegion::Unstaged {
            Style::default()
                .fg(header_fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(dim_fg).bg(bg)
        };
        self.render_line(buf, area.x, y, area.width, &unstaged_header, h_style);
        y += 1;

        // ── Unstaged files ──
        for i in 0..unstaged_files as usize {
            if y >= area.y + area.height {
                break;
            }
            if let Some(entry) = self.unstaged.get(i) {
                let is_sel = self.region == GitRegion::Unstaged && i == self.unstaged_selected;
                let marker = entry.status.marker();
                let marker_color = status_color(&entry.status);
                let line_bg = if is_sel { selected_bg } else { bg };

                // Clear line
                let clear = Style::default().fg(fg).bg(line_bg);
                self.render_line(buf, area.x, y, area.width, "", clear);

                // Marker
                let mx = area.x + 1;
                if mx < buf.area().right() && y < buf.area().bottom() {
                    buf[(mx, y)]
                        .set_char(marker)
                        .set_style(Style::default().fg(marker_color).bg(line_bg));
                }

                // Path
                let path_style = Style::default().fg(fg).bg(line_bg);
                for (ci, ch) in entry.path.chars().enumerate() {
                    let cx = area.x + 3 + ci as u16;
                    if cx < area.x + area.width
                        && cx < buf.area().right()
                        && y < buf.area().bottom()
                    {
                        buf[(cx, y)].set_char(ch).set_style(path_style);
                    }
                }
            }
            y += 1;
        }

        // ── Staged Changes header ──
        if y < area.y + area.height {
            let staged_header = format!(" Staged Changes ({})", self.staged.len());
            let h_style = if self.region == GitRegion::Staged {
                Style::default()
                    .fg(header_fg)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(dim_fg).bg(bg)
            };
            self.render_line(buf, area.x, y, area.width, &staged_header, h_style);
            y += 1;
        }

        // ── Staged files ──
        for i in 0..staged_files as usize {
            if y >= area.y + area.height {
                break;
            }
            if let Some(entry) = self.staged.get(i) {
                let is_sel = self.region == GitRegion::Staged && i == self.staged_selected;
                let marker = entry.status.marker();
                let marker_color = status_color(&entry.status);
                let line_bg = if is_sel { selected_bg } else { bg };

                let clear = Style::default().fg(fg).bg(line_bg);
                self.render_line(buf, area.x, y, area.width, "", clear);

                let mx = area.x + 1;
                if mx < buf.area().right() && y < buf.area().bottom() {
                    buf[(mx, y)]
                        .set_char(marker)
                        .set_style(Style::default().fg(marker_color).bg(line_bg));
                }

                let path_style = Style::default().fg(fg).bg(line_bg);
                for (ci, ch) in entry.path.chars().enumerate() {
                    let cx = area.x + 3 + ci as u16;
                    if cx < area.x + area.width
                        && cx < buf.area().right()
                        && y < buf.area().bottom()
                    {
                        buf[(cx, y)].set_char(ch).set_style(path_style);
                    }
                }
            }
            y += 1;
        }

        // ── Separator ──
        let sep_y = area.y + area.height - commit_height;
        if sep_y < buf.area().bottom() {
            let sep_style = Style::default().fg(theme::BORDER_DIM).bg(bg);
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() {
                    buf[(cx, sep_y)].set_char('─').set_style(sep_style);
                }
            }
        }

        // ── Commit input ──
        let input_y = sep_y + 1;
        if input_y < buf.area().bottom() {
            let input_bg = theme::INPUT_BG;
            let input_style = Style::default().fg(fg).bg(input_bg);
            let is_commit_focused = self.region == GitRegion::CommitInput;

            // Clear input lines
            for row in 0..2u16 {
                let iy = input_y + row;
                if iy >= buf.area().bottom() {
                    break;
                }
                for col in 0..area.width {
                    let cx = area.x + col;
                    if cx < buf.area().right() {
                        buf[(cx, iy)].set_char(' ').set_style(input_style);
                    }
                }
            }

            // Prompt
            let prompt = "> ";
            let prompt_style = Style::default().fg(header_fg).bg(input_bg);
            for (i, ch) in prompt.chars().enumerate() {
                let cx = area.x + i as u16;
                if cx < buf.area().right() && input_y < buf.area().bottom() {
                    buf[(cx, input_y)].set_char(ch).set_style(prompt_style);
                }
            }

            // Commit message text
            let text_x = area.x + prompt.len() as u16;
            if self.commit_input.text.is_empty() && !is_commit_focused {
                let hint = "commit message...";
                let hint_style = Style::default().fg(dim_fg).bg(input_bg);
                for (i, ch) in hint.chars().enumerate() {
                    let cx = text_x + i as u16;
                    if cx < area.x + area.width
                        && cx < buf.area().right()
                        && input_y < buf.area().bottom()
                    {
                        buf[(cx, input_y)].set_char(ch).set_style(hint_style);
                    }
                }
            } else {
                for (i, ch) in self.commit_input.text.chars().enumerate() {
                    let cx = text_x + i as u16;
                    if cx < area.x + area.width
                        && cx < buf.area().right()
                        && input_y < buf.area().bottom()
                    {
                        buf[(cx, input_y)].set_char(ch).set_style(input_style);
                    }
                }
            }

            // Cursor
            if focused && is_commit_focused {
                let cursor_char_pos = self.commit_input.cursor;
                let cursor_x = text_x + cursor_char_pos as u16;
                if cursor_x < area.x + area.width
                    && cursor_x < buf.area().right()
                    && input_y < buf.area().bottom()
                {
                    let cursor_style = Style::default().fg(input_bg).bg(theme::TEXT_FG);
                    buf[(cursor_x, input_y)].set_style(cursor_style);
                }
            }

            // Hint line
            let hint_y = input_y + 1;
            if hint_y < buf.area().bottom() {
                let hint = " [c]ommit  [a]mend  [s]tage  [u]nstage  [d]iscard";
                let hint_style = Style::default().fg(dim_fg).bg(input_bg);
                for (i, ch) in hint.chars().enumerate() {
                    let cx = area.x + i as u16;
                    if cx < area.x + area.width && cx < buf.area().right() {
                        buf[(cx, hint_y)].set_char(ch).set_style(hint_style);
                    }
                }
            }
        }

        // ── Error message overlay ──
        if let Some(ref err) = self.error_message {
            let err_y = area.y;
            let err_style = Style::default().fg(theme::PROPERTY_RED).bg(bg);
            let display = format!(" Error: {} ", err);
            for (i, ch) in display.chars().enumerate() {
                let cx = area.x + i as u16;
                if cx < area.x + area.width
                    && cx < buf.area().right()
                    && err_y < buf.area().bottom()
                {
                    buf[(cx, err_y)].set_char(ch).set_style(err_style);
                }
            }
        }

        // ── Branch picker overlay ──
        if self.branch_picker_open {
            let picker_bg = theme::INPUT_BG;
            let picker_fg = theme::TEXT_FG;
            let picker_sel_bg = theme::SELECTION_BG;
            let picker_header_fg = theme::FOCUS_BORDER;

            let filtered = self.filtered_branches();
            let max_rows = (area.height as usize)
                .saturating_sub(4)
                .min(filtered.len() + 2);
            let _picker_h = max_rows as u16 + 2; // +2 for header + filter
            let picker_y = area.y + 1;

            // Header
            if picker_y < buf.area().bottom() {
                let header = " Branches ";
                let h_style = Style::default()
                    .fg(picker_header_fg)
                    .bg(picker_bg)
                    .add_modifier(Modifier::BOLD);
                self.render_line(buf, area.x, picker_y, area.width, header, h_style);
            }

            // Filter input
            let filter_y = picker_y + 1;
            if filter_y < buf.area().bottom() {
                let filter_text = format!(" > {}", self.branch_filter);
                let f_style = Style::default().fg(picker_fg).bg(picker_bg);
                self.render_line(buf, area.x, filter_y, area.width, &filter_text, f_style);
            }

            // Branch list
            for (i, branch) in filtered.iter().enumerate().take(max_rows) {
                let by = filter_y + 1 + i as u16;
                if by >= buf.area().bottom() {
                    break;
                }

                let is_sel = i == self.branch_selected;
                let line_bg = if is_sel { picker_sel_bg } else { picker_bg };
                let marker = if branch.is_current { "* " } else { "  " };
                let text = format!(" {}{}", marker, branch.name);
                let style = Style::default().fg(picker_fg).bg(line_bg);
                self.render_line(buf, area.x, by, area.width, &text, style);
            }
        }
    }

    fn render_line(&self, buf: &mut RataBuf, x: u16, y: u16, width: u16, text: &str, style: Style) {
        for col in 0..width {
            let cx = x + col;
            if cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(' ').set_style(style);
            }
        }
        for (i, ch) in text.chars().enumerate() {
            let cx = x + i as u16;
            if cx < x + width && cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(ch).set_style(style);
            }
        }
    }
}

fn status_color(status: &FileStatus) -> Color {
    match status {
        FileStatus::Modified => theme::WARNING,     // yellow
        FileStatus::Added => theme::CODE_GREEN,     // green
        FileStatus::Deleted => theme::PROPERTY_RED, // red
        FileStatus::Untracked => theme::TEXT_DIM,   // dim
        FileStatus::Renamed => theme::INFO_CYAN,    // cyan
    }
}
