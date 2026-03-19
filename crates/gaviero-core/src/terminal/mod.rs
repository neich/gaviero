//! Terminal subsystem — PTY management, shell integration, command detection.
//!
//! This module implements the Manager → Instance hierarchy described in
//! TERMINAL_ARCHITECTURE.md. All pipeline logic lives here in gaviero-core;
//! the TUI crate only handles rendering and input mapping.

pub mod types;
pub mod config;
pub mod event;
pub mod osc;
pub mod pty;
pub mod instance;
pub mod manager;
pub mod shell_integration;
pub mod history;
pub mod context;
pub mod session;

pub use config::{ShellConfig, ShellType, TerminalConfig};
pub use event::TerminalEvent;
pub use types::{CommandRecord, ShellState, TerminalId};
pub use instance::TerminalInstance;
pub use manager::TerminalManager;
