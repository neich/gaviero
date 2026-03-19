//! PTY spawning, reader task creation, and CommandBuilder setup.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::config::ShellConfig;
use super::event::TerminalEvent;
use super::types::TerminalId;

/// Handles returned from PTY allocation, before the reader task is started.
pub struct PtyHandle {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub reader: Box<dyn Read + Send>,
}

/// Spawn a PTY with the configured shell.
pub fn spawn_pty(config: &ShellConfig, cwd: &Path, rows: u16, cols: u16) -> Result<PtyHandle> {
    let pty_system = NativePtySystem::default();
    let size = PtySize {
        rows: rows.max(2),
        cols: cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = pty_system
        .openpty(size)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to open PTY")?;

    let cmd = build_command(config, cwd);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to spawn shell")?;
    drop(pair.slave); // Close slave side in parent

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to clone PTY reader")?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to take PTY writer")?;

    Ok(PtyHandle {
        master: pair.master,
        writer,
        child,
        reader,
    })
}

/// Build a `CommandBuilder` from the shell config.
fn build_command(config: &ShellConfig, cwd: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&config.shell_path);
    for arg in &config.shell_args {
        cmd.arg(arg);
    }
    cmd.cwd(cwd);

    // Remove potentially interfering env vars
    cmd.env_remove("TMUX");
    cmd.env_remove("STY");

    // Apply configured env overrides
    for (key, val) in &config.env_overrides {
        cmd.env(key, val);
    }

    cmd
}

/// Spawn a blocking reader task that reads from the PTY and sends raw bytes
/// through a bounded channel. Returns a join handle for cleanup.
///
/// The bounded channel provides natural backpressure: when full, the
/// `blocking_send` call blocks the reader thread, throttling PTY reads.
pub fn spawn_reader_task(
    id: TerminalId,
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<TerminalEvent>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — child exited
                    let _ = tx.blocking_send(TerminalEvent::PtyExited {
                        id,
                        exit_code: None,
                    });
                    break;
                }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    if tx
                        .blocking_send(TerminalEvent::PtyOutput { id, data })
                        .is_err()
                    {
                        // Channel closed — manager was dropped
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.blocking_send(TerminalEvent::PtyExited {
                        id,
                        exit_code: None,
                    });
                    break;
                }
            }
        }
    })
}
