//! TUI HISTORY panel — "what happened in this turn?"
//!
//! Reads `<workspace>/.gaviero/history/turns.ndjson` (written by the single
//! `gaviero_core::history::HistoryRecorder`) and shows, per turn, the full
//! prompt, every tool call with its arguments and result, every MCP call with
//! its request and response, the memory call with its response, and the
//! token totals.
//!
//! Rules this panel keeps:
//! * **Read-only.** No key writes to `memory.db` or the workspace. The file is
//!   read off the event loop (`spawn_blocking`) and delivered as
//!   `Event::HistoryLoaded`; a missing or malformed log degrades to a message.
//! * **Every token number carries its provenance.** Per-item numbers are
//!   prefixed `~` and the footer legend names the estimators; exact numbers
//!   are labelled `exact` and come only from provider-reported usage.
//! * **Diffs come from the turn-capture blob store, never from the workspace.**
//!   The log keeps hashes only ([`ChangedFile`]), so `Enter` on a file reads both
//!   sides back through `gaviero_core::turn_capture::file_texts` and opens them
//!   with `crate::app::editing::open_diff_view` — the read-only diff *tab* the
//!   editor already builds for the git panel, with its syntax highlighting,
//!   gutter, wrapping and scrolling. The panel renders no diff of its own, so the
//!   app keeps one whole-file diff viewer, not two. The handler does the reading
//!   (render never touches disk); content the store has already reclaimed is
//!   reported as such, never approximated from the file that happens to be on
//!   disk now.
//! * **Render is pure.** All state changes go through the action handler.

use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use gaviero_core::history::{
    CaptureMode, ChangedFile, Estimator, FilesChanged, HistoryKind, HistoryRecord, McpCall,
    MemoryInjection, ToolCall, ToolOutput, TurnEnd, TurnRecords, TurnStart, TurnStatus,
    compact_count, grouped_count,
};

const COLOR_ACCENT: Color = Color::Rgb(97, 175, 239);
const COLOR_TEXT: Color = Color::Rgb(171, 178, 191);
const COLOR_MUTED: Color = Color::Rgb(127, 132, 142);
const COLOR_BORDER: Color = Color::Rgb(80, 86, 95);
const COLOR_WARN: Color = Color::Rgb(224, 108, 117);
const COLOR_OK: Color = Color::Rgb(152, 195, 121);

/// Below this inner width the turn list stacks above the detail instead of
/// sitting beside it.
const TWO_COLUMN_MIN_WIDTH: u16 = 90;

/// Screen rows one turn occupies in the turn list (title + meta).
const ROWS_PER_TURN: usize = 2;

/// Files `J`/`K` / `PgUp`/`PgDn` skip in the FILES list.
pub const FILE_PAGE: isize = 10;

/// The rects the panel paints into, derived from its outer area.
///
/// `render` and the mouse hit-tests ([`HistoryPanelState::hit_test_turn`],
/// [`HistoryPanelState::hit_test_file`]) both go through [`HistoryPanelState::panel_geometry`],
/// so a click can never drift from what was drawn.
#[derive(Debug, Clone, Copy)]
pub struct PanelGeometry {
    /// The section body: the panel minus its status bar and legend.
    pub body: Rect,
    /// The turn-list column. `None` in the expanded view, which has no list.
    pub list: Option<Rect>,
    /// Where the detail (the focused section) is painted.
    pub detail: Rect,
    /// `Enter`: the detail fills the whole body and there is no list beside it.
    pub expanded: bool,
}

/// The six detail sections. `Tab` / `Alt+O` / `Alt+I` cycle; `1`–`6` jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySection {
    Prompt,
    Tools,
    Mcp,
    Memory,
    Totals,
    /// Files the turn changed on disk and how its review resolved.
    Files,
}

impl HistorySection {
    pub const ALL: [HistorySection; 6] = [
        HistorySection::Prompt,
        HistorySection::Tools,
        HistorySection::Mcp,
        HistorySection::Memory,
        HistorySection::Totals,
        HistorySection::Files,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// `'1'`..`'6'` → section.
    pub fn from_digit(c: char) -> Option<Self> {
        let n = c.to_digit(10)? as usize;
        (1..=Self::ALL.len()).contains(&n).then(|| Self::ALL[n - 1])
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Prompt => "PROMPT",
            Self::Tools => "TOOLS",
            Self::Mcp => "MCP",
            Self::Memory => "MEMORY",
            Self::Totals => "TOTALS",
            Self::Files => "FILES",
        }
    }
}

/// Which turns the list shows. `a` toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryScope {
    AllConversations,
    ActiveConversation,
}

/// What a file read produced, shaped for the panel.
#[derive(Debug, Clone)]
pub struct HistoryLoad {
    pub source: PathBuf,
    /// Oldest first, as the reader returns them.
    pub turns: Vec<TurnRecords>,
    pub skipped: usize,
}

/// Read and group the log. Pure (no `App`), so the loader is testable and
/// runs inside `spawn_blocking`. Only the current generation is read; the
/// rotated `.1` generation is reachable through `gaviero-cli --history`.
pub fn load_history(path: &Path) -> HistoryLoad {
    let out = gaviero_core::history::read_records(path, false);
    HistoryLoad {
        source: path.to_path_buf(),
        turns: gaviero_core::history::group_turns(out.records),
        skipped: out.skipped,
    }
}

/// `M` / `A` / `D` and its colour, from the record's `change` word.
fn change_glyph(change: &str) -> (&'static str, Color) {
    match change {
        "added" => ("A", COLOR_OK),
        "deleted" => ("D", COLOR_WARN),
        _ => ("M", COLOR_WARN),
    }
}

/// The FILES section's order: **modified, added, deleted** — what changed, what
/// appeared, what went — each group by path so the list is stable across
/// reloads. An unknown kind (a newer record) sorts last rather than panicking.
pub fn ordered_files(turn: &TurnRecords) -> Vec<&ChangedFile> {
    let Some(changed) = files_changed(turn) else {
        return Vec::new();
    };
    let mut out: Vec<&ChangedFile> = changed.files.iter().collect();
    out.sort_by(|a, b| {
        change_rank(&a.change)
            .cmp(&change_rank(&b.change))
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn change_rank(change: &str) -> u8 {
    match change {
        "modified" => 0,
        "added" => 1,
        "deleted" => 2,
        _ => 3,
    }
}

/// The turn's `files_changed` record, if capture recorded one.
pub fn files_changed(turn: &TurnRecords) -> Option<&FilesChanged> {
    turn.records.iter().find_map(|e| match &e.record.payload {
        HistoryKind::FilesChanged(f) => Some(f),
        _ => None,
    })
}

#[derive(Debug, Clone)]
pub struct HistoryPanelState {
    pub loaded: bool,
    pub loading: bool,
    /// A reload was requested while a read was in flight.
    pub reload_pending: bool,
    pub source: Option<PathBuf>,
    /// All turns, oldest first.
    pub turns: Vec<TurnRecords>,
    pub skipped_lines: usize,
    /// Index into [`Self::visible`] (newest first).
    pub selected: usize,
    pub section: HistorySection,
    pub section_scroll: usize,
    pub filter: String,
    pub filter_editing: bool,
    pub scope: HistoryScope,
    /// The chat's active conversation, captured when the panel is opened or
    /// the scope is toggled. Drives [`HistoryScope::ActiveConversation`].
    pub active_conv: Option<String>,
    /// `Enter`: the focused section fills the whole panel.
    pub expanded: bool,
    /// Cursor into the focused turn's [`ordered_files`] list (FILES section).
    pub file_selected: usize,
}

impl Default for HistoryPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryPanelState {
    pub fn new() -> Self {
        Self {
            loaded: false,
            loading: false,
            reload_pending: false,
            source: None,
            turns: Vec::new(),
            skipped_lines: 0,
            selected: 0,
            section: HistorySection::Prompt,
            section_scroll: 0,
            filter: String::new(),
            filter_editing: false,
            scope: HistoryScope::AllConversations,
            active_conv: None,
            expanded: false,
            file_selected: 0,
        }
    }

    /// Install a finished load, keeping the selected turn when it still
    /// exists.
    pub fn apply_load(&mut self, load: HistoryLoad) {
        let keep = self.selected_turn().and_then(|t| t.turn_id.clone());
        self.source = Some(load.source);
        self.turns = load.turns;
        self.skipped_lines = load.skipped;
        self.loaded = true;
        self.loading = false;
        let visible = self.visible();
        self.selected = keep
            .and_then(|id| {
                visible
                    .iter()
                    .position(|&i| self.turns[i].turn_id.as_deref() == Some(id.as_str()))
            })
            .unwrap_or(0);
        self.clamp_selection();
        // A reload may have changed the file list under an open diff.
        self.reset_file_cursor();
    }

    /// Indices into `turns` for the list, newest first, after scope and
    /// filter.
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        (0..self.turns.len())
            .rev()
            .filter(|&i| {
                let turn = &self.turns[i];
                let in_scope = match self.scope {
                    HistoryScope::AllConversations => true,
                    HistoryScope::ActiveConversation => {
                        turn.conv_id.is_some() && turn.conv_id == self.active_conv
                    }
                };
                in_scope && (needle.is_empty() || turn_matches(turn, &needle))
            })
            .collect()
    }

    pub fn selected_turn(&self) -> Option<&TurnRecords> {
        self.visible()
            .get(self.selected)
            .and_then(|&i| self.turns.get(i))
    }

    pub fn select_next(&mut self) {
        let n = self.visible().len();
        if self.selected + 1 < n {
            self.selected += 1;
            self.section_scroll = 0;
            self.reset_file_cursor();
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.section_scroll = 0;
            self.reset_file_cursor();
        }
    }

    /// Drop the FILES cursor: another turn's files are a different list.
    pub fn reset_file_cursor(&mut self) {
        self.file_selected = 0;
    }

    /// The focused turn's changed files, in the section's own order.
    pub fn files(&self) -> Vec<&ChangedFile> {
        self.selected_turn().map(ordered_files).unwrap_or_default()
    }

    pub fn file_count(&self) -> usize {
        self.files().len()
    }

    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.files().get(self.file_selected).copied()
    }

    pub fn select_file_next(&mut self) {
        let n = self.file_count();
        if self.file_selected + 1 < n {
            self.file_selected += 1;
        }
    }

    pub fn select_file_prev(&mut self) {
        self.file_selected = self.file_selected.saturating_sub(1);
    }

    /// `J`/`K` / `PgUp`/`PgDn` in the FILES section: jump whole pages of files.
    pub fn select_file_by(&mut self, delta: isize) {
        let last = self.file_count().saturating_sub(1) as isize;
        let next = (self.file_selected as isize).saturating_add(delta);
        self.file_selected = next.clamp(0, last.max(0)) as usize;
    }

    /// `Home` / `End` in the FILES section: the first / last file.
    pub fn select_file_to(&mut self, bottom: bool) {
        self.file_selected = if bottom {
            self.file_count().saturating_sub(1)
        } else {
            0
        };
    }

    /// Click on a FILES row: put the cursor there (clamped).
    pub fn select_file(&mut self, idx: usize) {
        self.file_selected = idx.min(self.file_count().saturating_sub(1));
    }

    /// Click on the turn list: select that turn and reset what belonged to the
    /// previous one (`section_scroll`, the FILES cursor).
    pub fn select_turn(&mut self, visible_idx: usize) {
        let n = self.visible().len();
        if n == 0 {
            return;
        }
        let next = visible_idx.min(n - 1);
        if next != self.selected {
            self.selected = next;
            self.section_scroll = 0;
            self.reset_file_cursor();
        }
    }

    /// The focused section's body plus the body-line index of each FILES row —
    /// the map [`Self::hit_test_file`] turns a screen row back into a file.
    pub fn body_with_rows(&self, turn: &TurnRecords) -> (Vec<Line<'static>>, Vec<usize>) {
        let cursor = (self.section == HistorySection::Files).then_some(self.file_selected);
        let mut rows = Vec::new();
        let lines = section_lines_with_rows(turn, self.section, cursor, &mut rows);
        (lines, rows)
    }

    /// Which visible turn is at `(col, row)`, or `None` off the turn list.
    ///
    /// Mirrors `render_list`: two rows per turn, scrolled so the selected turn
    /// stays visible. The unit is a *visible* index (newest first), the unit
    /// [`Self::selected`] already uses.
    pub fn hit_test_turn(&self, area: Rect, col: u16, row: u16) -> Option<usize> {
        let list = self.panel_geometry(area)?.list?;
        if !list.contains((col, row).into()) {
            return None;
        }
        let visible = self.visible();
        let capacity = (list.height as usize / ROWS_PER_TURN).max(1);
        let first = self.selected.saturating_sub(capacity.saturating_sub(1));
        let pos = first + (row - list.y) as usize / ROWS_PER_TURN;
        (pos < visible.len()).then_some(pos)
    }

    /// Which row of the focused turn's [`ordered_files`] is at `(col, row)`, or
    /// `None` off the FILES list (or on one of its headings).
    pub fn hit_test_file(&self, area: Rect, col: u16, row: u16) -> Option<usize> {
        if self.section != HistorySection::Files {
            return None;
        }
        let geo = self.panel_geometry(area)?;
        if !geo.detail.contains((col, row).into()) {
            return None;
        }
        let turn = self.selected_turn()?;
        let (body, file_rows) = self.body_with_rows(turn);
        let header = self.detail_header_height(turn, geo.expanded, geo.detail.height);
        let screen = row.checked_sub(geo.detail.y.saturating_add(header))? as usize;
        let width = geo.detail.width.max(1) as usize;
        // `Paragraph::scroll` counts *wrapped* rows, so a file row that is too
        // wide for the panel occupies several of them.
        let target = screen + self.body_scroll(&body, width);
        let mut top = 0usize;
        for (line, text) in body.iter().enumerate() {
            let height = wrapped_height(text, width);
            if target < top + height {
                return file_rows.binary_search(&line).ok();
            }
            top += height;
        }
        None
    }

    /// The clamped vertical scroll `render_detail` draws the body with. Both
    /// sides must agree, or a click near the bottom of a long section would
    /// name the wrong row.
    fn body_scroll(&self, body: &[Line<'static>], width: usize) -> usize {
        let total: usize = body.iter().map(|l| wrapped_height(l, width)).sum();
        self.section_scroll.min(total.saturating_sub(1))
    }

    /// Rows the pinned detail header takes above the section body.
    fn detail_header_height(&self, turn: &TurnRecords, expanded: bool, detail_height: u16) -> u16 {
        if expanded {
            0
        } else {
            (detail_header_lines(turn, self.section).len() as u16).min(detail_height)
        }
    }

    /// Where everything lands inside `area` (the panel's own content rect).
    /// `None` when the panel is too small to lay out at all, which is also
    /// where `render` stops.
    pub fn panel_geometry(&self, area: Rect) -> Option<PanelGeometry> {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        if inner.height < 3 || inner.width < 10 {
            return None;
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
        let body = rows[1];
        if self.expanded {
            return Some(PanelGeometry {
                body,
                list: None,
                detail: body,
                expanded: true,
            });
        }
        let (list, detail) = if body.width >= TWO_COLUMN_MIN_WIDTH {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                .split(body);
            (cols[0], cols[1])
        } else {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                .split(body);
            (rows[0], rows[1])
        };
        Some(PanelGeometry {
            body,
            list: Some(list),
            detail,
            expanded: false,
        })
    }

    pub fn set_section(&mut self, section: HistorySection) {
        if self.section != section {
            self.section = section;
            self.section_scroll = 0;
        }
    }

    pub fn scroll_section(&mut self, delta: isize) {
        self.section_scroll = self.section_scroll.saturating_add_signed(delta);
    }

    pub fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            HistoryScope::AllConversations => HistoryScope::ActiveConversation,
            HistoryScope::ActiveConversation => HistoryScope::AllConversations,
        };
        self.selected = 0;
        self.section_scroll = 0;
        self.reset_file_cursor();
    }

    /// `Esc`: collapse the expanded view, else stop editing the filter, else
    /// clear it. Returns whether anything changed.
    pub fn escape(&mut self) -> bool {
        if self.expanded {
            self.expanded = false;
        } else if self.filter_editing {
            self.filter_editing = false;
        } else if !self.filter.is_empty() {
            self.filter.clear();
            self.selected = 0;
        } else {
            return false;
        }
        true
    }

    pub fn clamp_selection(&mut self) {
        let n = self.visible().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// The focused section's records of the selected turn, as NDJSON — what
    /// `c` copies.
    pub fn focused_records_ndjson(&self) -> Option<String> {
        let turn = self.selected_turn()?;
        let lines: Vec<String> = turn
            .records
            .iter()
            .filter(|e| record_in_section(&e.record, self.section))
            .filter_map(|e| serde_json::to_string(&e.record).ok())
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let title = if focused {
            "HISTORY (Tab/1-6: section · / filter · a scope · r reload · c copy · Enter expand)"
        } else {
            "HISTORY"
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused { COLOR_ACCENT } else { COLOR_BORDER }));
        let inner = block.inner(area);
        block.render(area, buf);
        // The same size gate `panel_geometry` applies, so `render` and the
        // mouse hit-tests agree on when the panel has a body at all.
        let Some(geo) = self.panel_geometry(area) else {
            return;
        };

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
        Paragraph::new(self.status_line()).render(rows[0], buf);
        Paragraph::new(legend_line(self.source.as_deref()))
            .style(Style::default().fg(COLOR_MUTED))
            .render(rows[2], buf);
        let body = geo.body;

        let Some(turn) = self.selected_turn() else {
            Paragraph::new(self.empty_message())
                .style(Style::default().fg(COLOR_MUTED))
                .wrap(Wrap { trim: false })
                .render(body, buf);
            return;
        };

        if geo.expanded {
            self.render_detail(turn, geo.detail, buf, true);
            return;
        }
        if let Some(list_area) = geo.list {
            self.render_list(list_area, buf);
        }
        self.render_detail(turn, geo.detail, buf, false);
    }

    fn status_line(&self) -> Line<'static> {
        let visible = self.visible();
        let incomplete = visible
            .iter()
            .filter(|&&i| {
                self.turns[i].attributed && self.turns[i].summary.status == TurnStatus::Incomplete
            })
            .count();
        let scope = match self.scope {
            HistoryScope::AllConversations => "all conversations",
            HistoryScope::ActiveConversation => "active conversation",
        };
        // Most important first: a narrow panel clips the tail.
        let mut spans = vec![Span::styled(
            format!("{} turns · {incomplete} incomplete", visible.len()),
            Style::default().fg(COLOR_TEXT),
        )];
        if self.skipped_lines > 0 {
            spans.push(Span::styled(
                format!(" · {} unreadable lines skipped", self.skipped_lines),
                Style::default().fg(COLOR_WARN),
            ));
        }
        if self.filter_editing || !self.filter.is_empty() {
            let cursor = if self.filter_editing { "▏" } else { "" };
            spans.push(Span::styled(
                format!(" · filter: {}{cursor}", self.filter),
                Style::default().fg(COLOR_ACCENT),
            ));
        }
        if self.loading {
            spans.push(Span::styled(
                " · loading…",
                Style::default().fg(COLOR_MUTED),
            ));
        }
        spans.push(Span::styled(
            format!(" · {scope}"),
            Style::default().fg(COLOR_MUTED),
        ));
        Line::from(spans)
    }

    fn empty_message(&self) -> String {
        if self.loading && !self.loaded {
            return "Loading history…".to_string();
        }
        if self.turns.is_empty() {
            let path = self
                .source
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| gaviero_core::history::HISTORY_DIR.to_string());
            return format!("No turns recorded yet at {path}. Send a chat prompt, then press r.");
        }
        "No turns match the current scope / filter. Esc clears the filter, a toggles scope."
            .to_string()
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let visible = self.visible();
        let capacity = (area.height as usize / ROWS_PER_TURN).max(1);
        let first = self.selected.saturating_sub(capacity.saturating_sub(1));
        let mut lines: Vec<Line> = Vec::new();
        for (pos, &i) in visible.iter().enumerate().skip(first).take(capacity) {
            lines.extend(turn_list_lines(&self.turns[i], pos == self.selected));
        }
        Paragraph::new(lines).render(area, buf);
    }

    fn render_detail(&self, turn: &TurnRecords, area: Rect, buf: &mut Buffer, expanded: bool) {
        let mut lines = if expanded {
            Vec::new()
        } else {
            detail_header_lines(turn, self.section)
        };
        let header_len = lines.len();
        lines.extend(self.section_body(turn));
        // Keep the header pinned: scroll only the section body.
        let header: Vec<Line> = lines.drain(..header_len).collect();
        let header_height = (header.len() as u16).min(area.height);
        let [head, rest] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(0)])
            .areas(area);
        Paragraph::new(header).render(head, buf);
        let scroll = self.body_scroll(&lines, area.width.max(1) as usize);
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
            .render(rest, buf);
    }

    /// The focused section's body, with the FILES cursor marked when the panel
    /// has one. Shared by the side-by-side and the expanded layouts.
    fn section_body(&self, turn: &TurnRecords) -> Vec<Line<'static>> {
        let cursor = (self.section == HistorySection::Files).then_some(self.file_selected);
        section_lines_with_cursor(turn, self.section, cursor)
    }
}

fn turn_matches(turn: &TurnRecords, needle: &str) -> bool {
    let s = &turn.summary;
    let hay = |v: &str| v.to_lowercase().contains(needle);
    s.conv_title.as_deref().is_some_and(hay)
        || s.turn_id.as_deref().is_some_and(hay)
        || turn.records.iter().any(|e| match &e.record.payload {
            HistoryKind::TurnStart(start) => hay(&start.prompt),
            HistoryKind::ToolCall(c) => hay(&c.tool),
            HistoryKind::McpCall(m) => hay(&m.tool),
            _ => false,
        })
}

fn record_in_section(record: &HistoryRecord, section: HistorySection) -> bool {
    matches!(
        (section, &record.payload),
        (HistorySection::Prompt, HistoryKind::TurnStart(_))
            | (HistorySection::Tools, HistoryKind::ToolCall(_))
            | (HistorySection::Mcp, HistoryKind::McpCall(_))
            | (HistorySection::Memory, HistoryKind::MemoryInjection(_))
            | (HistorySection::Totals, HistoryKind::TurnEnd(_))
            | (HistorySection::Files, HistoryKind::FilesChanged(_))
            | (HistorySection::Files, HistoryKind::TurnReview(_))
    )
}

fn status_glyph(status: TurnStatus) -> (&'static str, Color) {
    match status {
        TurnStatus::Complete => ("✓", COLOR_OK),
        TurnStatus::Incomplete => ("…", COLOR_MUTED),
        TurnStatus::Cancelled => ("⊘", COLOR_WARN),
        TurnStatus::Failed => ("✗", COLOR_WARN),
    }
}

/// `2026-09-15T18:43:44.512Z` → local `09-15 20:43`.
fn local_time(ts: Option<&str>) -> String {
    ts.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "--:--".to_string())
}

fn fmt_bytes(n: usize) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.1} KiB", n as f64 / 1024.0),
        _ => format!("{:.1} MiB", n as f64 / 1_048_576.0),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The footer legend: names every estimator and the exact-usage source.
pub fn legend_line(source: Option<&Path>) -> Line<'static> {
    let path = source
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ".gaviero/history/turns.ndjson".to_string());
    Line::from(format!(
        "~ est: {} (text) / {} (JSON) · exact: provider-reported usage · {path}",
        Estimator::WordsX13.label(),
        Estimator::CharsDiv4.label(),
    ))
}

/// Two lines per turn in the list.
pub fn turn_list_lines(turn: &TurnRecords, selected: bool) -> Vec<Line<'static>> {
    let s = &turn.summary;
    let marker = if selected { "›" } else { " " };
    let row_style = if selected {
        Style::default()
            .fg(COLOR_ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COLOR_TEXT)
    };
    if !turn.attributed {
        return vec![
            Line::from(Span::styled(
                format!("{marker} UNATTRIBUTED MCP calls"),
                row_style,
            )),
            Line::from(Span::styled(
                format!(
                    "    {} mcp · issued while zero or several conversations streamed",
                    s.mcp_count
                ),
                Style::default().fg(COLOR_MUTED),
            )),
        ];
    }
    let (glyph, glyph_color) = status_glyph(s.status);
    let title = s.conv_title.as_deref().unwrap_or("(untitled)");
    vec![
        Line::from(vec![
            Span::styled(format!("{marker} "), row_style),
            Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
            Span::styled(
                format!(
                    "{} {} · ~{}/~{}",
                    local_time(s.started_at.as_deref()),
                    truncate_chars(title, 24),
                    compact_count(s.in_tokens_est),
                    compact_count(s.out_tokens_est),
                ),
                row_style,
            ),
        ]),
        Line::from(Span::styled(
            format!(
                "    {} tools · {} mcp · {} mem · {} · {}",
                s.tool_count,
                s.mcp_count,
                s.mem_items,
                s.provider.as_deref().unwrap_or("?"),
                truncate_chars(&s.prompt_preview, 60),
            ),
            Style::default().fg(COLOR_MUTED),
        )),
    ]
}

/// The pinned header: turn identity plus one summary line per section, the
/// focused one marked.
pub fn detail_header_lines(turn: &TurnRecords, focused: HistorySection) -> Vec<Line<'static>> {
    let s = &turn.summary;
    let mut lines = Vec::with_capacity(7);
    if turn.attributed {
        let (glyph, color) = status_glyph(s.status);
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "turn {} · {} · ",
                    s.turn_id.as_deref().unwrap_or("?"),
                    s.model.as_deref().unwrap_or("?"),
                ),
                Style::default().fg(COLOR_MUTED),
            ),
            Span::styled(
                format!("{glyph} {}", s.status.label()),
                Style::default().fg(color),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "UNATTRIBUTED — MCP calls with no single streaming conversation",
            Style::default().fg(COLOR_WARN),
        )));
    }

    for section in HistorySection::ALL {
        let text = match section {
            HistorySection::Prompt => format!(
                "{} · ~{} tok",
                fmt_bytes(s.prompt_bytes),
                compact_count(s.in_tokens_est)
            ),
            HistorySection::Tools => {
                if s.tool_count == 0 {
                    tools_absent_reason(s.provider.as_deref()).to_string()
                } else {
                    format!(
                        "{} calls{} · ~{} in / ~{} out",
                        s.tool_count,
                        if s.summary_only_tools > 0 {
                            format!(" ({} summary-only)", s.summary_only_tools)
                        } else {
                            String::new()
                        },
                        compact_count(s.tool_in_tokens_est),
                        compact_count(s.tool_out_tokens_est),
                    )
                }
            }
            HistorySection::Mcp => format!(
                "{} calls · ~{} in / ~{} out",
                s.mcp_count,
                compact_count(s.mcp_in_tokens_est),
                compact_count(s.mcp_out_tokens_est),
            ),
            HistorySection::Memory => {
                match turn.records.iter().find_map(|e| match &e.record.payload {
                    HistoryKind::MemoryInjection(m) => Some(m),
                    _ => None,
                }) {
                    Some(m) => format!(
                        "{}/{} items · ~{}/{} tok",
                        m.items_injected,
                        m.pool_size,
                        compact_count(m.tokens_used_est),
                        compact_count(m.token_budget),
                    ),
                    None => "no memory call this turn".to_string(),
                }
            }
            HistorySection::Totals => {
                let boot = s
                    .bootstrap_tokens_est
                    .map(|b| format!("~{} boot", compact_count(b)))
                    .unwrap_or_else(|| "no bootstrap".to_string());
                match s.exact_usage {
                    Some(u) => format!(
                        "{boot} · exact {} in / {} out",
                        grouped_count(u.prefix_tokens()),
                        grouped_count(u.output_tokens)
                    ),
                    None => format!("{boot} · no exact usage"),
                }
            }
            HistorySection::Files => {
                let reviewed = turn
                    .records
                    .iter()
                    .any(|e| matches!(e.record.payload, HistoryKind::TurnReview(_)));
                match s.files_changed {
                    0 => "no file changes".to_string(),
                    n => format!(
                        "{n} file{} changed · {}",
                        if n == 1 { "" } else { "s" },
                        if reviewed {
                            "reviewed"
                        } else {
                            "review pending"
                        }
                    ),
                }
            }
        };
        let is_focused = section == focused;
        let style = if is_focused {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(COLOR_TEXT)
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} {:<7} {}",
                if is_focused { "▸" } else { " " },
                section.index() + 1,
                section.label(),
                text
            ),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(COLOR_BORDER),
    )));
    lines
}

fn tools_absent_reason(provider: Option<&str>) -> &'static str {
    match provider {
        Some("codex") => "tools: not reported by provider (codex)",
        Some("ollama") => "tools: not reported by provider (ollama streams text only)",
        _ => "no tool calls this turn",
    }
}

/// The focused section's body, with the FILES list's cursor marked (the panel
/// passes its own `file_selected`; callers with no cursor pass `None`).
pub fn section_lines_with_cursor(
    turn: &TurnRecords,
    section: HistorySection,
    file_selected: Option<usize>,
) -> Vec<Line<'static>> {
    section_lines_with_rows(turn, section, file_selected, &mut Vec::new())
}

/// [`section_lines_with_cursor`], additionally reporting the body-line index of
/// every FILES row into `rows` — the map that lets a mouse row be turned back
/// into a file (see [`HistoryPanelState::hit_test_file`]). Both functions build
/// the same lines in the same order, so the indices always agree with what
/// [`HistoryPanelState::render_detail`] drew.
pub fn section_lines_with_rows(
    turn: &TurnRecords,
    section: HistorySection,
    file_selected: Option<usize>,
    rows: &mut Vec<usize>,
) -> Vec<Line<'static>> {
    let provider = turn
        .summary
        .provider
        .clone()
        .unwrap_or_else(|| "provider".to_string());
    let mut lines: Vec<Line<'static>> = Vec::new();
    let heading = |text: String| {
        Line::from(Span::styled(
            text,
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let muted = |text: String| Line::from(Span::styled(text, Style::default().fg(COLOR_MUTED)));

    match section {
        HistorySection::Prompt => match find_start(turn) {
            Some(start) => prompt_lines(start, &mut lines),
            None if !turn.attributed => {
                lines.push(muted("Unattributed records carry no prompt.".into()))
            }
            None => lines.push(muted(
                "No turn_start record (the log starts mid-turn, e.g. after rotation).".into(),
            )),
        },
        HistorySection::Tools => {
            let calls: Vec<(u32, &ToolCall)> = turn
                .records
                .iter()
                .filter_map(|e| match &e.record.payload {
                    HistoryKind::ToolCall(c) => Some((e.record.seq, c)),
                    _ => None,
                })
                .collect();
            if calls.is_empty() {
                lines.push(muted(tools_absent_reason(Some(&provider)).to_string()));
            }
            for (seq, call) in calls {
                tool_lines(seq, call, &provider, &mut lines, &heading, &muted);
            }
        }
        HistorySection::Mcp => {
            let calls: Vec<(u32, &McpCall)> = turn
                .records
                .iter()
                .filter_map(|e| match &e.record.payload {
                    HistoryKind::McpCall(m) => Some((e.record.seq, m)),
                    _ => None,
                })
                .collect();
            if calls.is_empty() {
                lines.push(muted("No MCP calls attributed to this turn.".into()));
            }
            for (seq, call) in calls {
                mcp_lines(seq, call, &mut lines, &heading, &muted);
            }
        }
        HistorySection::Memory => {
            let mut any = false;
            for e in &turn.records {
                if let HistoryKind::MemoryInjection(m) = &e.record.payload {
                    any = true;
                    memory_lines(m, &mut lines, &heading, &muted);
                }
            }
            if !any {
                lines.push(muted(
                    "No memory call this turn (memory is injected on bootstrap turns only).".into(),
                ));
            }
        }
        HistorySection::Totals => totals_lines(turn, &provider, &mut lines, &heading, &muted),
        HistorySection::Files => {
            files_lines(turn, file_selected, rows, &mut lines, &heading, &muted)
        }
    }
    lines
}

/// FILES: what the turn changed on disk, in the section's own order —
/// **modified, added, deleted** — with the review decision per file and the
/// panel's cursor. `selected` is the index into [`ordered_files`].
///
/// Each file row's body-line index is pushed into `rows` (its position in
/// `lines`), which is what makes the list clickable.
fn files_lines(
    turn: &TurnRecords,
    selected: Option<usize>,
    rows: &mut Vec<usize>,
    lines: &mut Vec<Line<'static>>,
    heading: &dyn Fn(String) -> Line<'static>,
    muted: &dyn Fn(String) -> Line<'static>,
) {
    let review = turn.records.iter().find_map(|e| match &e.record.payload {
        HistoryKind::TurnReview(r) => Some(r),
        _ => None,
    });
    let Some(changed) = files_changed(turn) else {
        lines.push(muted(
            "No file changes recorded for this turn (or turn capture was off).".into(),
        ));
        return;
    };
    let files = ordered_files(turn);
    let count = |kind: &str| files.iter().filter(|f| f.change == kind).count();
    lines.push(heading(format!(
        "{} file(s) changed · {} · {} modified / {} added / {} deleted",
        files.len(),
        changed.outcome,
        count("modified"),
        count("added"),
        count("deleted"),
    )));
    if selected.is_some() && !files.is_empty() {
        lines.push(muted(
            "j/k pick a file · Enter: the whole file with its diffs · [ ] previous/next turn"
                .into(),
        ));
    }
    let mut group = "";
    for (i, f) in files.iter().enumerate() {
        if f.change != group {
            group = f.change.as_str();
            lines.push(Line::from(Span::styled(
                format!("{} ({})", group.to_uppercase(), count(group)),
                Style::default()
                    .fg(COLOR_MUTED)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let decision = review
            .and_then(|r| r.decisions.iter().find(|d| d.path == f.path))
            .map(|d| match &d.detail {
                Some(detail) => format!("{} ({detail})", d.result),
                None => d.result.clone(),
            })
            .unwrap_or_else(|| "pending".to_string());
        let mut flags = String::new();
        if !f.overlap_with.is_empty() {
            flags.push_str(&format!("  overlap: {}", f.overlap_with.join(", ")));
        }
        if !f.revertible {
            flags.push_str("  not revertible");
        }
        let (glyph, glyph_color) = change_glyph(&f.change);
        let is_cursor = selected == Some(i);
        let row = if is_cursor {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(COLOR_TEXT)
        };
        rows.push(lines.len());
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} [{i:>2}] ", if is_cursor { "›" } else { " " }),
                row,
            ),
            Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
            Span::styled(format!("{:<9}", f.change), Style::default().fg(COLOR_MUTED)),
            Span::styled(f.path.clone(), row),
            Span::styled(
                format!("  → {decision}{flags}"),
                Style::default().fg(COLOR_MUTED),
            ),
        ]));
    }
    for p in &changed.auto_reverted {
        lines.push(Line::from(Span::styled(
            format!("  sensitive  {p}  → auto-reverted"),
            Style::default().fg(COLOR_WARN),
        )));
    }
    if !changed.between_turns.is_empty() {
        lines.push(muted(format!(
            "Changed outside any turn before this one: {}",
            changed.between_turns.join(", ")
        )));
    }
    for w in &changed.warnings {
        lines.push(Line::from(Span::styled(
            format!("⚠ {w}"),
            Style::default().fg(COLOR_WARN),
        )));
    }
}

/// How many display rows `line` needs at `width`.
///
/// The section body is drawn with `Wrap { trim: false }`, and ratatui applies
/// `Paragraph::scroll` *after* wrapping — so the scroll offset counts wrapped
/// rows, not logical lines. The mouse hit-tests have to count them the same way.
fn wrapped_height(line: &Line<'static>, width: usize) -> usize {
    let text = line.to_string();
    if text.is_empty() {
        return 1;
    }
    crate::widgets::render_utils::word_wrap(&text, width)
        .len()
        .max(1)
}

fn find_start(turn: &TurnRecords) -> Option<&TurnStart> {
    turn.records.iter().find_map(|e| match &e.record.payload {
        HistoryKind::TurnStart(s) => Some(s),
        _ => None,
    })
}

fn find_end(turn: &TurnRecords) -> Option<&TurnEnd> {
    turn.records.iter().find_map(|e| match &e.record.payload {
        HistoryKind::TurnEnd(end) => Some(end),
        _ => None,
    })
}

fn text_lines(text: &str, style: Style, out: &mut Vec<Line<'static>>) {
    for line in text.lines() {
        out.push(Line::from(Span::styled(line.to_string(), style)));
    }
    if text.is_empty() {
        out.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(COLOR_MUTED),
        )));
    }
}

fn json_lines(value: &serde_json::Value, out: &mut Vec<Line<'static>>) {
    let text = match value {
        // A capped payload is stored as the head of its serialization.
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    text_lines(&text, Style::default().fg(COLOR_TEXT), out);
}

fn prompt_lines(start: &TurnStart, out: &mut Vec<Line<'static>>) {
    out.push(Line::from(Span::styled(
        format!(
            "{} · ~{} tok ({}) · {}",
            fmt_bytes(start.prompt_bytes),
            compact_count(start.input_tokens_est.unwrap_or(0)),
            start.estimator.unwrap_or(Estimator::WordsX13).label(),
            start.workspace_root,
        ),
        Style::default().fg(COLOR_MUTED),
    )));
    text_lines(&start.prompt, Style::default().fg(COLOR_TEXT), out);
    if start.prompt_truncated {
        out.push(Line::from(Span::styled(
            format!(
                "── truncated: stored {} of {} ──",
                fmt_bytes(start.prompt.len()),
                fmt_bytes(start.prompt_bytes)
            ),
            Style::default().fg(COLOR_WARN),
        )));
    }
}

fn tool_lines(
    seq: u32,
    call: &ToolCall,
    provider: &str,
    out: &mut Vec<Line<'static>>,
    heading: &dyn Fn(String) -> Line<'static>,
    muted: &dyn Fn(String) -> Line<'static>,
) {
    let est = match (call.input_tokens_est, call.output_tokens_est) {
        (None, None) => String::new(),
        (i, o) => format!(
            " · ~{}/~{} ({})",
            i.map(compact_count).unwrap_or_else(|| "-".into()),
            o.map(compact_count).unwrap_or_else(|| "-".into()),
            call.estimator.unwrap_or(Estimator::CharsDiv4).label(),
        ),
    };
    out.push(heading(format!(
        "#{seq} {}{}{}{}",
        call.tool,
        call.tool_use_id
            .as_deref()
            .map(|id| format!(" · {id}"))
            .unwrap_or_default(),
        call.duration_ms
            .map(|ms| format!(" · {ms} ms"))
            .unwrap_or_default(),
        est,
    )));
    if let Some(summary) = &call.summary {
        out.push(muted(format!("summary: {summary}")));
    }
    if call.capture == CaptureMode::SummaryOnly {
        out.push(muted(format!(
            "summary only — {provider} did not expose arguments or result"
        )));
    }
    if let Some(input) = &call.input {
        out.push(muted(if call.input_truncated {
            "input (truncated):".into()
        } else {
            "input:".into()
        }));
        json_lines(input, out);
    }
    match &call.output {
        ToolOutput::Full { content, is_error } => {
            out.push(muted(match (*is_error, call.output_truncated) {
                (true, true) => "result (error, truncated):".into(),
                (true, false) => "result (error):".into(),
                (false, true) => "result (truncated):".into(),
                (false, false) => "result:".into(),
            }));
            let style = Style::default().fg(if *is_error { COLOR_WARN } else { COLOR_TEXT });
            text_lines(content, style, out);
        }
        ToolOutput::Summary { text } => {
            out.push(muted("result (summary):".into()));
            text_lines(text, Style::default().fg(COLOR_TEXT), out);
        }
        ToolOutput::None if call.capture == CaptureMode::Full => {
            out.push(muted(
                "no result — the call was still running when the turn ended".into(),
            ));
        }
        ToolOutput::None => {}
    }
    out.push(Line::from(""));
}

fn mcp_lines(
    seq: u32,
    call: &McpCall,
    out: &mut Vec<Line<'static>>,
    heading: &dyn Fn(String) -> Line<'static>,
    muted: &dyn Fn(String) -> Line<'static>,
) {
    out.push(heading(format!(
        "#{seq} {} · {:.1} ms · ~{}/~{} ({}){}{}",
        call.tool,
        call.duration_us as f64 / 1000.0,
        compact_count(call.input_tokens_est),
        compact_count(call.output_tokens_est),
        call.estimator.label(),
        if call.empty_result {
            " · empty result"
        } else {
            ""
        },
        match call.attribution {
            gaviero_core::history::Attribution::TurnInferred => " · attributed by inference",
            gaviero_core::history::Attribution::Unattributed => " · unattributed",
        },
    )));
    if let Some(err) = &call.error {
        out.push(Line::from(Span::styled(
            format!("error: {err}"),
            Style::default().fg(COLOR_WARN),
        )));
    }
    out.push(muted(if call.input_truncated {
        "request (truncated):".into()
    } else {
        "request:".into()
    }));
    json_lines(&call.input, out);
    out.push(muted(if call.output_truncated {
        "response (truncated):".into()
    } else {
        "response:".into()
    }));
    json_lines(&call.output, out);
    out.push(Line::from(""));
}

fn memory_lines(
    m: &MemoryInjection,
    out: &mut Vec<Line<'static>>,
    heading: &dyn Fn(String) -> Line<'static>,
    muted: &dyn Fn(String) -> Line<'static>,
) {
    out.push(heading(format!(
        "{} of {} candidates injected · ~{} of {} tok budget ({})",
        m.items_injected,
        m.pool_size,
        compact_count(m.tokens_used_est),
        compact_count(m.token_budget),
        m.estimator.label(),
    )));
    match &m.manifest {
        Some(serde_json::Value::Object(manifest)) => {
            let get = |k: &str| manifest.get(k);
            if let Some(q) = get("query_text").and_then(|v| v.as_str()) {
                out.push(muted(format!("query: {}", truncate_chars(q, 200))));
            }
            if let Some(ids) = get("selected_ids") {
                out.push(muted(format!("selected ids: {ids}")));
            }
            if let Some(serde_json::Value::Array(dist)) = get("scope_distribution") {
                let parts: Vec<String> = dist
                    .iter()
                    .map(|d| {
                        format!(
                            "{} {}/{}",
                            d.get("scope_label").and_then(|v| v.as_str()).unwrap_or("?"),
                            d.get("count_selected")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            d.get("count_in_pool").and_then(|v| v.as_u64()).unwrap_or(0),
                        )
                    })
                    .collect();
                out.push(muted(format!(
                    "scopes (selected/pool): {}",
                    parts.join(", ")
                )));
            }
            out.push(muted(format!(
                "embedder: {} · reranker: {} · scoring: {}",
                get("embedder_name").and_then(|v| v.as_str()).unwrap_or("?"),
                get("reranker_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none"),
                get("scoring_formula_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
            )));
            if let Some(n) = get("candidate_pool_omitted") {
                out.push(muted(format!(
                    "candidate pool omitted from the log ({n} entries)"
                )));
            }
        }
        Some(other) => {
            out.push(muted("manifest (truncated):".into()));
            json_lines(other, out);
        }
        None => out.push(muted(
            "no manifest captured (manifests disabled, or nothing injected)".into(),
        )),
    }
    match &m.response_block {
        Some(block) => {
            out.push(muted(if m.response_block_truncated {
                "response (injected block, truncated):".into()
            } else {
                "response (injected block):".into()
            }));
            text_lines(block, Style::default().fg(COLOR_TEXT), out);
        }
        None => out.push(muted("response: nothing injected".into())),
    }
    out.push(Line::from(""));
}

fn totals_lines(
    turn: &TurnRecords,
    provider: &str,
    out: &mut Vec<Line<'static>>,
    heading: &dyn Fn(String) -> Line<'static>,
    muted: &dyn Fn(String) -> Line<'static>,
) {
    let s = &turn.summary;
    let words = Estimator::WordsX13.label();
    let chars = Estimator::CharsDiv4.label();
    out.push(heading("Estimates".into()));
    out.push(Line::from(format!(
        "prompt              ~{} tok ({words})",
        compact_count(s.in_tokens_est)
    )));
    out.push(Line::from(format!(
        "tool args / results ~{} / ~{} tok ({chars})",
        compact_count(s.tool_in_tokens_est),
        compact_count(s.tool_out_tokens_est)
    )));
    out.push(Line::from(format!(
        "mcp requests / resp ~{} / ~{} tok ({chars})",
        compact_count(s.mcp_in_tokens_est),
        compact_count(s.mcp_out_tokens_est)
    )));
    out.push(Line::from(format!(
        "memory injected     ~{} tok ({words}, measured by the injection)",
        compact_count(s.memory_tokens_est)
    )));
    out.push(Line::from(match s.bootstrap_tokens_est {
        Some(b) => format!(
            "bootstrap context   ~{} tok (graph + memory + file refs)",
            compact_count(b)
        ),
        None => "bootstrap context   none this turn".to_string(),
    }));
    out.push(Line::from(format!(
        "assistant output    ~{} tok ({words})",
        compact_count(s.out_tokens_est)
    )));
    out.push(Line::from(""));

    out.push(heading("Provider-reported (exact)".into()));
    match s.exact_usage {
        Some(u) => {
            out.push(Line::from(format!(
                "input (fresh)       {}",
                grouped_count(u.input_tokens)
            )));
            out.push(Line::from(format!(
                "cache creation      {}",
                grouped_count(u.cache_creation_input_tokens)
            )));
            out.push(Line::from(format!(
                "cache read          {}",
                grouped_count(u.cache_read_input_tokens)
            )));
            out.push(Line::from(format!(
                "output              {}",
                grouped_count(u.output_tokens)
            )));
            out.push(Line::from(format!(
                "context window used {} (input incl. cache, last iteration)",
                grouped_count(u.prefix_tokens())
            )));
        }
        None => out.push(muted(format!("no provider usage reported by {provider}"))),
    }
    out.push(Line::from(""));

    out.push(heading("Outcome".into()));
    match find_end(turn) {
        Some(end) => {
            out.push(Line::from(format!(
                "{} · {} proposal(s) · {} of assistant output",
                s.status.label(),
                end.proposal_count,
                fmt_bytes(end.assistant_bytes),
            )));
            if let Some(err) = &end.error {
                out.push(Line::from(Span::styled(
                    format!("error: {err}"),
                    Style::default().fg(COLOR_WARN),
                )));
            }
            if let Some(excerpt) = &end.assistant_excerpt {
                out.push(muted(if end.assistant_truncated {
                    "assistant output (head):".into()
                } else {
                    "assistant output:".into()
                }));
                text_lines(excerpt, Style::default().fg(COLOR_TEXT), out);
            }
        }
        None => out.push(muted(
            "incomplete — no turn_end (still streaming, or the process exited mid-turn)".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::history::{HistoryRecorder, ProviderUsage, TurnEnd, tool_call_record};

    fn start(prompt: &str, title: &str) -> TurnStart {
        TurnStart {
            provider: "claude".into(),
            model: "claude:sonnet".into(),
            conv_title: Some(title.into()),
            workspace_root: "C:/w".into(),
            prompt: prompt.into(),
            prompt_bytes: 0,
            prompt_truncated: false,
            input_tokens_est: None,
            estimator: None,
        }
    }

    /// Two turns in two conversations: one full, one cancelled.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let r = HistoryRecorder::with_path_and_cap(path.clone(), 1 << 20);
        r.begin_turn(
            "c1",
            "c1-1",
            start("explain the token estimator", "estimator"),
            false,
        );
        r.tool_started("c1-1", "Read src/tokens.rs");
        r.tool_completed(
            "c1-1",
            tool_call_record(
                "Read",
                Some("toolu_1".into()),
                Some(8),
                Some(serde_json::json!({"file_path": "src/tokens.rs"})),
                ToolOutput::Full {
                    content: "pub fn estimate()".into(),
                    is_error: false,
                },
                Some("Read src/tokens.rs".into()),
            ),
        );
        r.push(
            "c1-1",
            HistoryKind::MemoryInjection(gaviero_core::history::memory_injection_record(
                2,
                9,
                120,
                2000,
                Some("<project_memory>\n- estimator is words×1.3\n</project_memory>".into()),
                Some(serde_json::json!({
                    "query_text": "token estimator",
                    "selected_ids": [4, 7],
                    "embedder_name": "nomic",
                })),
            )),
        );
        r.note_usage(
            "c1-1",
            ProviderUsage {
                input_tokens: 12,
                cache_creation_input_tokens: 800,
                cache_read_input_tokens: 31_000,
                output_tokens: 1_204,
            },
        );
        r.note_assistant_output("c1-1", "It multiplies words by 1.3.");
        r.end_turn("c1-1", TurnEnd::new(false, None, 0));
        r.begin_turn("c2", "c2-1", start("second conversation", "other"), false);
        r.end_turn("c2-1", TurnEnd::new(true, None, 0));
        // A malformed tail line.
        let mut body = std::fs::read_to_string(&path).unwrap();
        body.push_str("{\"v\":1,\"kind\":\"tool_call\"\n");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    fn loaded() -> (tempfile::TempDir, HistoryPanelState) {
        let (dir, path) = fixture();
        let mut state = HistoryPanelState::new();
        state.apply_load(load_history(&path));
        (dir, state)
    }

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn loader_counts_malformed_lines_instead_of_failing() {
        let (_dir, path) = fixture();
        let load = load_history(&path);
        assert_eq!(load.turns.len(), 2);
        assert_eq!(load.skipped, 1);
    }

    #[test]
    fn missing_log_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let load = load_history(&dir.path().join("absent.ndjson"));
        assert!(load.turns.is_empty());
        assert_eq!(load.skipped, 0);
    }

    #[test]
    fn list_is_newest_first_and_scope_narrows_to_the_active_conversation() {
        let (_dir, mut state) = loaded();
        let ids: Vec<_> = state
            .visible()
            .iter()
            .map(|&i| state.turns[i].turn_id.clone().unwrap())
            .collect();
        assert_eq!(ids, vec!["c2-1", "c1-1"]);

        state.active_conv = Some("c1".into());
        state.toggle_scope();
        assert_eq!(state.visible().len(), 1);
        assert_eq!(
            state.selected_turn().unwrap().turn_id.as_deref(),
            Some("c1-1")
        );
    }

    #[test]
    fn filter_matches_prompt_text_title_and_tool_names() {
        let (_dir, mut state) = loaded();
        state.filter = "ESTIMATOR".into();
        assert_eq!(state.visible().len(), 1);
        state.filter = "read".into(); // tool name
        assert_eq!(state.visible().len(), 1);
        state.filter = "other".into(); // conversation title
        assert_eq!(
            state.selected_turn().unwrap().turn_id.as_deref(),
            Some("c2-1")
        );
        state.filter = "no such thing".into();
        assert!(state.visible().is_empty());
        assert!(state.escape());
        assert!(state.filter.is_empty());
        assert!(!state.escape());
    }

    #[test]
    fn sections_cycle_and_jump() {
        assert_eq!(HistorySection::Totals.next(), HistorySection::Files);
        assert_eq!(HistorySection::Files.next(), HistorySection::Prompt);
        assert_eq!(HistorySection::Prompt.prev(), HistorySection::Files);
        assert_eq!(HistorySection::from_digit('3'), Some(HistorySection::Mcp));
        assert_eq!(HistorySection::from_digit('6'), Some(HistorySection::Files));
        assert_eq!(HistorySection::from_digit('7'), None);
        let mut state = HistoryPanelState::new();
        state.section_scroll = 9;
        state.set_section(HistorySection::Tools);
        assert_eq!(state.section_scroll, 0);
    }

    #[test]
    fn reload_keeps_the_selected_turn() {
        let (dir, mut state) = loaded();
        state.select_next(); // c1-1
        state.apply_load(load_history(&dir.path().join("turns.ndjson")));
        assert_eq!(
            state.selected_turn().unwrap().turn_id.as_deref(),
            Some("c1-1")
        );
    }

    #[test]
    fn copy_yields_the_focused_sections_records_as_ndjson() {
        let (_dir, mut state) = loaded();
        state.select_next(); // c1-1
        state.set_section(HistorySection::Tools);
        let ndjson = state.focused_records_ndjson().unwrap();
        assert_eq!(ndjson.lines().count(), 1);
        assert!(ndjson.contains("\"kind\":\"tool_call\""));
        state.set_section(HistorySection::Mcp);
        assert!(state.focused_records_ndjson().is_none());
    }

    #[test]
    fn sections_show_payloads_with_labelled_numbers() {
        let (_dir, mut state) = loaded();
        state.select_next();
        let turn = state.selected_turn().unwrap();
        let text = |section| {
            section_lines_with_cursor(turn, section, None)
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let tools = text(HistorySection::Tools);
        assert!(tools.contains("src/tokens.rs"));
        assert!(tools.contains("pub fn estimate()"));
        assert!(tools.contains("chars÷4"));
        let memory = text(HistorySection::Memory);
        assert!(memory.contains("query: token estimator"));
        assert!(memory.contains("<project_memory>"));
        let totals = text(HistorySection::Totals);
        assert!(
            totals.contains("31,812"),
            "context window = prefix tokens: {totals}"
        );
        assert!(totals.contains("1,204"));
        assert!(text(HistorySection::Mcp).contains("No MCP calls"));
        assert!(text(HistorySection::Prompt).contains("explain the token estimator"));
    }

    #[test]
    fn render_smoke_shows_list_detail_and_legend() {
        let (_dir, mut state) = loaded();
        state.select_next();
        for width in [60u16, 120] {
            let area = Rect::new(0, 0, width, 30);
            let mut buf = Buffer::empty(area);
            state.render(area, &mut buf, true);
            let text = buffer_text(&buf);
            assert!(text.contains("HISTORY"), "{text}");
            assert!(text.contains("estimator"), "{text}");
            assert!(text.contains("PROMPT"), "{text}");
            assert!(text.contains("TOTALS"), "{text}");
            assert!(text.contains("~ est:"), "{text}");
            assert!(text.contains("unreadable lines skipped"), "{text}");
        }
    }

    #[test]
    fn render_without_turns_explains_the_empty_log() {
        let mut state = HistoryPanelState::new();
        state.apply_load(HistoryLoad {
            source: PathBuf::from("C:/w/.gaviero/history/turns.ndjson"),
            turns: Vec::new(),
            skipped: 0,
        });
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        state.render(area, &mut buf, false);
        assert!(buffer_text(&buf).contains("No turns recorded yet"));
    }

    // ── FILES section: order, cursor, whole-file diff ──────────────────────

    /// A closed turn whose only interesting record is its `files_changed`.
    fn files_state(files: Vec<ChangedFile>) -> (tempfile::TempDir, HistoryPanelState) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let r = HistoryRecorder::with_path_and_cap(path.clone(), 1 << 20);
        r.begin_turn("c1", "c1-1", start("edit the files", "files"), false);
        r.push(
            "c1-1",
            HistoryKind::FilesChanged(FilesChanged {
                outcome: "completed".into(),
                files,
                auto_reverted: Vec::new(),
                between_turns: Vec::new(),
                warnings: Vec::new(),
            }),
        );
        r.end_turn("c1-1", TurnEnd::new(false, None, 0));
        let mut state = HistoryPanelState::new();
        state.apply_load(load_history(&path));
        state.set_section(HistorySection::Files);
        (dir, state)
    }

    fn changed(path: &str, change: &str) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            change: change.into(),
            before_sha256: None,
            after_sha256: None,
            revertible: true,
            overlap_with: Vec::new(),
        }
    }

    fn section_text(turn: &TurnRecords, selected: Option<usize>) -> String {
        section_lines_with_cursor(turn, HistorySection::Files, selected)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn files_are_ordered_modified_then_added_then_deleted() {
        let (_dir, state) = files_state(vec![
            changed("z.rs", "added"),
            changed("b.rs", "deleted"),
            changed("m.rs", "modified"),
            changed("a.rs", "added"),
            changed("c.rs", "modified"),
            changed("a-gone.rs", "deleted"),
        ]);
        let ordered: Vec<(&str, &str)> = state
            .files()
            .iter()
            .map(|f| (f.change.as_str(), f.path.as_str()))
            .collect();
        assert_eq!(
            ordered,
            vec![
                ("modified", "c.rs"),
                ("modified", "m.rs"),
                ("added", "a.rs"),
                ("added", "z.rs"),
                ("deleted", "a-gone.rs"),
                ("deleted", "b.rs"),
            ]
        );
        // The record itself keeps whatever order capture wrote; only the view sorts.
        assert_eq!(state.file_count(), 6);
    }

    #[test]
    fn file_cursor_moves_pages_and_clamps() {
        let (_dir, mut state) = files_state(vec![
            changed("m1.rs", "modified"),
            changed("m2.rs", "modified"),
            changed("a1.rs", "added"),
        ]);
        assert_eq!(state.file_count(), 3);
        assert_eq!(state.selected_file().unwrap().path, "m1.rs");

        state.select_file_next();
        assert_eq!(state.selected_file().unwrap().path, "m2.rs");
        state.select_file_by(FILE_PAGE);
        assert_eq!(state.file_selected, 2);
        state.select_file_next();
        assert_eq!(state.file_selected, 2, "clamped at the end");

        state.select_file_prev();
        assert_eq!(state.file_selected, 1);
        state.select_file_by(-FILE_PAGE);
        assert_eq!(state.file_selected, 0);
        state.select_file_prev();
        assert_eq!(state.file_selected, 0, "clamped at the start");

        state.select_file_to(true);
        assert_eq!(state.file_selected, 2);
        state.select_file_to(false);
        assert_eq!(state.file_selected, 0);

        // Turning to another turn drops the cursor: another turn's files are a
        // different list.
        state.file_selected = 2;
        state.reset_file_cursor();
        assert_eq!(state.file_selected, 0);

        // No files at all: every movement is a no-op rather than a panic.
        let (_dir, mut empty) = files_state(Vec::new());
        assert_eq!(empty.file_count(), 0);
        assert!(empty.selected_file().is_none());
        empty.select_file_next();
        empty.select_file_prev();
        empty.select_file_by(FILE_PAGE);
        empty.select_file_by(-FILE_PAGE);
        empty.select_file_to(true);
        assert_eq!(empty.file_selected, 0);
    }

    #[test]
    fn files_section_groups_the_three_kinds_and_hints_at_enter() {
        let (_dir, state) = files_state(vec![
            changed("b.rs", "deleted"),
            changed("a.rs", "added"),
            changed("m.rs", "modified"),
        ]);
        let turn = state.selected_turn().unwrap();
        let text = section_text(turn, Some(1));
        assert!(
            text.contains("3 file(s) changed · completed · 1 modified / 1 added / 1 deleted"),
            "{text}"
        );
        assert!(
            text.contains("j/k pick a file · Enter: the whole file with its diffs"),
            "{text}"
        );
        assert!(text.contains("› [ 1]"), "{text}");
        assert!(text.contains("M modified m.rs"), "{text}");
        let modified = text.find("MODIFIED (1)").expect(&text);
        let added = text.find("ADDED (1)").expect(&text);
        let deleted = text.find("DELETED (1)").expect(&text);
        assert!(modified < added && added < deleted, "{text}");

        // No cursor (the expanded body renders through the same path): no hint,
        // and no row is marked.
        let plain = section_text(turn, None);
        assert!(!plain.contains("j/k pick a file"), "{plain}");
        assert!(!plain.contains("› ["), "{plain}");

        // A turn capture recorded nothing about: say so rather than render an
        // empty list.
        let (_dir, no_capture) = loaded();
        let turn = no_capture.selected_turn().unwrap();
        let text = section_text(turn, Some(0));
        assert!(text.contains("No file changes recorded"), "{text}");
    }

    /// The panel renders the file list and nothing else: the diff `Enter` opens
    /// is a buffer tab owned by the editor, so no diff rows appear here.
    #[test]
    fn render_shows_the_expanded_file_list_with_the_enter_hint() {
        let (_dir, mut state) = files_state(vec![
            changed("b.rs", "deleted"),
            changed("a.rs", "added"),
            changed("m.rs", "modified"),
        ]);
        state.expanded = true;
        state.file_selected = 2;
        let area = Rect::new(0, 0, 100, 16);
        let mut buf = Buffer::empty(area);
        state.render(area, &mut buf, true);
        let text = buffer_text(&buf);
        // The ordinary panel title: no diff mode of its own.
        assert!(text.contains("HISTORY (Tab/1-6: section"), "{text}");
        assert!(text.contains("MODIFIED (1)"), "{text}");
        assert!(text.contains("ADDED (1)"), "{text}");
        assert!(text.contains("DELETED (1)"), "{text}");
        // The cursor sits on the last file, and the hint says what Enter does.
        // (`change` is padded to 9 columns so the paths line up under their
        // kind heading, hence the run of spaces after `deleted`.)
        assert!(text.contains("› [ 2] D deleted  b.rs"), "{text}");
        assert!(
            text.contains("Enter: the whole file with its diffs"),
            "{text}"
        );
        // No diff gutter: the panel is not a diff renderer any more.
        assert!(!text.contains(" - │ "), "{text}");
        assert!(!text.contains("file diff"), "{text}");
    }

    #[test]
    fn escape_collapses_the_expanded_view_then_the_filter() {
        let (_dir, mut state) = files_state(vec![changed("m.rs", "modified")]);
        state.expanded = true;
        assert!(state.escape());
        assert!(!state.expanded, "Esc collapses the expanded view first");

        // The rest of the ladder is unchanged: filter editing, then the filter.
        state.filter_editing = true;
        assert!(state.escape());
        assert!(!state.filter_editing);
        state.filter = "zzz".into();
        assert!(state.escape());
        assert!(state.filter.is_empty());
        assert!(!state.escape(), "nothing left to close");
    }
}
