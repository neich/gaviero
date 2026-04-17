//! Terminal subsystem — PTY management, shell integration, command detection.
//!
//! This module implements the Manager → Instance hierarchy described in
//! TERMINAL_ARCHITECTURE.md. All pipeline logic lives here in gaviero-core;
//! the TUI crate only handles rendering and input mapping.

pub mod config;
pub mod context;
pub mod event;
pub mod history;
pub mod instance;
pub mod manager;
pub mod osc;
pub mod pty;
pub mod session;
pub mod shell_integration;
pub mod types;

pub use config::{ShellConfig, ShellType, TerminalConfig};
pub use event::TerminalEvent;
pub use instance::TerminalInstance;
pub use manager::TerminalManager;
pub use types::{CommandRecord, ShellState, TerminalId};
