//! TerminalManager — owns all terminal instances, tab ordering, event routing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use super::config::{ShellConfig, TerminalConfig};
use super::event::TerminalEvent;
use super::instance::TerminalInstance;
use super::osc::{Osc133Marker, OscResult};
use super::types::{ShellState, TerminalId};

/// Central manager for all terminal tabs.
pub struct TerminalManager {
    pub(crate) terminals: HashMap<TerminalId, TerminalInstance>,
    tab_order: Vec<TerminalId>,
    active_tab: Option<TerminalId>,

    /// Sender side of the bounded event channel (cloned to reader tasks).
    event_tx: mpsc::Sender<TerminalEvent>,
    /// Receiver side — taken once by the TUI to bridge into its unified event loop.
    event_rx: Option<mpsc::Receiver<TerminalEvent>>,

    viewport_rows: u16,
    viewport_cols: u16,
    config: TerminalConfig,

    /// Pending resize: (rows, cols, timestamp). Applied after debounce window.
    pending_resize: Option<(u16, u16, Instant)>,
}

impl TerminalManager {
    /// Create a new manager with the given configuration.
    pub fn new(config: TerminalConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        Self {
            terminals: HashMap::new(),
            tab_order: Vec::new(),
            active_tab: None,
            event_tx: tx,
            event_rx: Some(rx),
            viewport_rows: 24,
            viewport_cols: 80,
            config,
            pending_resize: None,
        }
    }

    /// Take the event receiver (called once by the TUI to set up the bridge).
    pub fn take_event_rx(&mut self) -> mpsc::Receiver<TerminalEvent> {
        self.event_rx
            .take()
            .expect("TerminalManager::take_event_rx called more than once")
    }

    /// The sender side, for external use (e.g. tests).
    pub fn event_tx(&self) -> &mpsc::Sender<TerminalEvent> {
        &self.event_tx
    }

    /// Create a new terminal tab, spawn the shell, and return the tab ID.
    pub fn create_tab(&mut self, cwd: &Path) -> Result<TerminalId> {
        let id = TerminalId::next();
        let mut shell_config = match &self.config.default_shell {
            Some(shell) => ShellConfig::with_shell(shell),
            None => ShellConfig::default_for_user(),
        };

        // Set up shell integration (OSC 133/7) and per-tab history
        let _ = super::history::ensure_history_dir();
        let histfile = super::history::history_file_for(&id);
        shell_config
            .env_overrides
            .insert("HISTFILE".into(), histfile.to_string_lossy().into_owned());

        if shell_config.enable_integration {
            if let Ok(init_path) =
                super::shell_integration::create_init_file(&shell_config.shell_type, &id, &histfile)
            {
                super::shell_integration::build_shell_args(&mut shell_config, &init_path);
            }
        }

        let mut instance = TerminalInstance::new(
            id,
            shell_config,
            cwd.to_path_buf(),
            self.viewport_rows,
            self.viewport_cols,
            self.config.scrollback_lines,
        );
        instance
            .spawn(self.event_tx.clone())
            .context("failed to spawn terminal")?;

        self.terminals.insert(id, instance);
        self.tab_order.push(id);

        // Auto-activate if this is the first tab
        if self.active_tab.is_none() {
            self.active_tab = Some(id);
        }

        Ok(id)
    }

    /// Create a tab without spawning it (for lazy spawn on focus).
    pub fn create_tab_lazy(&mut self, cwd: &Path) -> TerminalId {
        let id = TerminalId::next();
        let mut shell_config = match &self.config.default_shell {
            Some(shell) => ShellConfig::with_shell(shell),
            None => ShellConfig::default_for_user(),
        };

        // Set up shell integration and history (even for lazy tabs)
        let _ = super::history::ensure_history_dir();
        let histfile = super::history::history_file_for(&id);
        shell_config
            .env_overrides
            .insert("HISTFILE".into(), histfile.to_string_lossy().into_owned());

        if shell_config.enable_integration {
            if let Ok(init_path) =
                super::shell_integration::create_init_file(&shell_config.shell_type, &id, &histfile)
            {
                super::shell_integration::build_shell_args(&mut shell_config, &init_path);
            }
        }

        let instance = TerminalInstance::new(
            id,
            shell_config,
            cwd.to_path_buf(),
            self.viewport_rows,
            self.viewport_cols,
            self.config.scrollback_lines,
        );

        self.terminals.insert(id, instance);
        self.tab_order.push(id);

        if self.active_tab.is_none() {
            self.active_tab = Some(id);
        }

        id
    }

    /// Ensure the active tab is spawned (lazy spawn on focus).
    pub fn ensure_active_spawned(&mut self) -> Result<()> {
        if let Some(id) = self.active_tab {
            if let Some(inst) = self.terminals.get_mut(&id) {
                if !inst.spawned {
                    inst.spawn(self.event_tx.clone())?;
                }
            }
        }
        Ok(())
    }

    /// Close a terminal tab.
    pub fn close_tab(&mut self, id: TerminalId) {
        if let Some(mut instance) = self.terminals.remove(&id) {
            instance.kill();
        }
        self.tab_order.retain(|&tid| tid != id);

        // Update active tab
        if self.active_tab == Some(id) {
            self.active_tab = self.tab_order.last().copied();
        }
    }

    /// Get the active terminal instance (immutable).
    pub fn active_instance(&self) -> Option<&TerminalInstance> {
        self.active_tab
            .and_then(|id| self.terminals.get(&id))
    }

    /// Get the active terminal instance (mutable).
    pub fn active_instance_mut(&mut self) -> Option<&mut TerminalInstance> {
        self.active_tab
            .and_then(|id| self.terminals.get_mut(&id))
    }

    /// Get an instance by ID.
    pub fn instance(&self, id: TerminalId) -> Option<&TerminalInstance> {
        self.terminals.get(&id)
    }

    /// Switch to a specific tab. Lazy-resizes if dimensions mismatch.
    pub fn switch_tab(&mut self, id: TerminalId) {
        if !self.terminals.contains_key(&id) {
            return;
        }
        self.active_tab = Some(id);

        // Lazy resize: if this tab's dimensions don't match the viewport, resize it
        if let Some(inst) = self.terminals.get_mut(&id) {
            if inst.rows != self.viewport_rows || inst.cols != self.viewport_cols {
                inst.resize(self.viewport_rows, self.viewport_cols);
            }
        }
    }

    /// Cycle through tabs by delta (+1 = next, -1 = prev).
    pub fn cycle_tab(&mut self, delta: i32) {
        if self.tab_order.is_empty() {
            return;
        }
        let current_idx = self
            .active_tab
            .and_then(|id| self.tab_order.iter().position(|&tid| tid == id))
            .unwrap_or(0);
        let len = self.tab_order.len() as i32;
        let new_idx = ((current_idx as i32 + delta).rem_euclid(len)) as usize;
        let new_id = self.tab_order[new_idx];
        self.switch_tab(new_id);
    }

    /// Handle a viewport resize event. Only resizes the active tab immediately;
    /// stores pending dimensions for debouncing.
    pub fn handle_resize(&mut self, rows: u16, cols: u16) {
        self.viewport_rows = rows;
        self.viewport_cols = cols;
        self.pending_resize = Some((rows, cols, Instant::now()));

        // Resize active tab immediately
        if let Some(inst) = self.active_instance_mut() {
            inst.resize(rows, cols);
        }
    }

    /// Called on each tick — applies debounced resize if the window has been stable.
    pub fn tick(&mut self) {
        if let Some((rows, cols, when)) = self.pending_resize {
            let debounce = std::time::Duration::from_millis(self.config.resize_debounce_ms);
            if when.elapsed() >= debounce {
                // Resize all non-active terminals that are mismatched
                let active = self.active_tab;
                for (&id, inst) in &mut self.terminals {
                    if Some(id) != active
                        && inst.spawned
                        && (inst.rows != rows || inst.cols != cols)
                    {
                        inst.resize(rows, cols);
                    }
                }
                self.pending_resize = None;
            }
        }
    }

    /// Process a terminal event from the bounded channel.
    pub fn process_event(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::PtyOutput { id, data } => {
                if let Some(inst) = self.terminals.get_mut(&id) {
                    let osc_results = inst.process_output(&data);
                    self.handle_osc_results(id, osc_results);
                }
            }
            TerminalEvent::PtyExited { id, exit_code: _ } => {
                if let Some(inst) = self.terminals.get_mut(&id) {
                    inst.spawned = false;
                    inst.is_dirty = true;
                }
            }
            // These events are informational — the manager already updated state
            // when processing OSC results. They exist for the TUI to react to.
            TerminalEvent::ShellStateChanged { .. }
            | TerminalEvent::CwdChanged { .. }
            | TerminalEvent::CommandStarted { .. }
            | TerminalEvent::CommandFinished { .. }
            | TerminalEvent::TitleChanged { .. } => {}
        }
    }

    /// Handle OSC results extracted during output processing.
    fn handle_osc_results(&mut self, id: TerminalId, results: Vec<OscResult>) {
        for result in results {
            match result {
                OscResult::Osc133(marker) => {
                    if let Some(inst) = self.terminals.get_mut(&id) {
                        match marker {
                            Osc133Marker::PromptStart => {
                                inst.shell_state = ShellState::PromptDrawing;
                            }
                            Osc133Marker::CommandInputStart => {
                                inst.shell_state = ShellState::Idle;
                            }
                            Osc133Marker::CommandOutputStart => {
                                // Extract command text from screen contents between
                                // the prompt and cursor position.
                                let cmd = extract_command_from_screen(inst.screen());
                                inst.start_command(cmd);
                            }
                            Osc133Marker::CommandFinished { exit_code } => {
                                inst.finish_command(exit_code);
                            }
                        }
                    }
                }
                OscResult::Osc7(url) => {
                    if let Some(inst) = self.terminals.get_mut(&id) {
                        // Parse file://host/path → PathBuf
                        if let Some(path) = parse_osc7_path(&url) {
                            inst.cwd = path;
                        }
                    }
                }
                OscResult::Title(title) => {
                    if let Some(inst) = self.terminals.get_mut(&id) {
                        inst.title = title;
                    }
                }
            }
        }
    }

    /// Number of open terminal tabs.
    pub fn tab_count(&self) -> usize {
        self.tab_order.len()
    }

    /// Ordered tab IDs.
    pub fn tab_order(&self) -> &[TerminalId] {
        &self.tab_order
    }

    /// Active tab ID.
    pub fn active_tab(&self) -> Option<TerminalId> {
        self.active_tab
    }

    /// Whether there are no terminal tabs.
    pub fn is_empty(&self) -> bool {
        self.tab_order.is_empty()
    }

    /// Active tab index within tab_order.
    pub fn active_tab_index(&self) -> usize {
        self.active_tab
            .and_then(|id| self.tab_order.iter().position(|&tid| tid == id))
            .unwrap_or(0)
    }
}

/// Extract the command text from the current screen line (heuristic).
/// This captures text from the last line that contains the cursor.
fn extract_command_from_screen(screen: &vt100::Screen) -> String {
    let (row, _col) = screen.cursor_position();
    // Get the full line at cursor row
    let mut line = String::new();
    for col in 0..screen.size().1 {
        if let Some(cell) = screen.cell(row, col) {
            let contents = cell.contents();
            if contents.is_empty() {
                line.push(' ');
            } else {
                line.push_str(&contents);
            }
        }
    }
    // Trim the line; the command is typically after the prompt
    // Look for common prompt endings: $, %, >, #
    let trimmed = line.trim_end();
    if let Some(pos) = trimmed.rfind(|c| matches!(c, '$' | '%' | '>' | '#')) {
        trimmed[pos + 1..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Parse an OSC 7 URL (file://hostname/path) into a PathBuf.
fn parse_osc7_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // Skip hostname (everything up to the next /)
    let path_start = rest.find('/')?;
    let path_str = &rest[path_start..];
    // URL-decode percent-encoded characters
    let decoded = percent_decode(path_str);
    Some(PathBuf::from(decoded))
}

/// Simple percent-decoding for file paths.
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next().unwrap_or('0');
            let lo = chars.next().unwrap_or('0');
            if let Ok(byte) = u8::from_str_radix(&format!("{hi}{lo}"), 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push(hi);
                result.push(lo);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc7_simple() {
        let path = parse_osc7_path("file://localhost/home/user/project");
        assert_eq!(path, Some(PathBuf::from("/home/user/project")));
    }

    #[test]
    fn parse_osc7_no_host() {
        let path = parse_osc7_path("file:///tmp/test");
        assert_eq!(path, Some(PathBuf::from("/tmp/test")));
    }

    #[test]
    fn parse_osc7_percent_encoded() {
        let path = parse_osc7_path("file://host/home/user/my%20project");
        assert_eq!(path, Some(PathBuf::from("/home/user/my project")));
    }

    #[test]
    fn parse_osc7_invalid() {
        assert!(parse_osc7_path("not-a-url").is_none());
    }
}
