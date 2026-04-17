//! Fundamental types for the terminal subsystem.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a terminal tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalId(u64);

impl TerminalId {
    /// Generate a new unique terminal ID.
    pub fn next() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Create from a raw u64 (for deserialization / session restore).
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// The raw numeric value.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Shell lifecycle state, driven by OSC 133 markers.
#[derive(Debug, Clone)]
pub enum ShellState {
    /// No shell integration detected yet.
    Unknown,
    /// OSC 133;A received — shell is drawing the prompt.
    PromptDrawing,
    /// OSC 133;B received — prompt rendered, awaiting input.
    Idle,
    /// OSC 133;C received — a command is executing.
    Running { command: String, started: Instant },
    /// OSC 133;D received — command finished, exit code available.
    Finished { exit_code: Option<i32> },
}

impl ShellState {
    /// Returns `true` if the shell is idle (waiting for user input).
    pub fn is_idle(&self) -> bool {
        matches!(self, ShellState::Idle | ShellState::Unknown)
    }

    /// Returns `true` if a command is currently running.
    pub fn is_running(&self) -> bool {
        matches!(self, ShellState::Running { .. })
    }
}

/// A recorded command execution captured via OSC 133 markers.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    /// Unix timestamp (seconds since epoch).
    pub timestamp: u64,
    /// The command text that was executed.
    pub command: String,
    /// Exit code, if available (from OSC 133;D).
    pub exit_code: Option<i32>,
    /// Working directory at time of execution.
    pub cwd: PathBuf,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// First N lines of output (truncated preview).
    pub output_preview: String,
}
