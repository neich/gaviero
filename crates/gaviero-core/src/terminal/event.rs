//! Terminal event types sent from PTY reader tasks to the main loop.

use std::path::PathBuf;

use super::types::{ShellState, TerminalId};

/// Events produced by terminal background tasks, consumed by the main loop.
#[derive(Debug)]
pub enum TerminalEvent {
    /// Raw PTY output bytes ready for parsing.
    PtyOutput {
        id: TerminalId,
        data: Vec<u8>,
    },
    /// PTY child process exited.
    PtyExited {
        id: TerminalId,
        exit_code: Option<i32>,
    },
    /// Shell state changed (driven by OSC 133 markers).
    ShellStateChanged {
        id: TerminalId,
        state: ShellState,
    },
    /// Working directory changed (driven by OSC 7).
    CwdChanged {
        id: TerminalId,
        cwd: PathBuf,
    },
    /// A command started executing (OSC 133;C).
    CommandStarted {
        id: TerminalId,
        command: String,
    },
    /// A command finished executing (OSC 133;D).
    CommandFinished {
        id: TerminalId,
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    /// Terminal title changed (OSC 0/2).
    TitleChanged {
        id: TerminalId,
        title: String,
    },
}
