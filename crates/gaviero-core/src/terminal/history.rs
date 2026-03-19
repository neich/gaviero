//! Per-tab history file management and command log.

use std::collections::VecDeque;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::types::{CommandRecord, TerminalId};

/// Base directory for per-tab shell history files.
pub fn history_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("gaviero/history")
}

/// History file path for a specific terminal tab.
pub fn history_file_for(id: &TerminalId) -> PathBuf {
    history_dir().join(format!("tab-{}", id))
}

/// Ensure the history directory exists.
pub fn ensure_history_dir() -> Result<()> {
    std::fs::create_dir_all(history_dir()).context("creating history directory")?;
    Ok(())
}

/// Bounded ring buffer of recent command records.
#[derive(Debug)]
pub struct CommandLog {
    entries: VecDeque<CommandRecord>,
    max: usize,
}

impl CommandLog {
    pub fn new(max: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max),
            max,
        }
    }

    /// Push a new record, evicting the oldest if at capacity.
    pub fn push(&mut self, record: CommandRecord) {
        if self.entries.len() >= self.max {
            self.entries.pop_front();
        }
        self.entries.push_back(record);
    }

    /// Get the most recent N records.
    pub fn recent(&self, n: usize) -> Vec<&CommandRecord> {
        let start = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(start).collect()
    }

    /// All entries as a slice.
    pub fn entries(&self) -> &VecDeque<CommandRecord> {
        &self.entries
    }
}
