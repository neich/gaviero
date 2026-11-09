//! The prompt box shared by the agent chat and the SWARM form: wrapped
//! multi-line text, caret movement by visual row, `$skill` / `@file`
//! completion, and drawing.
//!
//! The chat keeps these pieces on `AgentChatState` next to its history and
//! slash commands and calls the functions here. [`PromptInput`] bundles them
//! for a panel that only needs the box.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};

use super::text_input::TextInput;
use crate::theme;

/// What the autocomplete popup is completing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocompleteMode {
    /// `@path` references inside the prompt body — workspace files only,
    /// inserts `@path ` on accept.
    FileRef,
    /// `/attach <path>` argument — workspace files plus filesystem listing
    /// when the partial starts with `/` or `~`, inserts the bare path.
    AttachPath,
    /// `/detach <name|all>` argument — current attachment display names
    /// plus `all`, inserts the bare token.
    DetachName,
    /// `$skill` invocation — catalog-backed name completion.
    SkillRef,
    /// `/model <provider:model>` argument — provider prefix and model names.
    ModelSpec,
    /// `/effort <level>` argument — the levels the current model offers.
    Effort,
}

/// Autocomplete state for @file references or /attach|/detach arguments.
#[derive(Debug, Clone)]
pub struct FileAutocomplete {
    /// Whether the autocomplete popup is visible.
    pub active: bool,
    /// The partial text being matched (excludes any leading `@`).
    pub query: String,
    /// Byte offset where the inserted path will be anchored. For [`FileRef`]
    /// this points at the `@`; for [`AttachPath`]/[`DetachName`] it points
    /// at the first character of the argument (after `/attach ` / `/detach `).
    ///
    /// [`FileRef`]: AutocompleteMode::FileRef
    /// [`AttachPath`]: AutocompleteMode::AttachPath
    /// [`DetachName`]: AutocompleteMode::DetachName
    pub at_pos: usize,
    /// What kind of completion is active.
    pub mode: AutocompleteMode,
    /// Filtered file paths matching the query.
    pub matches: Vec<String>,
    /// Currently selected index in the matches list.
    pub selected: usize,
}

impl FileAutocomplete {
    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            at_pos: 0,
            mode: AutocompleteMode::FileRef,
            matches: Vec::new(),
            selected: 0,
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.query.clear();
        self.at_pos = 0;
        self.mode = AutocompleteMode::FileRef;
        self.matches.clear();
        self.selected = 0;
    }

    /// The popup is on screen.
    pub fn is_open(&self) -> bool {
        self.active && !self.matches.is_empty()
    }

    pub fn select_previous(&mut self) {
        if self.is_open() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if self.is_open() {
            self.selected = (self.selected + 1).min(self.matches.len() - 1);
        }
    }

    fn open(&mut self, mode: AutocompleteMode, at_pos: usize, query: &str) {
        self.active = true;
        self.mode = mode;
        self.at_pos = at_pos;
        self.query = query.to_string();
        self.selected = 0;
    }

    fn clamp_selection(&mut self) {
        if self.selected >= self.matches.len() {
            self.selected = 0;
        }
    }
}

// ── Layout ──────────────────────────────────────────────────────

/// Build visual lines as `(start_char_idx, char_count)` for `text`.
pub fn build_visual_lines(
    text: &str,
    first_line_width: usize,
    full_width: usize,
) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut pos = 0;
    for (logical_idx, logical_line) in text.split('\n').enumerate() {
        let line_char_count = logical_line.chars().count();
        let avail = if lines.is_empty() {
            first_line_width
        } else {
            full_width
        };
        if line_char_count == 0 {
            lines.push((pos, 0));
        } else {
            let mut col = 0;
            let first_visual = lines.len();
            while col < line_char_count {
                let w = if lines.is_empty() {
                    first_line_width
                } else if lines.len() == first_visual {
                    avail
                } else {
                    full_width
                };
                let w = w.max(1);
                let take = w.min(line_char_count - col);
                lines.push((pos + col, take));
                col += take;
            }
        }
        pos += line_char_count;
        if logical_idx < text.matches('\n').count() {
            pos += 1;
        }
    }
    if lines.is_empty() {
        lines.push((0, 0));
    }
    lines
}

/// Find which visual line the cursor is on and the column within it.
///
/// The row that owns the position is the last one starting at or before it.
/// At a wrap boundary that is the later row — the character under the cursor
/// is drawn there and the earlier row has no cell left for it. A position at
/// the end of a logical line stays on that line's own last row instead of
/// jumping to the row after the newline.
pub fn find_cursor_in_visual_lines(
    lines: &[(usize, usize)],
    cursor_char_pos: usize,
) -> (usize, usize) {
    let mut idx = 0;
    for (i, &(start, _)) in lines.iter().enumerate() {
        if start <= cursor_char_pos {
            idx = i;
        } else {
            break;
        }
    }
    let (start, len) = lines[idx];
    (idx, cursor_char_pos.saturating_sub(start).min(len))
}

/// Box height for `visual_lines` of text: the user's size when set, else
/// grown to fit. Both stay within 3 rows and `max_rows`.
pub fn box_height(user_rows: u16, visual_lines: usize, max_rows: u16) -> u16 {
    let max_rows = max_rows.max(3);
    if user_rows > 0 {
        user_rows.clamp(3, max_rows)
    } else {
        (visual_lines as u16).clamp(3, max_rows)
    }
}

/// The user's box size after one `Alt+Up`.
pub fn grow_rows(current: u16) -> u16 {
    (current.max(3) + 1).min(30)
}

/// The user's box size after one `Alt+Down`. Below 3 rows it returns to 0,
/// which lets the box fit its text again.
pub fn shrink_rows(current: u16) -> u16 {
    if current <= 3 { 0 } else { current - 1 }
}

// ── Vertical movement ───────────────────────────────────────────

/// Column Up/Down aims for: the sticky goal while the cursor still sits
/// where it was recorded, else the cursor's own column.
fn goal_col_for(goal: Option<(usize, usize)>, cursor: usize, current_col: usize) -> usize {
    match goal {
        Some((at, col)) if at == cursor => col,
        _ => current_col,
    }
}

/// Whether `vline` is the final visual row of its logical line.
///
/// A wrapped continuation starts exactly where the previous row ends; a gap
/// between them means a newline was consumed, so the row before it closes
/// its logical line.
fn row_is_last_of_logical_line(lines: &[(usize, usize)], vline: usize) -> bool {
    match lines.get(vline + 1) {
        Some(&(next_start, _)) => next_start != lines[vline].0 + lines[vline].1,
        None => true,
    }
}

/// Highest column on visual row `vline` a caret may occupy.
///
/// `len` (one past the row's last character) is a real cell only on the final
/// row of a logical line. On a wrapped row that position is drawn on the row
/// below, so a caret clamped to `len` would resolve back to the row it just
/// left — the row has to stop one column short instead.
fn last_owned_col(lines: &[(usize, usize)], vline: usize, len: usize) -> usize {
    if row_is_last_of_logical_line(lines, vline) {
        len
    } else {
        len.saturating_sub(1)
    }
}

/// Move the cursor up one visual line. Returns whether it moved.
pub fn move_up_visual(
    input: &mut TextInput,
    goal: &mut Option<(usize, usize)>,
    first_line_width: usize,
    full_width: usize,
) -> bool {
    let lines = build_visual_lines(&input.text, first_line_width, full_width);
    let (cur_vline, cur_col) = find_cursor_in_visual_lines(&lines, input.cursor);
    if cur_vline == 0 {
        return false;
    }
    let goal_col = goal_col_for(*goal, input.cursor, cur_col);
    let (prev_start, prev_len) = lines[cur_vline - 1];
    let max_col = last_owned_col(&lines, cur_vline - 1, prev_len);
    input.cursor = prev_start + goal_col.min(max_col);
    *goal = Some((input.cursor, goal_col));
    true
}

/// Move the cursor down one visual line. Returns whether it moved.
pub fn move_down_visual(
    input: &mut TextInput,
    goal: &mut Option<(usize, usize)>,
    first_line_width: usize,
    full_width: usize,
) -> bool {
    let lines = build_visual_lines(&input.text, first_line_width, full_width);
    let (cur_vline, cur_col) = find_cursor_in_visual_lines(&lines, input.cursor);
    if cur_vline >= lines.len() - 1 {
        return false;
    }
    let goal_col = goal_col_for(*goal, input.cursor, cur_col);
    let (next_start, next_len) = lines[cur_vline + 1];
    let max_col = last_owned_col(&lines, cur_vline + 1, next_len);
    input.cursor = next_start + goal_col.min(max_col);
    *goal = Some((input.cursor, goal_col));
    true
}

/// Char index under a screen position in a box drawn at `area`, using the
/// same wrap and cursor-follow scroll as [`render_input`].
pub fn screen_to_char(
    area: Rect,
    input: &TextInput,
    label_len: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    if row < area.y || row >= area.y + area.height || col < area.x || col >= area.x + area.width {
        return None;
    }
    if input.text.is_empty() {
        return Some(0);
    }

    let text_width = (area.width as usize).saturating_sub(label_len);
    if text_width == 0 {
        return Some(0);
    }

    let lines = build_visual_lines(&input.text, text_width, area.width as usize);
    let (cursor_line, _) = find_cursor_in_visual_lines(&lines, input.cursor);
    let total_rows = area.height as usize;
    let scroll = if cursor_line >= total_rows {
        cursor_line - total_rows + 1
    } else {
        0
    };

    let viewport_row = (row - area.y) as usize;
    let line_idx = scroll + viewport_row;
    if line_idx >= lines.len() {
        return Some(input.char_count());
    }

    let (start, len) = lines[line_idx];
    let x_start = if line_idx == 0 {
        area.x.saturating_add(label_len as u16)
    } else {
        area.x
    };
    if col <= x_start {
        return Some(start);
    }

    let col_in_line = (col - x_start) as usize;
    let offset = col_in_line.min(len);
    Some((start + offset).min(input.char_count()))
}

// ── Completion ──────────────────────────────────────────────────

/// Open, move or close the popup for the `$skill` or `@path` at the cursor.
pub fn update_reference_autocomplete(input: &TextInput, autocomplete: &mut FileAutocomplete) {
    let byte_pos = input.cursor_byte_offset();
    let text = &input.text;
    let before_cursor = &text[..byte_pos];

    if let Some(pos) = before_cursor.rfind('$') {
        let starts_token = pos == 0 || matches!(text.as_bytes()[pos - 1], b' ' | b'\n' | b'\t');
        let query = &before_cursor[pos + 1..];
        if starts_token && !query.contains(' ') && query.len() >= 2 {
            autocomplete.open(AutocompleteMode::SkillRef, pos, query);
            return;
        }
    }

    let Some(pos) = before_cursor.rfind('@') else {
        autocomplete.reset();
        return;
    };
    if pos > 0 && !matches!(text.as_bytes()[pos - 1], b' ' | b'\n' | b'\t') {
        autocomplete.reset();
        return;
    }
    let query = &before_cursor[pos + 1..];
    if query.contains(' ') {
        autocomplete.reset();
        return;
    }
    autocomplete.open(AutocompleteMode::FileRef, pos, query);
}

/// Fill a `$skill` popup from the catalog.
pub fn set_skill_matches(
    autocomplete: &mut FileAutocomplete,
    catalog: &gaviero_core::skills::SkillCatalog,
    active_repo_id: Option<&str>,
) {
    if !autocomplete.active || autocomplete.mode != AutocompleteMode::SkillRef {
        return;
    }
    autocomplete.matches = match_skills(catalog, &autocomplete.query, active_repo_id, 10)
        .into_iter()
        .map(|(insert, _)| insert)
        .collect();
    autocomplete.clamp_selection();
}

/// Fill a path popup from workspace file paths.
pub fn set_file_matches(autocomplete: &mut FileAutocomplete, all_files: &[String]) {
    autocomplete.matches = match_file_paths(all_files, &autocomplete.query, 10);
    autocomplete.clamp_selection();
}

/// Replace the text being completed with the selected match.
pub fn accept_autocomplete(input: &mut TextInput, autocomplete: &mut FileAutocomplete) {
    if !autocomplete.is_open() {
        return;
    }
    let selected = autocomplete.selected.min(autocomplete.matches.len() - 1);
    let path = autocomplete.matches[selected].clone();
    let anchor = autocomplete.at_pos;

    let cursor_byte = input.cursor_byte_offset();
    let after_cursor = if matches!(
        autocomplete.mode,
        AutocompleteMode::ModelSpec | AutocompleteMode::Effort
    ) {
        // A model spec is a single whitespace-free token spanning to the
        // end of the line. Accepting with the cursor mid-token must
        // replace the *whole* token, so drop the remainder of it that
        // sits right of the cursor — otherwise an existing tail like
        // `cursor:composer-2.5` gets concatenated after the accepted
        // `claude:`, yielding the doubly-prefixed `claude:cursor:...`.
        let raw = &input.text[cursor_byte..];
        match raw.find(char::is_whitespace) {
            Some(ws) => raw[ws..].to_string(),
            None => String::new(),
        }
    } else {
        input.text[cursor_byte..].to_string()
    };
    input.text.truncate(anchor);

    match autocomplete.mode {
        AutocompleteMode::FileRef => {
            // Replace `@<query>` with `@<path> ` (trailing space keeps
            // typing fluid after accept).
            input.text.push('@');
            input.text.push_str(&path);
            input.text.push(' ');
        }
        AutocompleteMode::AttachPath | AutocompleteMode::DetachName => {
            // Replace just the argument; no trailing space because
            // `/attach` / `/detach` take a single token.
            input.text.push_str(&path);
        }
        AutocompleteMode::SkillRef => {
            input.text.push_str(&path);
            input.text.push(' ');
        }
        AutocompleteMode::ModelSpec | AutocompleteMode::Effort => {
            input.text.push_str(&path);
        }
    }
    input.cursor = input.char_count();
    input.text.push_str(&after_cursor);

    autocomplete.reset();
}

/// `$` completion matches: catalog skills whose name starts with `prefix`,
/// in the catalog's ranking, each with its insertion text. Shared by the
/// desktop popup and remote `request_skill_completions`.
pub fn match_skills<'a>(
    catalog: &'a gaviero_core::skills::SkillCatalog,
    prefix: &str,
    active_repo_id: Option<&str>,
    limit: usize,
) -> Vec<(String, &'a gaviero_core::skills::Skill)> {
    let hits = catalog.complete(prefix, active_repo_id);
    let mut by_name: std::collections::HashMap<&str, Vec<&gaviero_core::skills::Skill>> =
        std::collections::HashMap::new();
    for skill in &hits {
        by_name.entry(&skill.name).or_default().push(*skill);
    }
    hits.iter()
        .take(limit)
        .map(|skill| {
            let siblings = by_name
                .get(skill.name.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            (skill_autocomplete_insert(catalog, skill, siblings), *skill)
        })
        .collect()
}

/// Build the insertion text for a skill autocomplete selection.
pub fn skill_autocomplete_insert(
    catalog: &gaviero_core::skills::SkillCatalog,
    skill: &gaviero_core::skills::Skill,
    all_with_name: &[&gaviero_core::skills::Skill],
) -> String {
    let bare = format!("${}", skill.name);
    if all_with_name.len() <= 1 {
        if skill.arguments.is_empty() {
            return bare;
        }
        return format!("{}()", bare);
    }
    let label = catalog.source_label(skill);
    let qualified = format!("${}/{}", label, skill.name);
    if skill.arguments.is_empty() {
        qualified
    } else {
        format!("{}()", qualified)
    }
}

/// `@` completion matches: case-insensitive substring over the whole path,
/// in listing order; an empty query keeps the first `limit` paths. Shared by
/// the desktop popup and remote `request_file_completions`.
pub fn match_file_paths(all_files: &[String], query: &str, limit: usize) -> Vec<String> {
    let query_lower = query.to_lowercase();
    all_files
        .iter()
        .filter(|f| query_lower.is_empty() || f.to_lowercase().contains(&query_lower))
        .take(limit)
        .cloned()
        .collect()
}

// ── Rendering ───────────────────────────────────────────────────

/// What [`render_input`] draws. The caller clears `area` first.
pub struct InputView<'a> {
    pub input: &'a TextInput,
    /// Drawn before the first row of text.
    pub label: &'a str,
    /// Shown while the box is empty.
    pub hint: Option<&'a str>,
    pub show_caret: bool,
}

/// Draw the label, the wrapped text with its selection, the caret, and a
/// scrollbar when the text is taller than the box.
pub fn render_input(area: Rect, buf: &mut Buffer, view: &InputView<'_>) {
    let bg = theme::INPUT_BG;
    let fg = theme::TEXT_BRIGHT;
    let style = Style::default().fg(fg).bg(bg);
    let input = view.input;

    let prompt_style = Style::default().fg(theme::ACCENT).bg(bg);
    let mut x = area.x;
    for ch in view.label.chars() {
        if x < area.x + area.width && x < buf.area().right() {
            buf[(x, area.y)].set_char(ch).set_style(prompt_style);
            x += 1;
        }
    }

    // Input text — wraps across available rows
    let prompt_len = (x - area.x) as usize;
    let text_width = (area.width as usize).saturating_sub(prompt_len);
    let total_rows = area.height as usize;

    if let Some(hint) = view.hint.filter(|_| input.text.is_empty()) {
        let hint_style = Style::default().fg(theme::TEXT_DIM).bg(bg);
        for (i, ch) in hint.chars().enumerate() {
            let hx = x + i as u16;
            if hx < area.x + area.width && hx < buf.area().right() {
                buf[(hx, area.y)].set_char(ch).set_style(hint_style);
            }
        }
        // Show cursor at prompt position even with empty input
        if view.show_caret {
            let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
            let cx = x.min((area.x + area.width).saturating_sub(1));
            if cx < buf.area().right() && area.y < buf.area().bottom() {
                buf[(cx, area.y)].set_style(cursor_style);
            }
        }
    } else if text_width > 0 {
        let input_chars: Vec<char> = input.text.chars().collect();
        let lines = build_visual_lines(&input.text, text_width, area.width as usize);
        let (cursor_line, cursor_col) = find_cursor_in_visual_lines(&lines, input.cursor);

        // Scroll so cursor line is visible
        let scroll = if cursor_line >= total_rows {
            cursor_line - total_rows + 1
        } else {
            0
        };

        // Render visible lines
        for row in 0..total_rows {
            let line_idx = scroll + row;
            if line_idx >= lines.len() {
                break;
            }
            let (start, len) = lines[line_idx];
            let y = area.y + row as u16;
            let x_start = if line_idx == 0 { x } else { area.x };

            // Get selection range for highlighting
            let sel_range = input.selection_range();

            for i in 0..len {
                let ch_idx = start + i;
                if ch_idx < input_chars.len() {
                    let ch = input_chars[ch_idx];
                    if ch == '\n' || ch == '\r' {
                        continue;
                    }
                    let cx = x_start + i as u16;
                    if cx < area.x + area.width && cx < buf.area().right() && y < buf.area().bottom()
                    {
                        let ch_style = if sel_range.is_some_and(|(s, e)| ch_idx >= s && ch_idx < e) {
                            Style::default().fg(fg).bg(theme::SELECTION_BG)
                        } else {
                            style
                        };
                        buf[(cx, y)].set_char(ch).set_style(ch_style);
                    }
                }
            }
        }

        // Position cursor. When the caret column sits at the row's right
        // edge there is no cell left *inside* the panel for it, so it is
        // clamped onto the row's last cell (the readline convention) rather
        // than dropped, which used to hide the caret at the end of a prompt
        // that exactly filled its line.
        if view.show_caret {
            let visible_cursor_line = cursor_line.saturating_sub(scroll);
            if visible_cursor_line < total_rows {
                let y = area.y + visible_cursor_line as u16;
                let x_start = if cursor_line == 0 { x } else { area.x };
                let row_right = area.x + area.width; // one past the row's last cell
                let cursor_x = (x_start + cursor_col as u16).min(row_right - 1);
                if cursor_x < buf.area().right() && y < buf.area().bottom() {
                    let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
                    buf[(cursor_x, y)].set_style(cursor_style);
                }
            }
        }

        // Scrollbar when the prompt exceeds the input viewport.
        if lines.len() > total_rows {
            super::scrollbar::render_scrollbar(area, buf, lines.len(), total_rows, scroll);
        }
    }
}

/// Draw the popup rows into `area`.
pub fn render_autocomplete(autocomplete: &FileAutocomplete, area: Rect, buf: &mut Buffer) {
    let bg = theme::INPUT_BG;
    let fg = theme::TEXT_FG;
    let selected_bg = theme::SELECTION_BG;

    for (row, (i, path)) in autocomplete
        .matches
        .iter()
        .enumerate()
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y + row as u16;
        let is_selected = i == autocomplete.selected;
        let style = if is_selected {
            Style::default().fg(Color::White).bg(selected_bg)
        } else {
            Style::default().fg(fg).bg(bg)
        };

        // Clear row
        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(' ').set_style(style);
            }
        }

        // Prefix only relevant for @file refs — the /attach popup shows
        // bare paths so they line up with what gets inserted on accept.
        let display = match autocomplete.mode {
            AutocompleteMode::FileRef => format!(" @{}", path),
            AutocompleteMode::AttachPath
            | AutocompleteMode::DetachName
            | AutocompleteMode::ModelSpec
            | AutocompleteMode::Effort => {
                format!(" {}", path)
            }
            AutocompleteMode::SkillRef => path.clone(),
        };
        let display = fit_autocomplete_label(&display, area.width as usize);
        for (ci, ch) in display.chars().enumerate() {
            let cx = area.x + ci as u16;
            if cx < area.x + area.width && cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(ch).set_style(style);
            }
        }
    }
}

/// Fit an autocomplete row into `cols` cells.
///
/// Labels that fit are returned unchanged. Overflow drops the prefix and
/// keeps the tail — the filename — starting at a `/` or `\` when one falls
/// inside the window, so a long path stays identifiable.
pub(crate) fn fit_autocomplete_label(label: &str, cols: usize) -> String {
    let chars: Vec<char> = label.chars().collect();
    if cols == 0 || chars.len() <= cols {
        return if cols == 0 {
            String::new()
        } else {
            label.to_string()
        };
    }
    if cols == 1 {
        return "…".to_string();
    }
    let budget = cols - 1;
    let mut start = chars.len() - budget;
    if let Some(rel) = chars[start..].iter().position(|c| *c == '/' || *c == '\\') {
        let boundary = start + rel;
        if boundary + 1 < chars.len() && chars.len() - boundary <= budget {
            start = boundary;
        }
    }
    let mut out = String::from("…");
    out.extend(chars[start..].iter().copied());
    out
}

/// Where the popup for a box at `input` goes inside `bounds`: above the box,
/// or below it when more of the list fits there. `None` if neither has room.
pub fn popup_area(input: Rect, bounds: Rect, match_count: usize) -> Option<Rect> {
    let wanted = match_count.min(8) as u16;
    let above = input.y.saturating_sub(bounds.y);
    let below = bounds.bottom().saturating_sub(input.bottom());
    let (y, height) = if above >= wanted || above >= below {
        (input.y - wanted.min(above), wanted.min(above))
    } else {
        (input.bottom(), wanted.min(below))
    };
    (height > 0).then_some(Rect {
        x: input.x,
        y,
        width: input.width,
        height,
    })
}

// ── A standalone box ────────────────────────────────────────────

/// Width assumed before the box has been drawn once.
const DEFAULT_WIDTH: u16 = 40;

/// A prompt box for a panel with no chat around it.
#[derive(Debug, Clone)]
pub struct PromptInput {
    pub input: TextInput,
    pub autocomplete: FileAutocomplete,
    /// Where the box was last drawn; `None` while it is off screen.
    pub area: Option<Rect>,
    pub dragging: bool,
    goal_col: Option<(usize, usize)>,
}

impl PromptInput {
    /// Drawn before the first row of text, as in the chat.
    pub const LABEL: &'static str = "> ";

    pub fn new() -> Self {
        Self {
            input: TextInput::new(),
            autocomplete: FileAutocomplete::new(),
            area: None,
            dragging: false,
            goal_col: None,
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        self.input.insert_char(ch);
        self.update_autocomplete();
    }

    pub fn insert_str(&mut self, text: &str) {
        // A `\r` in a cell reaches the terminal and overwrites other panels.
        if text.contains('\r') {
            self.input
                .insert_str(&crate::editor::normalize_paste_newlines(text));
        } else {
            self.input.insert_str(text);
        }
        self.update_autocomplete();
    }

    pub fn backspace(&mut self) {
        self.input.backspace();
        self.update_autocomplete();
    }

    pub fn delete_word_back(&mut self) {
        self.input.delete_word_back();
        self.update_autocomplete();
    }

    pub fn accept_autocomplete(&mut self) {
        accept_autocomplete(&mut self.input, &mut self.autocomplete);
    }

    fn update_autocomplete(&mut self) {
        update_reference_autocomplete(&self.input, &mut self.autocomplete);
    }

    /// `(first_row_text_width, full_row_width)` of the box.
    fn widths(&self) -> (usize, usize) {
        let full = self.area.map_or(DEFAULT_WIDTH, |a| a.width) as usize;
        (full.saturating_sub(Self::LABEL.chars().count()), full)
    }

    /// Text rows the box needs at `width`.
    pub fn visual_line_count(&self, width: u16) -> usize {
        let full = width as usize;
        let first = full.saturating_sub(Self::LABEL.chars().count());
        build_visual_lines(&self.input.text, first, full).len()
    }

    /// Move up one visual row. False on the top row, so the caller can act.
    pub fn cursor_up(&mut self) -> bool {
        let (first, full) = self.widths();
        self.input.sel_anchor = None;
        move_up_visual(&mut self.input, &mut self.goal_col, first, full)
    }

    /// Move down one visual row. False on the bottom row.
    pub fn cursor_down(&mut self) -> bool {
        let (first, full) = self.widths();
        self.input.sel_anchor = None;
        move_down_visual(&mut self.input, &mut self.goal_col, first, full)
    }

    pub fn select_up(&mut self) {
        let (first, full) = self.widths();
        self.input.ensure_anchor();
        move_up_visual(&mut self.input, &mut self.goal_col, first, full);
    }

    pub fn select_down(&mut self) {
        let (first, full) = self.widths();
        self.input.ensure_anchor();
        move_down_visual(&mut self.input, &mut self.goal_col, first, full);
    }

    /// Move the cursor by `delta` visual rows (negative = up).
    pub fn scroll_by(&mut self, delta: i32) {
        for _ in 0..delta.unsigned_abs() {
            let moved = if delta < 0 {
                self.cursor_up()
            } else {
                self.cursor_down()
            };
            if !moved {
                break;
            }
        }
    }

    /// Place the cursor at a click and start a drag selection there.
    pub fn start_drag(&mut self, col: u16, row: u16) {
        let Some(index) = self.char_at(col, row) else {
            return;
        };
        self.input.sel_anchor = Some(index);
        self.input.cursor = index;
        self.dragging = true;
    }

    /// Extend the drag to a screen position, clamping to the text's ends when
    /// the pointer has left the box.
    pub fn extend_drag(&mut self, col: u16, row: u16) {
        let index = match (self.char_at(col, row), self.area) {
            (Some(index), _) => index,
            (None, Some(area)) if row < area.y => 0,
            (None, Some(_)) => self.input.char_count(),
            (None, None) => return,
        };
        self.input.cursor = index;
    }

    /// End the drag; a plain click leaves no selection.
    pub fn end_drag(&mut self) {
        self.dragging = false;
        if self.input.sel_anchor == Some(self.input.cursor) {
            self.input.sel_anchor = None;
        }
    }

    fn char_at(&self, col: u16, row: u16) -> Option<usize> {
        screen_to_char(self.area?, &self.input, Self::LABEL.chars().count(), col, row)
    }
}

impl Default for PromptInput {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> PromptInput {
        let mut prompt = PromptInput::new();
        prompt.insert_str(text);
        prompt
    }

    #[test]
    fn dollar_opens_skill_completion_after_two_characters() {
        let mut prompt = typed("do $l");
        assert!(!prompt.autocomplete.active);

        prompt.insert_char('i');
        assert!(prompt.autocomplete.active);
        assert_eq!(prompt.autocomplete.mode, AutocompleteMode::SkillRef);
        assert_eq!(prompt.autocomplete.query, "li");
    }

    #[test]
    fn a_dollar_inside_a_word_opens_nothing() {
        assert!(!typed("cost$lint").autocomplete.active);
    }

    #[test]
    fn at_sign_opens_file_completion_and_a_space_closes_it() {
        let mut prompt = typed("see @src");
        assert_eq!(prompt.autocomplete.mode, AutocompleteMode::FileRef);
        assert!(prompt.autocomplete.active);

        prompt.insert_char(' ');
        assert!(!prompt.autocomplete.active);
    }

    #[test]
    fn accepting_a_skill_replaces_the_query_and_keeps_later_text() {
        let mut prompt = typed("use $li now");
        prompt.input.cursor = "use $li".len();
        prompt.backspace();
        prompt.insert_char('i');
        prompt.autocomplete.matches = vec!["$lint".to_string()];

        prompt.accept_autocomplete();

        assert_eq!(prompt.input.text, "use $lint  now");
        assert_eq!(prompt.input.cursor, "use $lint ".len());
        assert!(!prompt.autocomplete.active);
    }

    #[test]
    fn a_pasted_carriage_return_never_reaches_the_text() {
        assert_eq!(typed("a\r\nb").input.text, "a\nb");
    }

    #[test]
    fn vertical_movement_follows_rows_and_reports_the_edges() {
        let mut prompt = typed("one\ntwo");
        assert!(prompt.cursor_up());
        assert_eq!(prompt.input.cursor, 3);
        assert!(!prompt.cursor_up(), "the top row hands Up back");
        assert!(prompt.cursor_down());
        assert!(!prompt.cursor_down(), "the bottom row hands Down back");
    }

    #[test]
    fn box_height_fits_the_text_until_the_user_sizes_it() {
        assert_eq!(box_height(0, 1, 10), 3);
        assert_eq!(box_height(0, 7, 10), 7);
        assert_eq!(box_height(0, 30, 10), 10);
        assert_eq!(box_height(6, 1, 10), 6);
        assert_eq!(box_height(20, 1, 10), 10);
    }

    #[test]
    fn resizing_steps_a_row_and_shrinks_back_to_fitting() {
        assert_eq!(grow_rows(0), 4);
        assert_eq!(grow_rows(30), 30);
        assert_eq!(shrink_rows(5), 4);
        assert_eq!(shrink_rows(3), 0);
        assert_eq!(shrink_rows(0), 0);
    }

    #[test]
    fn popup_prefers_above_and_flips_below_when_only_that_fits() {
        let bounds = Rect::new(0, 0, 40, 30);
        let low = Rect::new(1, 20, 38, 3);
        assert_eq!(popup_area(low, bounds, 5), Some(Rect::new(1, 15, 38, 5)));

        let high = Rect::new(1, 2, 38, 3);
        assert_eq!(popup_area(high, bounds, 5), Some(Rect::new(1, 5, 38, 5)));

        let full = Rect::new(0, 0, 40, 30);
        assert_eq!(popup_area(full, bounds, 5), None);
    }

    #[test]
    fn a_click_places_the_cursor_and_a_drag_selects() {
        let mut prompt = typed("hello world");
        prompt.area = Some(Rect::new(10, 5, 30, 3));

        prompt.start_drag(14, 5); // label is 2 cells: col 14 is char 2
        assert_eq!(prompt.input.cursor, 2);
        prompt.end_drag();
        assert_eq!(prompt.input.selection_range(), None, "a click selects nothing");

        prompt.start_drag(12, 5);
        prompt.extend_drag(17, 5);
        assert_eq!(prompt.input.selection_range(), Some((0, 5)));
        prompt.extend_drag(17, 20);
        assert_eq!(prompt.input.cursor, 11, "dragged past the box: the end");
    }
}
