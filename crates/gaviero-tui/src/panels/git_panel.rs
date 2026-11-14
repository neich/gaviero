//! Git panel — staging, committing, and branch management.
//!
//! Every repository the workspace covers is listed at once, one block per
//! repository: a title line (its folder, and its branch on the right), then
//! its unstaged and staged files. The selected row — a title or a file —
//! decides which repository commit, amend, the branch picker and
//! stage / unstage / discard act on; the commit box names it.

use std::cell::Cell;

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

/// Which part of the git panel has the keyboard: the file list (the
/// selection's region — `Unstaged` on a repository title) or the commit box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRegion {
    Unstaged,
    Staged,
    CommitInput,
}

/// One repository's block.
#[derive(Debug, Clone, Default)]
pub struct RepoSection {
    /// The title: the repository's folder name (see `scan_git_repos`).
    pub name: String,
    pub branch: String,
    pub branches: Vec<BranchEntry>,
    pub unstaged: Vec<FileStatusEntry>,
    pub staged: Vec<FileStatusEntry>,
    /// `git status` failed for this repository.
    pub error: Option<String>,
}

impl RepoSection {
    fn files(&self, region: GitRegion) -> &[FileStatusEntry] {
        match region {
            GitRegion::Unstaged => &self.unstaged,
            GitRegion::Staged => &self.staged,
            GitRegion::CommitInput => &[],
        }
    }
}

/// One line of the file list, in drawing order (see [`GitPanelState::rows`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRow {
    /// A repository's title line. Selectable.
    Repo(usize),
    /// "Unstaged (n)" / "Staged (n)" under a repository.
    Header(usize, GitRegion),
    /// A changed file: repository, region, index in that region. Selectable.
    File(usize, GitRegion, usize),
    /// "no changes", or the repository's status error.
    Note(usize),
}

impl GitRow {
    pub fn repo(&self) -> usize {
        match *self {
            GitRow::Repo(r) | GitRow::Header(r, _) | GitRow::File(r, _, _) | GitRow::Note(r) => r,
        }
    }

    fn selectable(&self) -> bool {
        matches!(self, GitRow::Repo(_) | GitRow::File(..))
    }
}

/// Rows below the file list: separator, commit input, key hints.
const COMMIT_HEIGHT: u16 = 3;
/// Longest repository name the commit prompt shows before truncating.
const PROMPT_NAME_MAX: usize = 24;

// ── State ──────────────────────────────────────────────────

pub struct GitPanelState {
    /// One per repository, in `App::git_repos` order.
    pub sections: Vec<RepoSection>,
    /// The selected row: a repository title or a file, never a header.
    pub selection: GitRow,
    pub region: GitRegion,
    pub commit_input: TextInput,
    pub error_message: Option<String>,

    // Branch picker
    pub branch_picker_open: bool,
    pub branch_filter: String,
    pub branch_selected: usize,

    /// The selection's repository (an index into the host's `git_repos`):
    /// the one every git action applies to.
    pub active_repo: usize,

    /// First list row drawn. View state only: `render` moves it to keep the
    /// selection visible, and mouse hit-testing reads it back.
    scroll: Cell<usize>,
}

impl GitPanelState {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
            selection: GitRow::Repo(0),
            region: GitRegion::Unstaged,
            commit_input: TextInput::new(),
            error_message: None,
            branch_picker_open: false,
            branch_filter: String::new(),
            branch_selected: 0,
            active_repo: 0,
            scroll: Cell::new(0),
        }
    }

    /// Match the sections to the host's repository list (titles in order).
    /// Sections keep their file lists until [`Self::refresh_repo`] reloads
    /// them; the selection is clamped to what still exists.
    pub fn set_repos(&mut self, names: Vec<String>) {
        self.sections.resize_with(names.len(), RepoSection::default);
        for (section, name) in self.sections.iter_mut().zip(names) {
            section.name = name;
        }
        self.error_message = None;
        self.fix_selection();
    }

    /// Move the selection to repository `idx`'s title (Alt+O / Alt+I step
    /// through repositories). No-op when there are fewer than two.
    pub fn cycle_repo(&mut self, delta: i32) {
        let n = self.sections.len();
        if n < 2 {
            return;
        }
        let next = (self.active_repo as i32 + delta).rem_euclid(n as i32) as usize;
        self.select(GitRow::Repo(next));
        self.error_message = None;
        self.close_branch_picker();
    }

    /// Keep the selection on the same repository after the host's list was
    /// rebuilt and that repository moved to `idx`.
    pub fn retarget_repo(&mut self, idx: usize) {
        self.selection = match self.selection {
            GitRow::File(_, region, i) => GitRow::File(idx, region, i),
            _ => GitRow::Repo(idx),
        };
        self.active_repo = idx;
    }

    /// Clear the panel when no folder holds or contains a repository.
    pub fn show_no_repository(&mut self) {
        self.sections.clear();
        self.selection = GitRow::Repo(0);
        self.active_repo = 0;
        self.error_message =
            Some("no git repository in this folder or its direct subfolders".to_string());
    }

    /// Reload the selected repository's files and branches (after an action
    /// on it).
    pub fn refresh(&mut self, repo: &GitRepo) {
        self.refresh_repo(self.active_repo, repo);
    }

    /// Reload repository `idx`'s files and branches.
    pub fn refresh_repo(&mut self, idx: usize, repo: &GitRepo) {
        let Some(section) = self.sections.get_mut(idx) else {
            return;
        };
        match repo.file_status() {
            Ok(entries) => {
                section.unstaged = entries.iter().filter(|e| !e.staged).cloned().collect();
                section.staged = entries.iter().filter(|e| e.staged).cloned().collect();
                section.error = None;
            }
            Err(e) => {
                section.unstaged.clear();
                section.staged.clear();
                section.error = Some(format!("git status: {e}"));
            }
        }
        section.branch = repo.current_branch().unwrap_or_else(|_| "HEAD".to_string());
        if let Ok(branches) = repo.branches() {
            section.branches = branches;
        }
        self.fix_selection();
    }

    /// Every list line, in drawing order: per repository its title, then
    /// "Unstaged (n)" and its files, "Staged (n)" and its files (an empty
    /// region is left out), or one note when it has nothing to show.
    pub fn rows(&self) -> Vec<GitRow> {
        let mut rows = Vec::new();
        for (r, section) in self.sections.iter().enumerate() {
            rows.push(GitRow::Repo(r));
            if section.error.is_some() || (section.unstaged.is_empty() && section.staged.is_empty())
            {
                rows.push(GitRow::Note(r));
                continue;
            }
            for region in [GitRegion::Unstaged, GitRegion::Staged] {
                let files = section.files(region);
                if files.is_empty() {
                    continue;
                }
                rows.push(GitRow::Header(r, region));
                rows.extend((0..files.len()).map(|i| GitRow::File(r, region, i)));
            }
        }
        rows
    }

    /// Select a repository title or a file row; the commit box gives up the
    /// keyboard to the list.
    pub fn select(&mut self, row: GitRow) {
        self.selection = row;
        self.active_repo = row.repo();
        self.region = match row {
            GitRow::File(_, region, _) => region,
            _ => GitRegion::Unstaged,
        };
    }

    /// Put the selection back on a row that exists after the file lists or
    /// the repository list changed: the same index in the same region when
    /// it is still there (the next file, after staging one), else the first
    /// file of the other region, else the repository's title.
    fn fix_selection(&mut self) {
        let in_commit = self.region == GitRegion::CommitInput;
        let n = self.sections.len();
        let row = match self.selection {
            _ if n == 0 => GitRow::Repo(0),
            GitRow::File(r, region, i) if r < n => {
                let section = &self.sections[r];
                let other = match region {
                    GitRegion::Staged => GitRegion::Unstaged,
                    _ => GitRegion::Staged,
                };
                let here = section.files(region).len();
                if section.error.is_some() {
                    GitRow::Repo(r)
                } else if here > 0 {
                    GitRow::File(r, region, i.min(here - 1))
                } else if !section.files(other).is_empty() {
                    GitRow::File(r, other, 0)
                } else {
                    GitRow::Repo(r)
                }
            }
            row => GitRow::Repo(row.repo().min(n - 1)),
        };
        self.select(row);
        if in_commit {
            self.region = GitRegion::CommitInput;
        }
    }

    /// Map a row offset within the panel area (and that area's height) to
    /// the selectable list row drawn there: a repository title or a file.
    /// Mirrors the layout in `render`, scroll included.
    pub fn hit_test_file(&self, rel_y: u16, area_height: u16) -> Option<GitRow> {
        if area_height < 6 || rel_y >= area_height.saturating_sub(COMMIT_HEIGHT) {
            return None;
        }
        let rows = self.rows();
        rows.get(self.scroll.get() + rel_y as usize)
            .copied()
            .filter(GitRow::selectable)
    }

    /// The repository-relative path of the selected file, if a file is
    /// selected and the list has the keyboard.
    pub fn selected_path(&self) -> Option<&str> {
        if self.region == GitRegion::CommitInput {
            return None;
        }
        match self.selection {
            GitRow::File(r, region, i) => self
                .sections
                .get(r)?
                .files(region)
                .get(i)
                .map(|e| e.path.as_str()),
            _ => None,
        }
    }

    /// The selected repository's title, when there is one.
    pub fn active_name(&self) -> Option<&str> {
        self.sections.get(self.active_repo).map(|s| s.name.as_str())
    }

    /// Tab: between the file list and the commit box.
    pub fn cycle_region(&mut self) {
        if self.region == GitRegion::CommitInput {
            let row = self.selection;
            self.select(row);
        } else {
            self.region = GitRegion::CommitInput;
        }
    }

    /// Move the selection to the previous title or file, across repositories.
    pub fn move_up(&mut self) {
        self.step(-1);
    }

    /// Move the selection to the next title or file, across repositories.
    pub fn move_down(&mut self) {
        self.step(1);
    }

    fn step(&mut self, delta: isize) {
        if self.region == GitRegion::CommitInput {
            return;
        }
        let rows = self.rows();
        let Some(pos) = rows.iter().position(|r| *r == self.selection) else {
            if let Some(first) = rows.first() {
                self.select(*first);
            }
            return;
        };
        let mut i = pos as isize + delta;
        while i >= 0 && (i as usize) < rows.len() {
            if rows[i as usize].selectable() {
                self.select(rows[i as usize]);
                return;
            }
            i += delta;
        }
    }

    // ── Branch picker ───────────────────────────────────────

    /// The selected repository's local branches matching the filter.
    pub fn filtered_branches(&self) -> Vec<&BranchEntry> {
        let filter = self.branch_filter.to_lowercase();
        let Some(section) = self.sections.get(self.active_repo) else {
            return Vec::new();
        };
        section
            .branches
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
            self.render_line(buf, area.x, area.y + row, area.width, "", default_style);
        }

        // ── File list, scrolled to keep the selection in view ──
        let list_height = area.height.saturating_sub(COMMIT_HEIGHT) as usize;
        let rows = self.rows();
        let mut top = self.scroll.get();
        if let Some(sel) = rows.iter().position(|r| *r == self.selection) {
            if sel < top {
                top = sel;
            } else if sel >= top + list_height {
                top = sel + 1 - list_height;
            }
        }
        top = top.min(rows.len().saturating_sub(list_height));
        self.scroll.set(top);

        let list_has_keys = self.region != GitRegion::CommitInput;
        for (offset, row) in rows.iter().skip(top).take(list_height).enumerate() {
            let y = area.y + offset as u16;
            let is_sel = list_has_keys && *row == self.selection;
            match *row {
                GitRow::Repo(r) => {
                    let section = &self.sections[r];
                    let line_bg = if is_sel { selected_bg } else { theme::TAB_BG };
                    let title_style = Style::default()
                        .fg(theme::TEXT_BRIGHT)
                        .bg(line_bg)
                        .add_modifier(Modifier::BOLD);
                    self.render_line(buf, area.x, y, area.width, &format!(" {}", section.name), title_style);
                    // Branch, right-aligned, when it fits beside the title.
                    let branch = format!(" {} {} ", '\u{E0A0}', section.branch);
                    let branch_w = branch.chars().count() as u16;
                    let title_w = section.name.chars().count() as u16 + 2;
                    if !section.branch.is_empty() && title_w + branch_w <= area.width {
                        let branch_style = Style::default().fg(theme::CODE_GREEN).bg(line_bg);
                        self.put_text(buf, area.x + area.width - branch_w, y, area.x + area.width, &branch, branch_style);
                    }
                }
                GitRow::Header(r, region) => {
                    let section = &self.sections[r];
                    let label = match region {
                        GitRegion::Staged => "Staged",
                        _ => "Unstaged",
                    };
                    let text = format!("   {label} ({})", section.files(region).len());
                    self.render_line(buf, area.x, y, area.width, &text, Style::default().fg(dim_fg).bg(bg));
                }
                GitRow::File(r, region, i) => {
                    let Some(entry) = self.sections[r].files(region).get(i) else {
                        continue;
                    };
                    let line_bg = if is_sel { selected_bg } else { bg };
                    self.render_line(buf, area.x, y, area.width, "", Style::default().fg(fg).bg(line_bg));
                    let marker_style = Style::default().fg(status_color(&entry.status)).bg(line_bg);
                    self.put_text(buf, area.x + 4, y, area.x + area.width, &entry.status.marker().to_string(), marker_style);
                    self.put_text(buf, area.x + 6, y, area.x + area.width, &entry.path, Style::default().fg(fg).bg(line_bg));
                }
                GitRow::Note(r) => {
                    let (text, style) = match &self.sections[r].error {
                        Some(e) => (format!("    {e}"), Style::default().fg(theme::PROPERTY_RED).bg(bg)),
                        None => ("    no changes".to_string(), Style::default().fg(dim_fg).bg(bg)),
                    };
                    self.render_line(buf, area.x, y, area.width, &text, style);
                }
            }
        }

        // ── Separator ──
        let sep_y = area.y + area.height - COMMIT_HEIGHT;
        if sep_y < buf.area().bottom() {
            let sep_style = Style::default().fg(theme::BORDER_DIM).bg(bg);
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() {
                    buf[(cx, sep_y)].set_char('─').set_style(sep_style);
                }
            }
        }

        // ── Commit input, prefixed with the repository it commits to ──
        let input_y = sep_y + 1;
        if input_y < buf.area().bottom() {
            let input_bg = theme::INPUT_BG;
            let input_style = Style::default().fg(fg).bg(input_bg);
            let is_commit_focused = self.region == GitRegion::CommitInput;
            let right = area.x + area.width;

            for row in 0..2u16 {
                self.render_line(buf, area.x, input_y + row, area.width, "", input_style);
            }

            let prompt = match self.active_name() {
                Some(name) if self.sections.len() > 1 => {
                    format!("{} > ", truncate_chars(name, PROMPT_NAME_MAX))
                }
                _ => "> ".to_string(),
            };
            let prompt_style = Style::default().fg(header_fg).bg(input_bg);
            self.put_text(buf, area.x, input_y, right, &prompt, prompt_style);

            let text_x = area.x + prompt.chars().count() as u16;
            if self.commit_input.text.is_empty() && !is_commit_focused {
                let hint_style = Style::default().fg(dim_fg).bg(input_bg);
                self.put_text(buf, text_x, input_y, right, "commit message...", hint_style);
            } else {
                self.put_text(buf, text_x, input_y, right, &self.commit_input.text, input_style);
            }

            if focused && is_commit_focused {
                let cursor_x = text_x + self.commit_input.cursor as u16;
                if cursor_x < right && cursor_x < buf.area().right() {
                    let cursor_style = Style::default().fg(input_bg).bg(theme::TEXT_FG);
                    buf[(cursor_x, input_y)].set_style(cursor_style);
                }
            }

            let hint = " [c]ommit  [a]mend  [s]tage  [u]nstage  [d]iscard  [b]ranch";
            let hint_style = Style::default().fg(dim_fg).bg(input_bg);
            self.put_text(buf, area.x, input_y + 1, right, hint, hint_style);
        }

        // ── Error message overlay ──
        if let Some(ref err) = self.error_message {
            let err_style = Style::default().fg(theme::PROPERTY_RED).bg(bg);
            self.put_text(buf, area.x, area.y, area.x + area.width, &format!(" Error: {} ", err), err_style);
        }

        // ── Branch picker overlay ──
        if self.branch_picker_open {
            let picker_bg = theme::INPUT_BG;
            let picker_fg = theme::TEXT_FG;
            let picker_sel_bg = theme::SELECTION_BG;

            let filtered = self.filtered_branches();
            let max_rows = (area.height as usize)
                .saturating_sub(4)
                .min(filtered.len() + 2);
            let picker_y = area.y + 1;

            let header = match self.active_name() {
                Some(name) => format!(" Branches · {name} "),
                None => " Branches ".to_string(),
            };
            let h_style = Style::default()
                .fg(header_fg)
                .bg(picker_bg)
                .add_modifier(Modifier::BOLD);
            self.render_line(buf, area.x, picker_y, area.width, &header, h_style);

            let filter_y = picker_y + 1;
            let filter_text = format!(" > {}", self.branch_filter);
            self.render_line(buf, area.x, filter_y, area.width, &filter_text, Style::default().fg(picker_fg).bg(picker_bg));

            for (i, branch) in filtered.iter().enumerate().take(max_rows) {
                let by = filter_y + 1 + i as u16;
                let line_bg = if i == self.branch_selected { picker_sel_bg } else { picker_bg };
                let marker = if branch.is_current { "* " } else { "  " };
                let text = format!(" {}{}", marker, branch.name);
                self.render_line(buf, area.x, by, area.width, &text, Style::default().fg(picker_fg).bg(line_bg));
            }
        }
    }

    /// Fill a whole line with `style`, then draw `text` from its start.
    fn render_line(&self, buf: &mut RataBuf, x: u16, y: u16, width: u16, text: &str, style: Style) {
        if y >= buf.area().bottom() {
            return;
        }
        for col in 0..width {
            let cx = x + col;
            if cx < buf.area().right() {
                buf[(cx, y)].set_char(' ').set_style(style);
            }
        }
        self.put_text(buf, x, y, x + width, text, style);
    }

    /// Draw `text` from column `x`, clipped at `right`.
    fn put_text(&self, buf: &mut RataBuf, x: u16, y: u16, right: u16, text: &str, style: Style) {
        if y >= buf.area().bottom() {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            let cx = x + i as u16;
            if cx >= right || cx >= buf.area().right() {
                break;
            }
            buf[(cx, y)].set_char(ch).set_style(style);
        }
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn status_color(status: &FileStatus) -> Color {
    match status {
        FileStatus::Modified => theme::WARNING,     // yellow
        FileStatus::Added => theme::CODE_GREEN,     // green
        FileStatus::Deleted => theme::PROPERTY_RED, // red
        FileStatus::Untracked => theme::TEXT_DIM,   // dim
        FileStatus::Renamed => theme::INFO_CYAN,    // cyan
        FileStatus::Unmerged => theme::PROPERTY_RED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, staged: bool) -> FileStatusEntry {
        FileStatusEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            staged,
        }
    }

    fn section(name: &str, unstaged: &[&str], staged: &[&str]) -> RepoSection {
        RepoSection {
            name: name.to_string(),
            branch: "main".to_string(),
            unstaged: unstaged.iter().map(|p| file(p, false)).collect(),
            staged: staged.iter().map(|p| file(p, true)).collect(),
            ..RepoSection::default()
        }
    }

    fn panel(sections: Vec<RepoSection>) -> GitPanelState {
        let mut p = GitPanelState::new();
        p.sections = sections;
        p.fix_selection();
        p
    }

    #[test]
    fn rows_group_files_under_their_repository() {
        use GitRegion::{Staged, Unstaged};
        let p = panel(vec![
            section("app", &["a.rs", "b.rs"], &["c.rs"]),
            section(".gaviero", &[], &[]),
            section("lib", &[], &["d.rs"]),
        ]);
        assert_eq!(
            p.rows(),
            [
                GitRow::Repo(0),
                GitRow::Header(0, Unstaged),
                GitRow::File(0, Unstaged, 0),
                GitRow::File(0, Unstaged, 1),
                GitRow::Header(0, Staged),
                GitRow::File(0, Staged, 0),
                GitRow::Repo(1),
                GitRow::Note(1),
                GitRow::Repo(2),
                GitRow::Header(2, Staged),
                GitRow::File(2, Staged, 0),
            ]
        );
    }

    #[test]
    fn moving_walks_titles_and_files_across_repositories() {
        let mut p = panel(vec![section("app", &["a.rs"], &[]), section("lib", &[], &["d.rs"])]);
        assert_eq!(p.selection, GitRow::Repo(0));
        p.move_down();
        assert_eq!(p.selected_path(), Some("a.rs"));
        p.move_down();
        assert_eq!((p.selection, p.active_repo), (GitRow::Repo(1), 1));
        assert_eq!(p.selected_path(), None);
        p.move_down();
        assert_eq!((p.selected_path(), p.active_repo, p.region), (Some("d.rs"), 1, GitRegion::Staged));
        p.move_down();
        assert_eq!(p.selected_path(), Some("d.rs"), "stops at the last row");
        p.move_up();
        p.move_up();
        assert_eq!((p.selected_path(), p.active_repo), (Some("a.rs"), 0));
    }

    #[test]
    fn alt_o_jumps_to_the_next_repository_title() {
        let mut p = panel(vec![section("app", &["a.rs"], &[]), section("lib", &[], &[])]);
        p.select(GitRow::File(0, GitRegion::Unstaged, 0));
        p.cycle_repo(1);
        assert_eq!((p.selection, p.active_repo), (GitRow::Repo(1), 1));
        p.cycle_repo(1);
        assert_eq!(p.selection, GitRow::Repo(0));
    }

    #[test]
    fn staging_the_selected_file_selects_the_next_one() {
        let mut p = panel(vec![section("app", &["a.rs", "b.rs"], &[])]);
        p.select(GitRow::File(0, GitRegion::Unstaged, 0));
        // What a refresh after staging a.rs leaves behind.
        p.sections[0] = section("app", &["b.rs"], &["a.rs"]);
        p.fix_selection();
        assert_eq!(p.selected_path(), Some("b.rs"));
        // Staging the last unstaged file moves to the staged ones.
        p.sections[0] = section("app", &[], &["a.rs", "b.rs"]);
        p.fix_selection();
        assert_eq!((p.selected_path(), p.region), (Some("a.rs"), GitRegion::Staged));
        // Committing everything leaves the title.
        p.sections[0] = section("app", &[], &[]);
        p.fix_selection();
        assert_eq!(p.selection, GitRow::Repo(0));
    }

    #[test]
    fn the_commit_box_keeps_the_keyboard_through_a_refresh() {
        let mut p = panel(vec![section("app", &["a.rs"], &[])]);
        p.cycle_region();
        assert_eq!(p.region, GitRegion::CommitInput);
        p.fix_selection();
        assert_eq!(p.region, GitRegion::CommitInput);
        p.cycle_region();
        assert_eq!(p.region, GitRegion::Unstaged);
    }

    #[test]
    fn clicks_hit_titles_and_files_through_the_scroll() {
        let mut p = panel(vec![section("app", &["a.rs"], &[]), section("lib", &["b.rs"], &[])]);
        // rows: Repo0, Header, File, Repo1, Header, File
        assert_eq!(p.hit_test_file(0, 20), Some(GitRow::Repo(0)));
        assert_eq!(p.hit_test_file(1, 20), None, "region header");
        assert_eq!(p.hit_test_file(2, 20), Some(GitRow::File(0, GitRegion::Unstaged, 0)));
        assert_eq!(p.hit_test_file(19, 20), None, "commit box");
        // A short panel scrolled to the last file: row 0 is now Repo(1)…
        p.select(GitRow::File(1, GitRegion::Unstaged, 0));
        let area = Rect::new(0, 0, 40, 6); // 3 list rows
        let mut buf = RataBuf::empty(area);
        p.render(area, &mut buf, true, &Theme::builtin_default());
        assert_eq!(p.hit_test_file(0, 6), Some(GitRow::Repo(1)));
        assert_eq!(p.hit_test_file(2, 6), Some(GitRow::File(1, GitRegion::Unstaged, 0)));
    }

    #[test]
    fn titles_and_the_commit_prompt_name_the_repository() {
        let mut p = panel(vec![section("app", &["a.rs"], &[]), section(".gaviero", &[], &[])]);
        p.select(GitRow::Repo(1));
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = RataBuf::empty(area);
        p.render(area, &mut buf, true, &Theme::builtin_default());
        let line = |y: u16| -> String { (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(line(0).starts_with(" app"), "{:?}", line(0));
        assert!(line(0).trim_end().ends_with("main"), "{:?}", line(0));
        assert!(line(3).starts_with(" .gaviero"), "{:?}", line(3));
        assert_eq!(line(4).trim_end(), "    no changes");
        assert!(line(8).starts_with(".gaviero > "), "{:?}", line(8));
    }
}
