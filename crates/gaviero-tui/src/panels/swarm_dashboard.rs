use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::theme;
use gaviero_core::swarm::models::{AgentStatus, SwarmResult};

/// An entry in the swarm dashboard table.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub status: AgentStatus,
    pub branch: Option<String>,
    pub detail: String,
    pub modified_files: Vec<String>,
}

/// State for the swarm dashboard panel.
pub struct SwarmDashboardState {
    pub agents: Vec<AgentEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub phase: String,
    pub tier_current: usize,
    pub tier_total: usize,
    pub result: Option<SwarmResult>,
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
        }
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
            entry.status = status.clone();
            entry.detail = detail.to_string();
        } else {
            self.agents.push(AgentEntry {
                id: id.to_string(),
                status: status.clone(),
                branch: Some(format!("gaviero/{}", id)),
                detail: detail.to_string(),
                modified_files: Vec::new(),
            });
        }
    }

    pub fn set_result(&mut self, result: SwarmResult) {
        // Update agent entries with final manifest data
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

    pub fn render(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let fg = theme::TEXT_FG;
        let sel_bg = if focused {
            Color::Rgb(55, 100, 180)
        } else {
            theme::DARK_BG
        };

        // Clear area
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        // Header: phase + tier
        let header = if self.tier_total > 0 {
            format!(" SWARM  Phase: {}  Tier: {}/{}  Agents: {}",
                self.phase, self.tier_current, self.tier_total, self.agents.len())
        } else {
            format!(" SWARM  Phase: {}  Agents: {}", self.phase, self.agents.len())
        };
        let header_style = Style::default()
            .fg(theme::WARNING)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        render_text(buf, area.x, area.y, area.right(), &header, header_style);

        if self.agents.is_empty() {
            let msg = " Press 's' to submit a task, or use --work-units in CLI";
            let style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            if area.y + 2 < area.bottom() {
                render_text(buf, area.x, area.y + 2, area.right(), msg, style);
            }
            return;
        }

        // Agent table
        let table_y = area.y + 1;
        let viewport = (area.height as usize).saturating_sub(1);

        for row in 0..viewport {
            let idx = self.scroll_offset + row;
            if idx >= self.agents.len() {
                break;
            }
            let y = table_y + row as u16;
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

            let icon_style = Style::default().fg(icon_color).bg(row_bg);
            render_text(buf, area.x + 1, y, area.x + 3, icon, icon_style);

            // Agent ID
            let id_style = Style::default().fg(fg).bg(row_bg).add_modifier(Modifier::BOLD);
            render_text(buf, area.x + 3, y, area.x + 20, &agent.id, id_style);

            // Detail
            let detail = if agent.detail.is_empty() {
                match &agent.status {
                    AgentStatus::Failed(e) => e.clone(),
                    _ => String::new(),
                }
            } else {
                agent.detail.clone()
            };
            let detail_style = Style::default().fg(theme::MEDIUM_GRAY).bg(row_bg);
            render_text(buf, area.x + 21, y, area.right(), &detail, detail_style);
        }
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
