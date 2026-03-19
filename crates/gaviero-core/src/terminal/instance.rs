//! Per-tab terminal instance — owns the vt100 parser, PTY handles, and state.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use portable_pty::PtySize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::config::ShellConfig;
use super::event::TerminalEvent;
use super::osc::{OscParser, OscResult};
use super::types::{CommandRecord, ShellState, TerminalId};

const MAX_COMMAND_HISTORY: usize = 100;

/// A single terminal tab with its PTY, parser, and state.
pub struct TerminalInstance {
    pub id: TerminalId,
    parser: vt100::Parser,
    osc_parser: OscParser,

    // PTY handles (None before spawn)
    pty_writer: Option<Box<dyn Write + Send>>,
    pty_master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_handle: Option<JoinHandle<()>>,

    pub shell_config: ShellConfig,
    pub cwd: PathBuf,
    pub title: String,
    pub shell_state: ShellState,
    pub rows: u16,
    pub cols: u16,
    pub is_dirty: bool,
    pub spawned: bool,

    /// Bounded ring buffer of recent command records.
    pub command_history: VecDeque<CommandRecord>,
    /// Timestamp when the current running command started.
    current_command_start: Option<Instant>,
    /// Text of the current running command (captured between OSC 133;B and C).
    current_command_text: Option<String>,
}

impl TerminalInstance {
    /// Create a new instance (not yet spawned).
    pub fn new(
        id: TerminalId,
        config: ShellConfig,
        cwd: PathBuf,
        rows: u16,
        cols: u16,
        scrollback: u32,
    ) -> Self {
        Self {
            id,
            parser: vt100::Parser::new(rows, cols, scrollback as usize),
            osc_parser: OscParser::new(),
            pty_writer: None,
            pty_master: None,
            child: None,
            reader_handle: None,
            shell_config: config,
            cwd,
            title: String::new(),
            shell_state: ShellState::Unknown,
            rows,
            cols,
            is_dirty: false,
            spawned: false,
            command_history: VecDeque::with_capacity(MAX_COMMAND_HISTORY),
            current_command_start: None,
            current_command_text: None,
        }
    }

    /// Spawn the shell subprocess and start the reader task.
    pub fn spawn(
        &mut self,
        event_tx: mpsc::Sender<TerminalEvent>,
    ) -> Result<()> {
        if self.spawned {
            return Ok(());
        }

        let handle = super::pty::spawn_pty(&self.shell_config, &self.cwd, self.rows, self.cols)?;

        self.pty_writer = Some(handle.writer);
        self.pty_master = Some(handle.master);
        self.child = Some(handle.child);
        self.spawned = true;

        // Reset parser to match current dimensions
        self.parser = vt100::Parser::new(self.rows, self.cols, 10_000);

        // Start the reader task
        let reader_handle = super::pty::spawn_reader_task(self.id, handle.reader, event_tx);
        self.reader_handle = Some(reader_handle);

        Ok(())
    }

    /// Process raw PTY output bytes: run through OSC parser then vt100.
    /// Returns any extracted OSC results for the manager to handle.
    pub fn process_output(&mut self, data: &[u8]) -> Vec<OscResult> {
        let (results, clean) = self.osc_parser.feed(data);
        if !clean.is_empty() {
            self.parser.process(&clean);
        }
        self.is_dirty = true;
        results
    }

    /// Send raw bytes to the PTY (user keystrokes).
    pub fn write_input(&mut self, data: &[u8]) {
        if let Some(writer) = &mut self.pty_writer {
            let _ = writer.write_all(data);
            let _ = writer.flush();
        }
    }

    /// Resize the PTY and vt100 parser.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == (self.rows, self.cols) {
            return;
        }
        self.rows = rows;
        self.cols = cols;

        if let Some(master) = &self.pty_master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Kill the PTY child process and clean up.
    pub fn kill(&mut self) {
        self.pty_writer.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.pty_master.take();
        self.spawned = false;
        // reader_handle will finish naturally when the PTY is closed
    }

    /// Access the vt100 screen for rendering.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Mark the instance as not dirty (after rendering).
    pub fn clear_dirty(&mut self) {
        self.is_dirty = false;
    }

    /// Record that a command started (called when OSC 133;C is detected).
    pub fn start_command(&mut self, command: String) {
        self.current_command_text = Some(command);
        self.current_command_start = Some(Instant::now());
        self.shell_state = ShellState::Running {
            command: self.current_command_text.clone().unwrap_or_default(),
            started: Instant::now(),
        };
    }

    /// Record that a command finished (called when OSC 133;D is detected).
    pub fn finish_command(&mut self, exit_code: Option<i32>) {
        let duration_ms = self
            .current_command_start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let command = self.current_command_text.take().unwrap_or_default();
        if !command.is_empty() {
            // Capture a preview of the last few lines of output
            let output_preview = self.recent_output_preview(5);

            let record = CommandRecord {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                command,
                exit_code,
                cwd: self.cwd.clone(),
                duration_ms,
                output_preview,
            };

            if self.command_history.len() >= MAX_COMMAND_HISTORY {
                self.command_history.pop_front();
            }
            self.command_history.push_back(record);
        }

        self.current_command_start = None;
        self.shell_state = ShellState::Finished { exit_code };
    }

    /// Get the last N lines of terminal content as plain text.
    fn recent_output_preview(&self, max_lines: usize) -> String {
        let contents = self.parser.screen().contents();
        let lines: Vec<&str> = contents.lines().collect();
        let start = lines.len().saturating_sub(max_lines);
        lines[start..]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for TerminalInstance {
    fn drop(&mut self) {
        self.kill();
    }
}
