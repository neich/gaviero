//! AI context extraction from terminal state.
//!
//! Provides a `TerminalContext` struct that can be included in agent prompts
//! to give the LLM awareness of the terminal's current state.

use std::path::PathBuf;

use super::instance::TerminalInstance;
use super::types::{CommandRecord, ShellState};

/// Structured snapshot of terminal state for AI agent context.
#[derive(Debug, Clone)]
pub struct TerminalContext {
    /// Current working directory (from OSC 7).
    pub cwd: PathBuf,
    /// Shell lifecycle state.
    pub shell_state: ShellState,
    /// Recent command records (last 10).
    pub last_commands: Vec<CommandRecord>,
    /// Recent terminal output (ANSI-stripped, last ~200 lines, ~4000 chars).
    pub recent_output: String,
    /// Terminal dimensions (rows, cols).
    pub terminal_dimensions: (u16, u16),
}

/// Maximum characters in the recent_output field.
const MAX_OUTPUT_CHARS: usize = 4000;
/// Maximum lines to capture from screen contents.
const MAX_OUTPUT_LINES: usize = 200;

impl TerminalInstance {
    /// Extract a snapshot of this terminal's state for AI context.
    pub fn extract_context(&self) -> TerminalContext {
        let contents = self.screen().contents();
        let lines: Vec<&str> = contents.lines().collect();
        let start = lines.len().saturating_sub(MAX_OUTPUT_LINES);
        let mut recent = lines[start..]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate to max chars
        if recent.len() > MAX_OUTPUT_CHARS {
            let truncation_point = recent
                .char_indices()
                .nth(MAX_OUTPUT_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(recent.len());
            recent.truncate(truncation_point);
        }

        let last_commands: Vec<CommandRecord> = self
            .command_history
            .iter()
            .rev()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        TerminalContext {
            cwd: self.cwd.clone(),
            shell_state: self.shell_state.clone(),
            last_commands,
            recent_output: recent,
            terminal_dimensions: (self.rows, self.cols),
        }
    }
}
