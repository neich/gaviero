use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::theme;
use crate::widgets::scroll_state::ScrollState;
use crate::widgets::scrollbar::render_scrollbar;
use gaviero_core::swarm::models::{AgentStatus, SwarmResult};
use gaviero_core::types::ModelTier;

// ── Activity log types ──────────────────────────────────────────

/// A single line in an agent's activity log.
#[derive(Debug, Clone)]
pub struct ActivityLine {
    pub kind: ActivityKind,
    pub text: String,
}

/// Kind of activity — determines rendering style.
#[derive(Debug, Clone, PartialEq)]
pub enum ActivityKind {
    /// Streaming text output (coalesced).
    Text,
    /// Tool call: "[Read] src/auth.rs".
    ToolCall,
    /// Status change.
    Status,
    /// File written: "[wrote src/auth.rs +10 -3]".
    FileChange,
}

/// Max activity lines stored per agent.
const MAX_ACTIVITY_LINES: usize = 500;

// ── Diff overlay types ──────────────────────────────────────────

/// Rendering category for a unified-diff line.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
    Header,
}

/// State for the per-agent diff overlay shown when the user presses Enter.
#[derive(Debug, Clone)]
pub struct SwarmAgentDiffState {
    pub agent_id: String,
    pub lines: Vec<(DiffLineKind, String)>,
    pub scroll: usize,
}

// ── Agent entry ─────────────────────────────────────────────────

/// An entry in the swarm dashboard table.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub status: AgentStatus,
    pub branch: Option<String>,
    pub detail: String,
    pub modified_files: Vec<String>,
    /// Model tier for this agent (set by tier routing).
    pub model_tier: Option<ModelTier>,
    /// Backend description (e.g. "sonnet", "haiku", "ollama:qwen2.5").
    pub backend: Option<String>,
    /// Rolling activity log for the detail pane.
    pub activity: Vec<ActivityLine>,
    /// When the agent started running (for elapsed display).
    pub started_at: Option<std::time::Instant>,
}

// ── Dashboard focus ─────────────────────────────────────────────

/// Which sub-panel of the dashboard has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DashboardFocus {
    /// Agent list — Up/Down selects agents.
    Table,
    /// Activity log — Up/Down scrolls output.
    Detail,
}

// ── Dashboard state ─────────────────────────────────────────────

/// State for the swarm dashboard panel.
pub struct SwarmDashboardState {
    pub agents: Vec<AgentEntry>,
    pub scroll: ScrollState,
    pub phase: String,
    pub tier_current: usize,
    pub tier_total: usize,
    pub result: Option<SwarmResult>,
    /// Running cost estimate (updated via on_cost_update).
    pub estimated_cost_usd: f64,
    /// Status message shown when no agents exist yet (e.g., during planning).
    pub status_message: String,
    /// Scroll position within the detail pane.
    pub detail_scroll: usize,
    /// Auto-follow latest activity in the detail pane.
    pub detail_auto_scroll: bool,
    /// Which sub-panel has keyboard focus.
    pub focus: DashboardFocus,
    /// Layout rect for the agent table (set during render, used for mouse hit-testing).
    pub table_rect: Rect,
    /// Layout rect for the detail pane (set during render, used for mouse hit-testing).
    pub detail_rect: Rect,
    /// When Some, the detail pane shows a read-only diff overlay for this agent.
    pub diff_agent: Option<SwarmAgentDiffState>,
    /// When true, a confirmation prompt is shown before undoing the swarm.
    pub pending_undo_confirm: bool,
}

impl SwarmDashboardState {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            scroll: ScrollState::new(),
            phase: "idle".to_string(),
            tier_current: 0,
            tier_total: 0,
            result: None,
            estimated_cost_usd: 0.0,
            status_message: String::new(),
            detail_scroll: 0,
            detail_auto_scroll: true,
            focus: DashboardFocus::Table,
            table_rect: Rect::default(),
            detail_rect: Rect::default(),
            diff_agent: None,
            pending_undo_confirm: false,
        }
    }

    /// Reset all state for a new swarm run.
    pub fn reset(&mut self, phase: &str) {
        self.agents.clear();
        self.scroll.reset();
        self.phase = phase.to_string();
        self.tier_current = 0;
        self.tier_total = 0;
        self.result = None;
        self.estimated_cost_usd = 0.0;
        self.status_message = String::new();
        self.detail_scroll = 0;
        self.detail_auto_scroll = true;
        self.focus = DashboardFocus::Table;
        self.diff_agent = None;
        self.pending_undo_confirm = false;
    }

    /// Open the diff overlay for `agent_id` using the provided unified diff text.
    pub fn show_diff(&mut self, agent_id: String, diff_text: String) {
        self.diff_agent = Some(SwarmAgentDiffState {
            agent_id,
            lines: parse_diff(&diff_text),
            scroll: 0,
        });
    }

    /// Close the diff overlay.
    pub fn close_diff(&mut self) {
        self.diff_agent = None;
    }

    /// Toggle focus between Table and Detail.
    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            DashboardFocus::Table => DashboardFocus::Detail,
            DashboardFocus::Detail => DashboardFocus::Table,
        };
    }

    pub fn set_phase(&mut self, phase: &str) {
        self.phase = phase.to_string();
    }

    pub fn set_tier(&mut self, current: usize, total: usize) {
        self.tier_current = current;
        self.tier_total = total;
    }

    pub fn update_agent(&mut self, id: &str, status: &AgentStatus, detail: &str) {
        if let Some(entry) = self.agents.iter_mut().find(|a| a.id == id) {
            // Track Running transitions for elapsed time
            if matches!(status, AgentStatus::Running) && entry.started_at.is_none() {
                entry.started_at = Some(std::time::Instant::now());
            }
            // Only log actual state transitions to avoid flooding
            let state_changed = std::mem::discriminant(&entry.status) != std::mem::discriminant(status);
            entry.status = status.clone();
            entry.detail = detail.to_string();
            if state_changed {
                let status_text = match status {
                    AgentStatus::Pending => format!("Queued: {}", detail),
                    AgentStatus::Running => format!("Running: {}", detail),
                    AgentStatus::Completed => format!("Completed: {}", detail),
                    AgentStatus::Failed(e) => format!("Failed: {}", e),
                };
                entry.activity.push(ActivityLine {
                    kind: ActivityKind::Status,
                    text: status_text,
                });
                cap_activity(&mut entry.activity);
            }
        } else {
            let started = if matches!(status, AgentStatus::Running) {
                Some(std::time::Instant::now())
            } else {
                None
            };
            self.agents.push(AgentEntry {
                id: id.to_string(),
                status: status.clone(),
                branch: Some(format!("gaviero/{}", id)),
                detail: detail.to_string(),
                modified_files: Vec::new(),
                model_tier: None,
                backend: None,
                activity: Vec::new(),
                started_at: started,
            });
        }
    }

    pub fn set_tier_dispatch(&mut self, unit_id: &str, tier: ModelTier, backend: &str) {
        if let Some(entry) = self.agents.iter_mut().find(|a| a.id == unit_id) {
            entry.model_tier = Some(tier);
            entry.backend = Some(backend.to_string());
        }
    }

    pub fn set_cost(&mut self, usd: f64) {
        self.estimated_cost_usd = usd;
    }

    pub fn set_result(&mut self, result: SwarmResult) {
        for manifest in &result.manifests {
            if let Some(entry) = self.agents.iter_mut().find(|a| a.id == manifest.work_unit_id) {
                entry.status = manifest.status.clone();
                entry.branch = manifest.branch.clone();
                entry.modified_files = manifest.modified_files
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                // Update detail to reflect the real committed file count (the earlier
                // on_agent_state_changed fires before the worktree commit, so it always
                // shows "Modified 0 files").
                let n = entry.modified_files.len();
                entry.detail = if n > 0 {
                    format!("Modified {} file{}", n, if n == 1 { "" } else { "s" })
                } else {
                    manifest.summary.clone().unwrap_or_else(|| entry.detail.clone())
                };
            }
        }
        self.result = Some(result);
    }

    // ── Event intake methods ────────────────────────────────────

    /// Append streaming text from an agent. Coalesces consecutive Text lines.
    pub fn append_stream_chunk(&mut self, agent_id: &str, text: &str) {
        let Some(entry) = self.agents.iter_mut().find(|a| a.id == agent_id) else { return };

        // Strip ANSI escape sequences — raw escape codes written char-by-char into
        // the ratatui cell buffer corrupt neighbouring panels.
        let clean = strip_ansi(text);

        // Coalesce with last Text line
        if let Some(last) = entry.activity.last_mut() {
            if last.kind == ActivityKind::Text {
                last.text.push_str(&clean);
                return;
            }
        }
        entry.activity.push(ActivityLine {
            kind: ActivityKind::Text,
            text: clean,
        });
        cap_activity(&mut entry.activity);
    }

    /// Record a tool call for an agent.
    /// `tool_info` may already be formatted as `"[Read] path"` or just `"Read"`.
    pub fn add_tool_call(&mut self, agent_id: &str, tool_info: &str) {
        let Some(entry) = self.agents.iter_mut().find(|a| a.id == agent_id) else { return };
        let text = if tool_info.starts_with('[') {
            tool_info.to_string()
        } else {
            format!("[{}]", tool_info)
        };
        entry.detail = text.clone();
        entry.activity.push(ActivityLine {
            kind: ActivityKind::ToolCall,
            text,
        });
        cap_activity(&mut entry.activity);
    }

    /// Update streaming status for an agent.
    /// Only updates `detail` for the table row. Does NOT push to activity log
    /// — status updates are transient labels (e.g., "Building plan..."),
    /// not meaningful events worth preserving in the log.
    pub fn set_streaming_status(&mut self, agent_id: &str, status: &str) {
        let Some(entry) = self.agents.iter_mut().find(|a| a.id == agent_id) else { return };
        entry.detail = status.to_string();
    }

    /// Record a file change for an agent.
    pub fn add_file_change(&mut self, agent_id: &str, path: &str, additions: usize, deletions: usize) {
        let Some(entry) = self.agents.iter_mut().find(|a| a.id == agent_id) else { return };
        let line = format!("[wrote {} +{} -{}]", path, additions, deletions);
        entry.detail = line.clone();
        entry.activity.push(ActivityLine {
            kind: ActivityKind::FileChange,
            text: line,
        });
        cap_activity(&mut entry.activity);
    }

    // ── Rendering ───────────────────────────────────────────────

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let sel_bg = if focused {
            theme::FOCUSED_SELECTION_BG
        } else {
            theme::DARK_BG
        };

        // Clear entire area
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        // Header (1 line)
        self.render_header(area.x, area.y, area.width, buf, bg);

        if self.agents.is_empty() {
            self.render_empty_state(area, buf, bg);
            return;
        }

        // Split remaining area: table + separator + detail
        let content_start = area.y + 1;
        let content_height = area.height.saturating_sub(1);

        // Table: min(agent_count, 50% of content, 10 rows max)
        let table_height = (self.agents.len() as u16)
            .min(content_height / 2)
            .min(10)
            .max(2);
        let detail_height = content_height.saturating_sub(table_height + 1); // +1 for separator

        let table_area = Rect {
            x: area.x,
            y: content_start,
            width: area.width,
            height: table_height,
        };

        let sep_y = content_start + table_height;

        let detail_area = Rect {
            x: area.x,
            y: sep_y + 1,
            width: area.width,
            height: detail_height,
        };

        // Store rects for mouse hit-testing
        self.table_rect = table_area;
        self.detail_rect = detail_area;

        // Use different highlight intensity based on focus
        let table_sel_bg = if self.focus == DashboardFocus::Table {
            sel_bg
        } else {
            theme::DARK_BG
        };

        self.scroll.set_viewport(table_area.height as usize);
        self.scroll.ensure_visible();
        self.render_agent_table(table_area, buf, bg, table_sel_bg);
        self.render_separator(area.x, sep_y, area.width, buf, bg);

        if detail_height > 0 {
            self.render_detail_pane(detail_area, buf, bg);
        }
    }

    fn render_header(&self, x: u16, y: u16, width: u16, buf: &mut Buffer, bg: Color) {
        let cost_part = if self.estimated_cost_usd > 0.0 {
            format!("  ~${:.3}", self.estimated_cost_usd)
        } else {
            String::new()
        };
        let header = if self.tier_total > 0 {
            format!(" SWARM  {}  Tier {}/{}  {} agents{}",
                self.phase, self.tier_current, self.tier_total, self.agents.len(), cost_part)
        } else {
            format!(" SWARM  {}  {} agents{}", self.phase, self.agents.len(), cost_part)
        };
        let style = Style::default()
            .fg(theme::WARNING)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        render_text(buf, x, y, x + width, &header, style);
    }

    fn render_empty_state(&self, area: Rect, buf: &mut Buffer, bg: Color) {
        let is_active = self.phase != "idle" && self.phase != "completed" && self.phase != "failed";
        if is_active {
            let spinner = match (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() / 300) % 4
            {
                0 => "⠋",
                1 => "⠙",
                2 => "⠹",
                _ => "⠸",
            };
            let msg = if !self.status_message.is_empty() {
                format!(" {} {}", spinner, self.status_message)
            } else {
                format!(" {} {}...", spinner, self.phase)
            };
            let style = Style::default().fg(theme::FOCUS_BORDER).bg(bg);
            if area.y + 2 < area.bottom() {
                render_text(buf, area.x, area.y + 2, area.right(), &msg, style);
            }
        } else if self.phase == "failed" {
            let msg = if !self.status_message.is_empty() {
                self.status_message.clone()
            } else {
                "Run failed".into()
            };
            // Render error message with word-wrap across available lines
            let style = Style::default().fg(theme::ERROR).bg(bg);
            let width = (area.width as usize).saturating_sub(2);
            let mut y = area.y + 2;
            for chunk in msg.as_bytes().chunks(width.max(1)) {
                if y >= area.bottom() {
                    break;
                }
                let text = std::str::from_utf8(chunk).unwrap_or("");
                render_text(buf, area.x + 1, y, area.right(), text, style);
                y += 1;
            }
        } else {
            let msg = " Press 's' to submit a task, or use --work-units in CLI";
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            if area.y + 2 < area.bottom() {
                render_text(buf, area.x, area.y + 2, area.right(), msg, style);
            }
        }
    }

    fn render_agent_table(&self, area: Rect, buf: &mut Buffer, bg: Color, sel_bg: Color) {
        let fg = theme::TEXT_FG;
        let viewport = area.height as usize;

        for row in 0..viewport {
            let idx = self.scroll.offset + row;
            if idx >= self.agents.len() {
                break;
            }
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }

            let agent = &self.agents[idx];
            let is_selected = idx == self.scroll.selected;
            let row_bg = if is_selected { sel_bg } else { bg };

            // Clear row
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(row_bg));
            }

            // Status icon
            let (icon, icon_color) = match &agent.status {
                AgentStatus::Pending => ("◯", theme::TEXT_DIM),
                AgentStatus::Running => ("●", theme::FOCUS_BORDER),
                AgentStatus::Completed => ("✓", theme::SUCCESS),
                AgentStatus::Failed(_) => ("✗", theme::ERROR),
            };
            render_text(buf, area.x + 1, y, area.x + 3, icon, Style::default().fg(icon_color).bg(row_bg));

            // Tier badge
            let mut col = area.x + 3;
            if let Some(tier) = &agent.model_tier {
                let (badge, badge_color) = match tier {
                    ModelTier::Cheap => ("C", theme::TIER_CHEAP),
                    ModelTier::Expensive => ("E", theme::TIER_EXPENSIVE),
                };
                render_text(buf, col, y, col + 2, badge, Style::default().fg(badge_color).bg(row_bg).add_modifier(Modifier::BOLD));
                col += 2;
            }

            // Agent ID (truncated to ~14 chars)
            let id_display: String = agent.id.chars().take(14).collect();
            render_text(buf, col, y, col + 15, &id_display, Style::default().fg(fg).bg(row_bg).add_modifier(Modifier::BOLD));
            col += 15;

            // Elapsed time (right-aligned)
            let elapsed_str = format_elapsed(agent);
            // "↵ diff" hint for the selected completed agent that has a branch
            let hint_str = if is_selected
                && matches!(agent.status, AgentStatus::Completed)
                && agent.branch.is_some()
            {
                " ↵ diff"
            } else {
                ""
            };
            let hint_width = hint_str.len() as u16;

            if !elapsed_str.is_empty() {
                let elapsed_x = area.right().saturating_sub(elapsed_str.len() as u16 + 1 + hint_width);
                if elapsed_x > col {
                    render_text(buf, elapsed_x, y, elapsed_x + elapsed_str.len() as u16,
                        &elapsed_str, Style::default().fg(theme::TEXT_DIM).bg(row_bg));
                }
                if !hint_str.is_empty() {
                    let hint_x = area.right().saturating_sub(hint_width);
                    render_text(buf, hint_x, y, area.right(), hint_str,
                        Style::default().fg(theme::ACCENT).bg(row_bg));
                }
                let detail_end = elapsed_x.saturating_sub(1);
                let detail = truncate_detail(agent, (detail_end.saturating_sub(col)) as usize);
                render_text(buf, col, y, detail_end, &detail,
                    Style::default().fg(theme::MEDIUM_GRAY).bg(row_bg));
            } else {
                let right_edge = area.right().saturating_sub(hint_width);
                if !hint_str.is_empty() {
                    render_text(buf, right_edge, y, area.right(), hint_str,
                        Style::default().fg(theme::ACCENT).bg(row_bg));
                }
                let detail = truncate_detail(agent, (right_edge.saturating_sub(col)) as usize);
                render_text(buf, col, y, right_edge, &detail,
                    Style::default().fg(theme::MEDIUM_GRAY).bg(row_bg));
            }
        }
    }

    fn render_separator(&self, x: u16, y: u16, width: u16, buf: &mut Buffer, bg: Color) {
        if y >= buf.area().bottom() {
            return;
        }
        let agent_name = self.agents.get(self.scroll.selected)
            .map(|a| a.id.as_str())
            .unwrap_or("(none)");

        let focus_indicator = if self.focus == DashboardFocus::Detail { " ▼" } else { "" };
        let label = format!(" {}{} ", agent_name, focus_indicator);
        let sep_color = if self.focus == DashboardFocus::Detail {
            theme::FOCUS_BORDER
        } else {
            theme::TEXT_DIM
        };
        let sep_style = Style::default().fg(sep_color).bg(bg);
        let label_style = Style::default().fg(theme::FOCUS_BORDER).bg(bg).add_modifier(Modifier::BOLD);

        // Draw separator line
        let mut cx = x;
        // Left dashes
        let left_dashes = 2u16;
        for _ in 0..left_dashes {
            if cx < x + width && cx < buf.area().right() {
                buf[(cx, y)].set_char('─').set_style(sep_style);
            }
            cx += 1;
        }
        // Label
        for ch in label.chars() {
            if cx < x + width && cx < buf.area().right() {
                buf[(cx, y)].set_char(ch).set_style(label_style);
            }
            cx += 1;
        }
        // Right dashes
        while cx < x + width && cx < buf.area().right() {
            buf[(cx, y)].set_char('─').set_style(sep_style);
            cx += 1;
        }
    }

    fn render_detail_pane(&self, area: Rect, buf: &mut Buffer, bg: Color) {
        // Diff overlay takes priority
        if let Some(ref diff) = self.diff_agent {
            self.render_diff_overlay(area, buf, bg, diff);
            // If undo confirm is also pending, show it at the bottom of the diff
            if self.pending_undo_confirm {
                render_undo_confirm_line(area, buf, bg);
            }
            return;
        }

        // Undo confirmation prompt (shown at bottom of detail pane)
        let detail_area = if self.pending_undo_confirm && area.height > 1 {
            render_undo_confirm_line(area, buf, bg);
            Rect { height: area.height - 1, ..area }
        } else {
            area
        };

        let Some(agent) = self.agents.get(self.scroll.selected) else { return };

        if agent.activity.is_empty() {
            let msg = " Waiting for activity...";
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            if detail_area.y < detail_area.bottom() {
                render_text(buf, detail_area.x, detail_area.y, detail_area.right(), msg, style);
            }
            return;
        }

        // Flatten activity + word-wrap to the available width.
        // Each element in display_lines is exactly one rendered row.
        let content_width = detail_area.width.saturating_sub(1) as usize; // -1 for scrollbar
        let display_lines = flatten_and_wrap_activity(&agent.activity, content_width);
        let total = display_lines.len();
        let viewport = detail_area.height as usize;

        // Auto-scroll: show the bottom
        let scroll = if self.detail_auto_scroll {
            total.saturating_sub(viewport)
        } else {
            self.detail_scroll.min(total.saturating_sub(viewport))
        };

        // Render visible lines
        for row in 0..viewport {
            let idx = scroll + row;
            if idx >= total {
                break;
            }
            let y = detail_area.y + row as u16;
            if y >= detail_area.bottom() {
                break;
            }

            let (kind, text) = &display_lines[idx];
            let style = match kind {
                ActivityKind::ToolCall => Style::default()
                    .fg(theme::ACTIVITY_TOOL_CALL)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
                ActivityKind::FileChange => Style::default()
                    .fg(theme::SUCCESS)
                    .bg(bg),
                ActivityKind::Status => Style::default()
                    .fg(theme::ACTIVITY_STATUS)
                    .bg(bg),
                ActivityKind::Text => Style::default()
                    .fg(theme::TEXT_FG)
                    .bg(bg),
            };

            render_text(buf, detail_area.x + 1, y, detail_area.x + content_width as u16, text, style);
        }

        // Scrollbar
        if total > viewport {
            render_scrollbar(detail_area, buf, total, viewport, scroll);
        }
    }

    fn render_diff_overlay(&self, area: Rect, buf: &mut Buffer, bg: Color, diff: &SwarmAgentDiffState) {
        if area.height == 0 { return; }

        // Header line
        let header = format!(" [diff] {}   Esc: back  j/k: scroll", diff.agent_id);
        let header_style = Style::default().fg(theme::ACCENT).bg(bg).add_modifier(Modifier::BOLD);
        render_text(buf, area.x, area.y, area.right(), &header, header_style);

        if area.height <= 1 { return; }
        let content_area = Rect { y: area.y + 1, height: area.height - 1, ..area };

        if diff.lines.is_empty() {
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            render_text(buf, content_area.x + 1, content_area.y, content_area.right(), " (no diff available)", style);
            return;
        }

        let content_width = content_area.width.saturating_sub(1) as usize; // -1 for scrollbar
        // Word-wrap diff lines so nothing is clipped
        let wrapped_lines: Vec<(DiffLineKind, String)> = diff.lines.iter()
            .flat_map(|(kind, text)| {
                word_wrap_line(text, content_width)
                    .into_iter()
                    .map(move |l| (kind.clone(), l))
            })
            .collect();

        let total = wrapped_lines.len();
        let viewport = content_area.height as usize;
        let scroll = diff.scroll.min(total.saturating_sub(viewport));

        for row in 0..viewport {
            let idx = scroll + row;
            if idx >= total { break; }
            let y = content_area.y + row as u16;
            if y >= content_area.bottom() { break; }

            let (kind, text) = &wrapped_lines[idx];
            let style = match kind {
                DiffLineKind::Added => Style::default().fg(Color::Green).bg(bg),
                DiffLineKind::Removed => Style::default().fg(Color::Red).bg(bg),
                DiffLineKind::Header => Style::default().fg(Color::Yellow).bg(bg).add_modifier(Modifier::BOLD),
                DiffLineKind::Context => Style::default().fg(theme::TEXT_DIM).bg(bg),
            };
            render_text(buf, content_area.x + 1, y, content_area.x + content_width as u16, text, style);
        }

        if total > viewport {
            render_scrollbar(content_area, buf, total, viewport, scroll);
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Cap activity log to MAX_ACTIVITY_LINES.
fn cap_activity(activity: &mut Vec<ActivityLine>) {
    if activity.len() > MAX_ACTIVITY_LINES {
        let drain = activity.len() - MAX_ACTIVITY_LINES;
        activity.drain(..drain);
    }
}

/// Count the number of rendered rows for `activity` when word-wrapped to `width` columns.
/// This matches exactly what `flatten_and_wrap_activity` produces, so scroll limits stay in sync.
pub fn count_display_lines(activity: &[ActivityLine], width: usize) -> usize {
    flatten_and_wrap_activity(activity, width).len()
}

/// Flatten activity lines AND word-wrap each one to `width` columns.
/// The returned vec has one entry per rendered row, suitable for direct indexing
/// in the scroll/viewport calculation.
fn flatten_and_wrap_activity(activity: &[ActivityLine], width: usize) -> Vec<(ActivityKind, String)> {
    let mut lines = Vec::new();
    for entry in activity {
        for line in entry.text.split('\n') {
            if !line.is_empty() {
                for wrapped in word_wrap_line(line, width) {
                    lines.push((entry.kind.clone(), wrapped));
                }
            }
        }
    }
    lines
}

/// Word-wrap a single line to at most `width` characters per output line.
/// Splits on whitespace boundaries; force-breaks words longer than `width`.
/// Leading whitespace (indentation) is preserved on continuation lines.
fn word_wrap_line(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let char_count = text.chars().count();
    if char_count <= width {
        return vec![text.to_string()];
    }

    // Detect leading indent so continuation lines are indented too
    let indent_chars = text.chars().take_while(|c| *c == ' ').count();
    let indent: String = " ".repeat(indent_chars);
    let wrap_width = width.max(1);

    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in text.split_ascii_whitespace() {
        if current_len == 0 {
            // First word on this row — start with indent
            let pfx = if result.is_empty() {
                // first line preserves the original indent (already in text)
                indent_chars
            } else {
                indent_chars
            };
            let _ = pfx; // indent is added explicitly below
            if result.is_empty() {
                current.push_str(&indent);
                current_len = indent_chars;
            } else {
                current.push_str(&indent);
                current_len = indent_chars;
            }
        }

        let available = wrap_width.saturating_sub(current_len);

        if word.chars().count() <= available {
            // Word fits — add a space separator unless we're at the indent start
            if current_len > indent_chars {
                current.push(' ');
                current_len += 1;
            }
            current.push_str(word);
            current_len += word.chars().count();
        } else if current_len == indent_chars {
            // Word is longer than the whole line — force-break it
            let mut remaining = word;
            while !remaining.is_empty() {
                let take = (wrap_width - current_len).max(1);
                // take by chars, not bytes
                let split_at = remaining
                    .char_indices()
                    .nth(take)
                    .map(|(i, _)| i)
                    .unwrap_or(remaining.len());
                current.push_str(&remaining[..split_at]);
                remaining = &remaining[split_at..];
                if !remaining.is_empty() {
                    result.push(std::mem::replace(&mut current, indent.clone()));
                    current_len = indent_chars;
                } else {
                    current_len += take;
                }
            }
        } else {
            // Word doesn't fit — flush current line, start fresh
            result.push(std::mem::replace(&mut current, indent.clone()));
            current_len = indent_chars;

            // Now add the word (may itself need force-breaking)
            let word_len = word.chars().count();
            let line_available = wrap_width - current_len;
            if word_len <= line_available {
                current.push_str(word);
                current_len += word_len;
            } else {
                // Force-break long word
                let mut remaining = word;
                while !remaining.is_empty() {
                    let take = (wrap_width - current_len).max(1);
                    let split_at = remaining
                        .char_indices()
                        .nth(take)
                        .map(|(i, _)| i)
                        .unwrap_or(remaining.len());
                    current.push_str(&remaining[..split_at]);
                    remaining = &remaining[split_at..];
                    if !remaining.is_empty() {
                        result.push(std::mem::replace(&mut current, indent.clone()));
                        current_len = indent_chars;
                    } else {
                        current_len += take;
                    }
                }
            }
        }
    }

    if !current.trim().is_empty() {
        result.push(current);
    } else if result.is_empty() {
        result.push(text.to_string());
    }
    result
}

/// Strip ANSI/VT100 escape sequences from a string.
/// Prevents raw escape chars from corrupting the ratatui cell buffer when
/// rendered character-by-character via `render_text`.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip until end of escape sequence (a letter or `m`/`J`/`K` etc.)
            match chars.peek() {
                Some('[') => {
                    chars.next(); // consume '['
                    // consume until alphabetic char
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() { break; }
                    }
                }
                Some(']') => {
                    chars.next(); // consume ']'
                    // OSC sequence — ends at BEL (\x07) or ST (\x1b\\)
                    for c in chars.by_ref() {
                        if c == '\x07' || c == '\x1b' { break; }
                    }
                }
                _ => {
                    // Single-char escape — skip next char
                    chars.next();
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Parse a unified diff string into classified lines for rendering.
pub fn parse_diff(text: &str) -> Vec<(DiffLineKind, String)> {
    text.lines()
        .map(|line| {
            let kind = if line.starts_with("+++") || line.starts_with("---")
                || line.starts_with("diff ") || line.starts_with("index ")
                || line.starts_with("@@") || line.starts_with("new file")
                || line.starts_with("deleted file")
            {
                DiffLineKind::Header
            } else if line.starts_with('+') {
                DiffLineKind::Added
            } else if line.starts_with('-') {
                DiffLineKind::Removed
            } else {
                DiffLineKind::Context
            };
            (kind, line.to_string())
        })
        .collect()
}

/// Render the undo confirmation message at the bottom of `area`.
fn render_undo_confirm_line(area: Rect, buf: &mut Buffer, bg: Color) {
    let y = area.bottom().saturating_sub(1);
    if y < area.y { return; }
    let msg = " Press u again to confirm UNDO ALL swarm changes. Esc to cancel.";
    let style = Style::default().fg(Color::Yellow).bg(bg).add_modifier(Modifier::BOLD);
    render_text(buf, area.x, y, area.right(), msg, style);
}

/// Format elapsed time for an agent.
fn format_elapsed(agent: &AgentEntry) -> String {
    let Some(started) = agent.started_at else { return String::new() };
    if matches!(agent.status, AgentStatus::Pending) {
        return String::new();
    }
    let secs = started.elapsed().as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Truncate the detail string for the compact table row.
fn truncate_detail(agent: &AgentEntry, max_chars: usize) -> String {
    let detail = if agent.detail.is_empty() {
        match &agent.status {
            AgentStatus::Failed(e) => e.clone(),
            _ => String::new(),
        }
    } else {
        agent.detail.clone()
    };
    if detail.len() > max_chars {
        detail.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…"
    } else {
        detail
    }
}

fn render_text(buf: &mut Buffer, x_start: u16, y: u16, x_max: u16, text: &str, style: Style) {
    let mut x = x_start;
    for ch in text.chars() {
        if x >= x_max {
            break;
        }
        if x < buf.area().right() && y < buf.area().bottom() {
            buf[(x, y)].set_char(ch).set_style(style);
        }
        x += 1;
    }
}
