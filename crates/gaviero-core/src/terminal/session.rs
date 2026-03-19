//! Terminal session persistence — save/restore tab metadata across sessions.

use serde::{Deserialize, Serialize};

use super::instance::TerminalInstance;
use super::manager::TerminalManager;

/// Serializable state for a single terminal tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalTabState {
    /// Terminal ID (stringified for serialization).
    pub id: String,
    /// Tab title.
    pub title: String,
    /// Working directory.
    pub cwd: String,
    /// Shell type name (bash, zsh, fish, etc.).
    pub shell_type: String,
    /// Path to the per-tab history file.
    pub history_file: String,
}

/// Serializable state for all terminal tabs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerminalSessionState {
    /// Terminal tabs in order.
    #[serde(default)]
    pub tabs: Vec<TerminalTabState>,
    /// Active tab ID.
    #[serde(default)]
    pub active_tab: Option<String>,
}

impl From<&TerminalInstance> for TerminalTabState {
    fn from(inst: &TerminalInstance) -> Self {
        Self {
            id: inst.id.to_string(),
            title: inst.title.clone(),
            cwd: inst.cwd.to_string_lossy().into_owned(),
            shell_type: inst.shell_config.shell_type.name().to_string(),
            history_file: inst
                .shell_config
                .env_overrides
                .get("HISTFILE")
                .cloned()
                .unwrap_or_default(),
        }
    }
}

impl TerminalManager {
    /// Capture the current terminal state for session persistence.
    pub fn save_state(&self) -> TerminalSessionState {
        let tabs: Vec<TerminalTabState> = self
            .tab_order()
            .iter()
            .filter_map(|&id| self.instance(id))
            .map(TerminalTabState::from)
            .collect();

        let active_tab = self.active_tab().map(|id| id.to_string());

        TerminalSessionState { tabs, active_tab }
    }

    /// Restore terminal tabs from saved session state.
    /// Creates lazy (unspawned) tabs with the saved CWDs and history files.
    pub fn restore_state(&mut self, state: &TerminalSessionState) {
        for tab in &state.tabs {
            let cwd = std::path::PathBuf::from(&tab.cwd);
            if !cwd.exists() {
                continue; // Skip tabs whose CWD no longer exists
            }
            let id = self.create_tab_lazy(&cwd);

            // Apply saved metadata
            if let Some(inst) = self.terminals.get_mut(&id) {
                inst.title = tab.title.clone();
                if !tab.history_file.is_empty() {
                    inst.shell_config
                        .env_overrides
                        .insert("HISTFILE".into(), tab.history_file.clone());
                }
            }
        }

        // Restore active tab (switch to the first tab if the saved one isn't found)
        if !self.tab_order().is_empty() {
            if let Some(active_id_str) = &state.active_tab {
                // Try to find a tab that matches — but since IDs are regenerated,
                // we just activate by position
                let active_idx = state
                    .tabs
                    .iter()
                    .position(|t| t.id == *active_id_str)
                    .unwrap_or(0);
                if active_idx < self.tab_order().len() {
                    let id = self.tab_order()[active_idx];
                    self.switch_tab(id);
                }
            }
        }
    }

    /// Extract AI context from the active terminal (convenience method).
    pub fn active_context(&self) -> Option<super::context::TerminalContext> {
        self.active_instance().map(|inst| inst.extract_context())
    }
}
