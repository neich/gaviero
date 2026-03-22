use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::theme;
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
    pub selected: usize,
    pub scroll_offset: usize,
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
}

impl SwarmDashboardState {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            selected: 0,
            scroll_offset: 0,
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
        }
    }

    /// Reset all state for a new swarm run.
    pub fn reset(&mut self, phase: &str) {
        self.agents.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.phase = phase.to_string();
        self.tier_current = 0;
        self.tier_total = 0;
        self.result = None;
        self.estimated_cost_usd = 0.0;
        self.status_message = String::new();
        self.detail_scroll = 0;
        self.detail_auto_scroll = true;
        self.focus = DashboardFocus::Table;
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
            }
        }
        self.result = Some(result);
    }

    // ── Event intake methods ────────────────────────────────────

    /// Append streaming text from an agent. Coalesces consecutive Text lines.
    pub fn append_stream_chunk(&mut self, agent_id: &str, text: &str) {
        let Some(entry) = self.agents.iter_mut().find(|a| a.id == agent_id) else { return };

        // Coalesce with last Text line
        if let Some(last) = entry.activity.last_mut() {
            if last.kind == ActivityKind::Text {
                last.text.push_str(text);
                return;
            }
        }
        entry.activity.push(ActivityLine {
            kind: ActivityKind::Text,
            text: text.to_string(),
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
            Color::Rgb(55, 100, 180)
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
            let idx = self.scroll_offset + row;
            if idx >= self.agents.len() {
                break;
            }
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }

            let agent = &self.agents[idx];
            let is_selected = idx == self.selected;
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
                    ModelTier::Coordinator => ("C", Color::Rgb(180, 120, 220)),
                    ModelTier::Reasoning => ("R", Color::Rgb(80, 160, 230)),
                    ModelTier::Execution => ("E", Color::Rgb(80, 200, 120)),
                    ModelTier::Mechanical => ("M", Color::Rgb(220, 200, 80)),
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
            if !elapsed_str.is_empty() {
                let elapsed_x = area.right().saturating_sub(elapsed_str.len() as u16 + 1);
                if elapsed_x > col {
                    render_text(buf, elapsed_x, y, area.right(), &elapsed_str,
                        Style::default().fg(theme::TEXT_DIM).bg(row_bg));
                }
                // Detail fills between col and elapsed
                let detail = truncate_detail(agent, (elapsed_x.saturating_sub(col).saturating_sub(1)) as usize);
                render_text(buf, col, y, elapsed_x.saturating_sub(1), &detail,
                    Style::default().fg(theme::MEDIUM_GRAY).bg(row_bg));
            } else {
                let detail = truncate_detail(agent, (area.right().saturating_sub(col)) as usize);
                render_text(buf, col, y, area.right(), &detail,
                    Style::default().fg(theme::MEDIUM_GRAY).bg(row_bg));
            }
        }
    }

    fn render_separator(&self, x: u16, y: u16, width: u16, buf: &mut Buffer, bg: Color) {
        if y >= buf.area().bottom() {
            return;
        }
        let agent_name = self.agents.get(self.selected)
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
        let Some(agent) = self.agents.get(self.selected) else { return };

        if agent.activity.is_empty() {
            let msg = " Waiting for activity...";
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            if area.y < area.bottom() {
                render_text(buf, area.x, area.y, area.right(), msg, style);
            }
            return;
        }

        // Flatten activity into display lines (split on newlines)
        let display_lines = flatten_activity(&agent.activity);
        let total = display_lines.len();
        let viewport = area.height as usize;

        // Auto-scroll: show the bottom
        let scroll = if self.detail_auto_scroll {
            total.saturating_sub(viewport)
        } else {
            self.detail_scroll.min(total.saturating_sub(viewport))
        };

        // Render visible lines
        let content_width = area.width.saturating_sub(1); // -1 for scrollbar
        for row in 0..viewport {
            let idx = scroll + row;
            if idx >= total {
                break;
            }
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }

            let (kind, text) = &display_lines[idx];
            let style = match kind {
                ActivityKind::ToolCall => Style::default()
                    .fg(Color::Rgb(80, 200, 220))
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
                ActivityKind::FileChange => Style::default()
                    .fg(theme::SUCCESS)
                    .bg(bg),
                ActivityKind::Status => Style::default()
                    .fg(Color::Rgb(200, 180, 80))
                    .bg(bg),
                ActivityKind::Text => Style::default()
                    .fg(theme::TEXT_FG)
                    .bg(bg),
            };

            // Truncate text to content width
            let display: String = text.chars().take(content_width as usize).collect();
            render_text(buf, area.x + 1, y, area.x + content_width, &display, style);
        }

        // Scrollbar
        if total > viewport {
            render_scrollbar(area, buf, total, viewport, scroll);
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

/// Flatten activity lines by splitting on newlines for rendering.
/// Count the number of display lines in an activity log (splitting on newlines).
pub fn count_display_lines(activity: &[ActivityLine]) -> usize {
    activity.iter()
        .map(|a| a.text.split('\n').filter(|l| !l.is_empty()).count().max(1))
        .sum()
}

fn flatten_activity(activity: &[ActivityLine]) -> Vec<(ActivityKind, String)> {
    let mut lines = Vec::new();
    for entry in activity {
        for line in entry.text.split('\n') {
            if !line.is_empty() {
                lines.push((entry.kind.clone(), line.to_string()));
            }
        }
    }
    lines
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
