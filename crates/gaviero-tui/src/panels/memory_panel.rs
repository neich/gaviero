//! TUI memory panel (Tier A / A4).
//!
//! Four stacked sections, vertically independent, `Tab` cycles focus:
//!
//! 1. **Injected Now** — the memories from the most recent chat turn's
//!    `injection_manifests` row. Updated live via
//!    `ManifestObserver::on_manifest_persisted`. `i` opens the full
//!    candidate pool; `h` walks previous turns (within the 30-day
//!    retention window).
//! 2. **Recently Written** — memories whose `created_at` falls within the
//!    configurable `ui.memoryPanel.recentWindowHours` window (default
//!    24h). Populated by `MemoryObserver::on_write_committed`.
//!    Destructive actions (`d` delete, `p` pin, `s` change scope, `e`
//!    edit text) enqueue `WriterMessage::PanelEdit` variants.
//! 3. **Scope Summary** — aggregate counts + last-write timestamps per
//!    scope level. Pure SQL.
//! 4. **Search** — `/` activates a live fuzzy search; top-5 hits via
//!    the same `MemoryStore::search_scoped` path chat injection uses.
//!
//! Lock / event discipline:
//! * All observer callbacks fan out through `Event::Memory*` into the
//!   single TUI event loop — no background task mutates `App` directly.
//! * All destructive ops enqueue `WriterMessage::PanelEdit` and await a
//!   500ms oneshot ack; the panel renders a pending spinner until the
//!   ack fires.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use gaviero_core::memory::store::{InjectionManifestRow, SessionLedgerTurn};
use gaviero_core::memory::{MemoryKind, MemorySource, ScoredMemory};

/// C2.6: lightweight projection of a single `deletions` audit row for
/// the Deletions tab. Only the columns the panel actually renders.
#[derive(Debug, Clone)]
#[allow(dead_code)] // projection populated from DB rows; some fields surface only in future panel views
pub struct DeletionRow {
    pub id: i64,
    pub memory_id: i64,
    pub memory_kind: String,
    pub memory_source: String,
    pub deleted_at: String,
    pub deleted_by: String,
    pub reason: Option<String>,
    pub restorable: bool,
}

/// Panel-local color palette. Kept internal so future theme integration
/// touches one place. Chosen to sit on dark-gray terminal backgrounds.
const COLOR_ACCENT: Color = Color::Rgb(97, 175, 239); // one-dark blue
const COLOR_TEXT: Color = Color::Rgb(171, 178, 191);
const COLOR_MUTED: Color = Color::Rgb(127, 132, 142);
const COLOR_BORDER: Color = Color::Rgb(80, 86, 95);
const COLOR_WARN: Color = Color::Rgb(224, 108, 117);

/// Per-scope counts + last-write timestamp for Section 3.
#[derive(Debug, Clone, Default)]
pub struct ScopeSummaryRow {
    pub scope_label: &'static str,
    pub count: i64,
    pub last_write: Option<String>,
}

/// One row displayed in Section 1 / 2. Lightweight projection of
/// `ScoredMemory` — keeps the panel's state small so 100-entry pools
/// don't balloon `App`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // projection populated from ScoredMemory; some fields surface only in future panel views
pub struct MemoryRow {
    pub id: i64,
    pub scope_level: i32,
    pub scope_label: String,
    pub memory_type: String,
    pub text: String,
    pub importance: f32,
    pub trust_score: f32,
    pub source: MemorySource,
    pub final_score: f32,
    pub created_at: String,
    /// Tier B / B6: utilization rate from `retrieval_use`. `None`
    /// when no telemetry rows exist for this memory yet (or telemetry
    /// is disabled).
    pub utilization_rate: Option<f32>,
}

impl MemoryRow {
    /// Project a `ScoredMemory` into a panel row, truncating long text.
    pub fn from_scored(m: &ScoredMemory) -> Self {
        Self {
            id: m.id,
            scope_level: m.scope_level,
            scope_label: format_scope_short(m.scope_level),
            memory_type: m.memory_type.as_str().to_string(),
            text: truncate(&m.content, 80),
            importance: m.importance,
            trust_score: m.trust_score,
            source: m.source,
            final_score: m.final_score,
            created_at: m.created_at.clone(),
            utilization_rate: None,
        }
    }
}

/// Which section has keyboard focus. `Tab` rotates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelSection {
    InjectedNow,
    RecentlyWritten,
    ScopeSummary,
    Search,
}

impl PanelSection {
    fn next(self) -> Self {
        match self {
            Self::InjectedNow => Self::RecentlyWritten,
            Self::RecentlyWritten => Self::ScopeSummary,
            Self::ScopeSummary => Self::Search,
            Self::Search => Self::InjectedNow,
        }
    }
}

/// Inline input mode for Recently Written's `e` / `s` actions.
/// `None` is the normal list state; the other two variants hijack
/// keystrokes until the user presses `Enter` (commit) or `Esc`
/// (cancel).
#[derive(Debug, Clone)]
pub enum PanelPromptMode {
    None,
    /// `e`: replace the memory's text. Cursor always at the end of
    /// `buffer` — MVP input, not a full editor.
    EditText {
        memory_id: i64,
        buffer: String,
    },
    /// `s`: cycle through the 5 scope levels. `selected` is the
    /// currently-highlighted choice; left/right arrows (or `h`/`l`)
    /// move it. Complex scopes (Run/Module) that need a `run_id` or
    /// `module_path` resolve from the current app context at commit
    /// time — the prompt only carries the level choice.
    SetScope {
        memory_id: i64,
        selected: ScopeChoice,
    },
}

/// Scope levels the inline picker can select. Mirrors `WriteScope`'s
/// five variants but without the per-variant payload, which is
/// resolved at commit time (see `side_panel::handle_memory_panel_action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeChoice {
    Global,
    Workspace,
    Repo,
    Module,
    Run,
}

impl ScopeChoice {
    pub const ALL: [ScopeChoice; 5] = [
        ScopeChoice::Global,
        ScopeChoice::Workspace,
        ScopeChoice::Repo,
        ScopeChoice::Module,
        ScopeChoice::Run,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScopeChoice::Global => "Global",
            ScopeChoice::Workspace => "Workspace",
            ScopeChoice::Repo => "Repo",
            ScopeChoice::Module => "Module",
            ScopeChoice::Run => "Run",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    pub fn prev(self) -> Self {
        let i = self.index();
        Self::ALL[i.checked_sub(1).unwrap_or(Self::ALL.len() - 1)]
    }

    pub fn next(self) -> Self {
        let i = (self.index() + 1) % Self::ALL.len();
        Self::ALL[i]
    }
}

/// TUI state for the memory panel.
#[derive(Debug)]
pub struct MemoryPanelState {
    pub focused: PanelSection,

    /// C1.5: which lifecycle class the panel is currently filtered to.
    /// Default is `Record` (the workhorse). Switching tabs (`1`/`2`/`3`
    /// keys) changes which kind populates the Recently Written list and
    /// gates the destructive keys — when on the History tab, the panel
    /// rejects `d`/`e`/`p`/`s` with a "history is read-only" message,
    /// reinforcing the C1.2 writer-task guard and the C1.3 SQL trigger
    /// at the UI layer.
    pub active_kind: MemoryKind,

    /// C2.6: when true, the "Recently Written" section renders the
    /// `deletions` audit log instead of the per-kind memory list.
    /// Mutually exclusive with the per-kind tabs (1/2/3); the `4`
    /// key toggles into this mode. `u` on a row dispatches a
    /// `WriterMessage::Restore` (skipped silently for redactions).
    pub viewing_deletions: bool,
    pub deletions_rows: Vec<DeletionRow>,
    pub deletions_cursor: usize,

    // ── Section 1: Injected Now ──────────────────────────────────
    /// Current turn's manifest row (may be `None` until a manifest
    /// lands). Payload JSON is parsed lazily for the detail view.
    pub current_manifest: Option<InjectionManifestRow>,
    /// Latest ledger row for the current turn (session_thread + open
    /// questions surfaced near the manifest).
    #[allow(dead_code)] // populated by the manifest observer; surfaced in a future panel view
    pub current_ledger: Option<SessionLedgerTurn>,
    /// Cursor index into `manifest_selected_items`.
    pub injected_cursor: usize,
    /// Items actually selected in the manifest (resolved from the
    /// payload's `selected_ids` after the manifest lands).
    pub manifest_selected_items: Vec<MemoryRow>,
    /// Full candidate pool (Inspect view). Empty until `i` is pressed
    /// and the pool is parsed out of the manifest's payload JSON.
    pub manifest_pool: Vec<ManifestPoolEntry>,
    /// When true, the inspect overlay is active.
    pub inspecting: bool,
    /// When true, the history overlay is active (prev N turns).
    pub history_mode: bool,
    pub history_rows: Vec<InjectionManifestRow>,
    pub history_cursor: usize,

    // ── Section 2: Recently Written ──────────────────────────────
    pub recent_rows: Vec<MemoryRow>,
    pub recent_cursor: usize,
    /// Pending delete confirmation — set on `d`, cleared on `y`/`n`.
    pub confirm_delete_id: Option<i64>,
    /// Inline `e` (edit text) or `s` (change scope) prompt.
    pub prompt_mode: PanelPromptMode,

    // ── Section 3: Scope Summary ─────────────────────────────────
    pub scope_summary: Vec<ScopeSummaryRow>,

    // ── Section 4: Search ────────────────────────────────────────
    pub search_active: bool,
    pub search_query: String,
    pub search_results: Vec<MemoryRow>,
    pub search_cursor: usize,
    /// Last time search ran — drives the 150ms debounce.
    pub search_last_run: Option<Instant>,

    // ── Debounce / activity ──────────────────────────────────────
    /// Last committed-write time. Panel refreshes `Recently Written`
    /// at most once per 100ms when observer events storm in.
    pub last_recent_refresh: Option<Instant>,
    pub write_activity_counter: u64,
    pub last_error: Option<(String, Instant)>,
}

/// One candidate-pool entry decoded from a manifest payload's
/// `candidate_pool` array, flattened for panel display.
///
/// B2: `rerank_score` and `blended_score` are populated when the
/// manifest was produced with the cross-encoder reranker enabled and
/// the entry survived to the rerank stage. Older v1 manifests parse
/// fine because both are optional.
#[derive(Debug, Clone)]
#[allow(dead_code)] // decoded from manifest payloads; some fields surface only in future panel views
pub struct ManifestPoolEntry {
    pub memory_id: i64,
    pub scope_label: String,
    pub namespace: String,
    pub raw_similarity: f32,
    pub composite_score: f32,
    pub selected: bool,
    pub exclusion_reason: Option<String>,
    pub rerank_score: Option<f32>,
    pub blended_score: Option<f32>,
}

impl Default for MemoryPanelState {
    fn default() -> Self {
        Self {
            focused: PanelSection::InjectedNow,
            active_kind: MemoryKind::Record,
            viewing_deletions: false,
            deletions_rows: Vec::new(),
            deletions_cursor: 0,
            current_manifest: None,
            current_ledger: None,
            injected_cursor: 0,
            manifest_selected_items: Vec::new(),
            manifest_pool: Vec::new(),
            inspecting: false,
            history_mode: false,
            history_rows: Vec::new(),
            history_cursor: 0,
            recent_rows: Vec::new(),
            recent_cursor: 0,
            confirm_delete_id: None,
            prompt_mode: PanelPromptMode::None,
            scope_summary: Vec::new(),
            search_active: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_cursor: 0,
            search_last_run: None,
            last_recent_refresh: None,
            write_activity_counter: 0,
            last_error: None,
        }
    }
}

impl MemoryPanelState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cycle focus to the next section. Bound to `Tab`.
    pub fn focus_next(&mut self) {
        self.focused = self.focused.next();
    }

    /// C1.5: switch the active kind tab. Returns `true` when the kind
    /// actually changed so callers can decide whether to refresh the
    /// rows. Bound to `1` (Records), `2` (History), `3` (Summaries).
    pub fn set_active_kind(&mut self, kind: MemoryKind) -> bool {
        if self.active_kind == kind {
            return false;
        }
        self.active_kind = kind;
        // Reset cursors so we don't land on a row that no longer
        // exists in the new tab's list.
        self.recent_cursor = 0;
        self.search_cursor = 0;
        self.recent_rows.clear();
        true
    }

    /// C1.5: convenience for the side_panel destructive-key guard.
    /// Returns `true` when the active tab is the read-only History
    /// tab — destructive ops should refuse and show a hint.
    pub fn history_tab_active(&self) -> bool {
        self.active_kind == MemoryKind::History
    }

    /// C2.6: enter the Deletions tab. Returns `true` when the mode
    /// actually changed so callers can decide whether to refresh the
    /// audit list. Bound to `4`.
    pub fn enter_deletions_tab(&mut self) -> bool {
        if self.viewing_deletions {
            return false;
        }
        self.viewing_deletions = true;
        self.deletions_cursor = 0;
        self.deletions_rows.clear();
        true
    }

    /// C2.6: leave the Deletions tab and reactivate the per-kind view.
    /// Called when 1/2/3 are pressed.
    pub fn leave_deletions_tab(&mut self) -> bool {
        if !self.viewing_deletions {
            return false;
        }
        self.viewing_deletions = false;
        self.deletions_rows.clear();
        true
    }

    /// C2.6: restore eligibility for the row under the cursor.
    /// Redactions (`deleted_by = user_redaction`) are not restorable
    /// per the plan — surfaced here so the side_panel can swallow `u`
    /// with an explanatory message.
    pub fn deletion_under_cursor(&self) -> Option<&DeletionRow> {
        self.deletions_rows.get(self.deletions_cursor)
    }

    /// Parse the candidate pool out of the current manifest's payload.
    /// Called when the user presses `i` to enter inspect mode; kept
    /// lazy so the panel doesn't parse potentially 100-entry pools on
    /// every manifest event.
    pub fn load_inspect_pool(&mut self) {
        self.manifest_pool.clear();
        let Some(manifest) = &self.current_manifest else {
            return;
        };
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&manifest.payload) else {
            return;
        };
        let Some(pool) = payload.get("candidate_pool").and_then(|v| v.as_array()) else {
            return;
        };
        for entry in pool {
            let get_f32 = |k: &str| entry.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let opt_f32 = |k: &str| entry.get(k).and_then(|v| v.as_f64()).map(|x| x as f32);
            self.manifest_pool.push(ManifestPoolEntry {
                memory_id: entry.get("memory_id").and_then(|v| v.as_i64()).unwrap_or(0),
                scope_label: entry
                    .get("scope_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                namespace: entry
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                raw_similarity: get_f32("raw_similarity"),
                composite_score: get_f32("composite_score"),
                selected: entry
                    .get("selected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                exclusion_reason: entry
                    .get("exclusion_reason")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                rerank_score: opt_f32("rerank_score"),
                blended_score: opt_f32("blended_score"),
            });
        }
    }

    /// Render the whole panel. Layout is fixed proportions per plan:
    /// 40% Injected / 25% Recent / 20% Scope / 15% Search.
    pub fn render(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let block = Block::default()
            .title(if focused {
                "MEMORY (focused — Tab cycles section)"
            } else {
                "MEMORY"
            })
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused { COLOR_ACCENT } else { COLOR_BORDER }));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.inspecting {
            self.render_inspect_overlay(inner, buf);
            return;
        }
        if self.history_mode {
            self.render_history_overlay(inner, buf);
            return;
        }

        // C1.5 tab strip: one line at the very top showing the
        // available kinds and which is active.
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        self.render_kind_tabs(outer[0], buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(25),
                Constraint::Percentage(20),
                Constraint::Percentage(15),
            ])
            .split(outer[1]);

        self.render_injected_now(chunks[0], buf, focused);
        self.render_recently_written(chunks[1], buf, focused);
        self.render_scope_summary(chunks[2], buf, focused);
        self.render_search(chunks[3], buf, focused);
    }

    /// C1.5 tab strip render. Single-line, three labels, current tab
    /// highlighted with the accent color. Switches via `1`/`2`/`3` —
    /// see [`super::super::app::side_panel::handle_memory_panel_action`].
    fn render_kind_tabs(&self, area: Rect, buf: &mut Buffer) {
        let mut spans: Vec<Span> = Vec::with_capacity(10);
        spans.push(Span::styled(" ", Style::default()));
        for (idx, kind) in [
            ("1", MemoryKind::Record),
            ("2", MemoryKind::History),
            ("3", MemoryKind::Summary),
        ]
        .iter()
        {
            let active = !self.viewing_deletions && *kind == self.active_kind;
            let label = match kind {
                MemoryKind::Record => "Records",
                MemoryKind::History => "History (read-only)",
                MemoryKind::Summary => "Summaries",
            };
            let style = if active {
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(COLOR_MUTED)
            };
            spans.push(Span::styled(format!("[{idx}] {label}"), style));
            spans.push(Span::raw("  "));
        }
        // C2.6: Deletions tab. Hosts the audit log; `u` restores
        // eligible rows. Redactions are visible but marked permanent.
        let active = self.viewing_deletions;
        let style = if active {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(COLOR_MUTED)
        };
        spans.push(Span::styled("[4] Deletions (u: restore)", style));
        Paragraph::new(Line::from(spans)).render(area, buf);
    }

    fn render_injected_now(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let heading = section_heading(
            "Injected now",
            focused && self.focused == PanelSection::InjectedNow,
        );
        let mut lines: Vec<Line> = vec![heading];

        if self.manifest_selected_items.is_empty() {
            lines.push(Line::from(Span::styled(
                match &self.current_manifest {
                    Some(_) => "(manifest has no selected items)",
                    None => "(waiting for first turn…)",
                },
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (i, m) in self.manifest_selected_items.iter().enumerate() {
                lines.push(format_memory_row(m, i == self.injected_cursor));
            }
        }

        // Footer hint — keep visible only when focused.
        if focused && self.focused == PanelSection::InjectedNow {
            lines.push(Line::from(Span::styled(
                "[i] inspect pool   [h] history",
                Style::default().fg(COLOR_MUTED),
            )));
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_recently_written(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        // C2.6: when the Deletions tab is active, this section becomes
        // the audit-log view instead of the per-kind recent list.
        if self.viewing_deletions {
            self.render_deletions(area, buf, focused);
            return;
        }
        let heading = section_heading(
            "Recently written (24h)",
            focused && self.focused == PanelSection::RecentlyWritten,
        );
        let mut lines: Vec<Line> = vec![heading];

        if self.recent_rows.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no writes in the last 24h)",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (i, m) in self
                .recent_rows
                .iter()
                .take(area.height as usize)
                .enumerate()
            {
                lines.push(format_memory_row(m, i == self.recent_cursor));
            }
        }

        if let Some(pending_id) = self.confirm_delete_id {
            lines.push(Line::from(Span::styled(
                format!("Delete memory #{pending_id}? [y/n]"),
                Style::default().fg(COLOR_WARN).add_modifier(Modifier::BOLD),
            )));
        } else {
            match &self.prompt_mode {
                PanelPromptMode::EditText { memory_id, buffer } => {
                    lines.push(Line::from(Span::styled(
                        format!("Edit #{memory_id}: [Enter] save  [Esc] cancel"),
                        Style::default()
                            .fg(COLOR_ACCENT)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("› {}", buffer),
                        Style::default().fg(COLOR_TEXT),
                    )));
                }
                PanelPromptMode::SetScope {
                    memory_id,
                    selected,
                } => {
                    lines.push(Line::from(Span::styled(
                        format!("Scope #{memory_id}: [←/→] change  [Enter] save  [Esc] cancel"),
                        Style::default()
                            .fg(COLOR_ACCENT)
                            .add_modifier(Modifier::BOLD),
                    )));
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    for (i, c) in ScopeChoice::ALL.iter().enumerate() {
                        if i > 0 {
                            spans.push(Span::raw("  "));
                        }
                        let style = if *c == *selected {
                            Style::default()
                                .fg(COLOR_ACCENT)
                                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                        } else {
                            Style::default().fg(COLOR_MUTED)
                        };
                        spans.push(Span::styled(c.label(), style));
                    }
                    lines.push(Line::from(spans));
                }
                PanelPromptMode::None => {
                    if focused && self.focused == PanelSection::RecentlyWritten {
                        lines.push(Line::from(Span::styled(
                            "[d] delete  [p] pin  [s] scope  [e] edit",
                            Style::default().fg(COLOR_MUTED),
                        )));
                    }
                }
            }
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    /// C2.6: render the audit-log view in place of the per-kind list.
    /// Each line is one `deletions` row (newest first); the cursor row
    /// gets the accent prefix. Permanent (`user_redaction`) rows show
    /// a distinct marker so the user knows `u` won't bring them back.
    fn render_deletions(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let heading = section_heading(
            "Deletions (audit log)",
            focused && self.focused == PanelSection::RecentlyWritten,
        );
        let mut lines: Vec<Line> = vec![heading];
        if self.deletions_rows.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no recent soft-deletes)",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (i, d) in self
                .deletions_rows
                .iter()
                .take(area.height.saturating_sub(2) as usize)
                .enumerate()
            {
                let cursor = if i == self.deletions_cursor { ">" } else { " " };
                let marker = if d.restorable {
                    Span::styled("◯", Style::default().fg(COLOR_ACCENT))
                } else {
                    Span::styled("⛔", Style::default().fg(COLOR_WARN))
                };
                let body = format!(
                    "{cursor} #{aid:>4}  mem={mid:<4}  {kind:<7}  {by:<14}  {at}  {reason}",
                    aid = d.id,
                    mid = d.memory_id,
                    kind = d.memory_kind,
                    by = d.deleted_by,
                    at = d.deleted_at,
                    reason = d.reason.as_deref().unwrap_or(""),
                );
                lines.push(Line::from(vec![
                    marker,
                    Span::raw(" "),
                    Span::styled(body, Style::default().fg(COLOR_TEXT)),
                ]));
            }
        }
        if focused && self.focused == PanelSection::RecentlyWritten {
            lines.push(Line::from(Span::styled(
                "[u] restore  [↑/↓] navigate  [1/2/3] back to per-kind",
                Style::default().fg(COLOR_MUTED),
            )));
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_scope_summary(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let heading = section_heading(
            "Scope summary",
            focused && self.focused == PanelSection::ScopeSummary,
        );
        let mut lines: Vec<Line> = vec![heading];
        if self.scope_summary.is_empty() {
            lines.push(Line::from(Span::styled(
                "(empty)",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for row in &self.scope_summary {
                let line = format!(
                    "{:<10}│ {:>5} │ {}",
                    row.scope_label,
                    row.count,
                    row.last_write.as_deref().unwrap_or("—"),
                );
                lines.push(Line::from(line));
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_search(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let heading = section_heading(
            if self.search_active {
                "Search (live)"
            } else {
                "Search  — / to activate"
            },
            focused && self.focused == PanelSection::Search,
        );
        let mut lines: Vec<Line> = vec![heading];
        if self.search_active {
            lines.push(Line::from(Span::styled(
                format!("› {}", self.search_query),
                Style::default().fg(COLOR_ACCENT),
            )));
            for (i, m) in self
                .search_results
                .iter()
                .take(5.min(area.height as usize))
                .enumerate()
            {
                lines.push(format_memory_row(m, i == self.search_cursor));
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_inspect_overlay(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "Manifest candidate pool  ([Esc] close)",
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ))];
        if self.manifest_pool.is_empty() {
            lines.push(Line::from(Span::styled(
                "(empty pool)",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("{:>5} {:<14} sim  comp  sel  reason", "id", "scope"),
                Style::default().fg(COLOR_MUTED),
            )));
            for entry in &self.manifest_pool {
                let sel = if entry.selected { "✓" } else { " " };
                let reason = entry.exclusion_reason.as_deref().unwrap_or("");
                lines.push(Line::from(format!(
                    "{:>5} {:<14} {:.2} {:.2}  {}    {}",
                    entry.memory_id,
                    truncate(&entry.scope_label, 14),
                    entry.raw_similarity,
                    entry.composite_score,
                    sel,
                    reason,
                )));
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_history_overlay(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "Injection history  ([Esc] close  [Enter] open)",
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ))];
        if self.history_rows.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no manifests in retention window)",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (i, row) in self.history_rows.iter().enumerate() {
                let query_text = extract_payload_str(&row.payload, "query_text");
                let selected_n = extract_payload_array_len(&row.payload, "selected_ids");
                let marker = if i == self.history_cursor { "›" } else { " " };
                lines.push(Line::from(format!(
                    "{} {} — \"{}\"  ({} selected)",
                    marker,
                    row.created_at,
                    truncate(&query_text, 40),
                    selected_n,
                )));
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

fn section_heading(text: &str, focused: bool) -> Line<'static> {
    let mut style = Style::default().add_modifier(Modifier::BOLD);
    if focused {
        style = style.fg(COLOR_ACCENT);
    } else {
        style = style.fg(COLOR_TEXT);
    }
    Line::from(Span::styled(format!("── {text} "), style))
}

fn format_memory_row(row: &MemoryRow, selected: bool) -> Line<'static> {
    let marker = if selected { "›" } else { " " };
    let src_badge = match row.source {
        MemorySource::UserRemember | MemorySource::UserPanel => "⟂U",
        MemorySource::LlmAnnotated => "⟂A",
        MemorySource::LlmExtracted => "⟂X",
        MemorySource::LlmConsolidated | MemorySource::SwarmConsolidated => "⟂C",
        MemorySource::McpImport => "⟂M",
        MemorySource::ToolOutput => "⟂T",
        MemorySource::RawTranscript => "⟂H",
        MemorySource::UnknownLegacy => "⟂?",
    };
    // B6 utilization indicator: ↑ for highly-used (>0.6), ⟂ for
    // rarely-used (<0.1), · for in-between, blank when no telemetry.
    let util_badge = match row.utilization_rate {
        Some(r) if r > 0.6 => format!(" ↑ {:.2}", r),
        Some(r) if r < 0.1 => format!(" ⟂ {:.2}", r),
        Some(r) => format!(" · {:.2}", r),
        None => String::new(),
    };
    let line = format!(
        "{} [{}] {} {} • \"{}\" • {:.2} • t {:.2}{}",
        marker,
        &row.scope_label[..row.scope_label.len().min(1)],
        src_badge,
        row.memory_type,
        row.text,
        row.final_score,
        row.trust_score,
        util_badge,
    );
    let style = if selected {
        Style::default().fg(COLOR_ACCENT)
    } else {
        Style::default().fg(COLOR_TEXT)
    };
    Line::from(Span::styled(line, style))
}

fn format_scope_short(level: i32) -> String {
    match level {
        0 => "Global",
        1 => "Workspace",
        2 => "Repo",
        3 => "Module",
        4 => "Run",
        _ => "?",
    }
    .to_string()
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn extract_payload_str(payload: &str, key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get(key).and_then(|x| x.as_str()).map(String::from))
        .unwrap_or_default()
}

fn extract_payload_array_len(payload: &str, key: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get(key).and_then(|x| x.as_array()).map(|a| a.len()))
        .unwrap_or(0)
}

/// Update / activity rate limiter for the Recently Written section.
/// Plan §A4 asks for ≥100ms debounce on observer storms.
pub const RECENT_REFRESH_DEBOUNCE: Duration = Duration::from_millis(100);

/// Live-search debounce. Plan §A4 Section 4 wants <10ms refresh budget
/// per section; 150ms debounce is lax enough for typing without
/// hammering sqlite-vec on every keystroke.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_choice_cycles_bidirectionally() {
        assert_eq!(ScopeChoice::Global.next(), ScopeChoice::Workspace);
        assert_eq!(ScopeChoice::Workspace.next(), ScopeChoice::Repo);
        assert_eq!(ScopeChoice::Run.next(), ScopeChoice::Global);
        assert_eq!(ScopeChoice::Global.prev(), ScopeChoice::Run);
        assert_eq!(ScopeChoice::Workspace.prev(), ScopeChoice::Global);
    }

    #[test]
    fn panel_section_cycle_wraps() {
        let mut s = PanelSection::InjectedNow;
        s = s.next();
        assert_eq!(s, PanelSection::RecentlyWritten);
        s = s.next();
        assert_eq!(s, PanelSection::ScopeSummary);
        s = s.next();
        assert_eq!(s, PanelSection::Search);
        s = s.next();
        assert_eq!(s, PanelSection::InjectedNow);
    }

    #[test]
    fn default_state_is_injected_focused() {
        let s = MemoryPanelState::new();
        assert_eq!(s.focused, PanelSection::InjectedNow);
        assert!(s.manifest_selected_items.is_empty());
    }

    #[test]
    fn load_inspect_pool_parses_candidate_pool() {
        let mut s = MemoryPanelState::new();
        s.current_manifest = Some(InjectionManifestRow {
            id: 1,
            turn_id: "t".into(),
            session_id: "c".into(),
            source_channel: "chat".into(),
            payload: serde_json::json!({
                "candidate_pool": [
                    {"memory_id": 42, "scope_label": "repo:abc",
                     "namespace": "default", "raw_similarity": 0.9,
                     "composite_score": 0.84, "selected": true}
                ]
            })
            .to_string(),
            created_at: "2026-04-22".into(),
        });
        s.load_inspect_pool();
        assert_eq!(s.manifest_pool.len(), 1);
        assert_eq!(s.manifest_pool[0].memory_id, 42);
        assert!(s.manifest_pool[0].selected);
    }
}
