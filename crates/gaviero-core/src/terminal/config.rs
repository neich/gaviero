//! Terminal and shell configuration types.

use std::collections::HashMap;
use std::path::PathBuf;

/// Shell type detected from the shell binary path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    Unknown(String),
}

impl ShellType {
    /// Detect shell type from a shell binary path or name.
    pub fn detect(shell: &str) -> Self {
        let basename = std::path::Path::new(shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(shell);
        match basename {
            "bash" => ShellType::Bash,
            "zsh" => ShellType::Zsh,
            "fish" => ShellType::Fish,
            other => ShellType::Unknown(other.to_string()),
        }
    }

    /// Short name for display.
    pub fn name(&self) -> &str {
        match self {
            ShellType::Bash => "bash",
            ShellType::Zsh => "zsh",
            ShellType::Fish => "fish",
            ShellType::Unknown(s) => s,
        }
    }
}

/// Per-tab shell configuration.
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Absolute path to the shell binary.
    pub shell_path: PathBuf,
    /// Detected shell type.
    pub shell_type: ShellType,
    /// Extra arguments to the shell (e.g. `--init-file`).
    pub shell_args: Vec<String>,
    /// Environment variable overrides.
    pub env_overrides: HashMap<String, String>,
    /// Whether to inject OSC 133 / OSC 7 shell integration.
    pub enable_integration: bool,
}

impl ShellConfig {
    /// Build a default config for the user's login shell.
    pub fn default_for_user() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let shell_type = ShellType::detect(&shell);
        Self {
            shell_path: PathBuf::from(&shell),
            shell_type,
            shell_args: Vec::new(),
            env_overrides: default_env(),
            enable_integration: true,
        }
    }

    /// Build a config with a specific shell path.
    pub fn with_shell(shell: &str) -> Self {
        let shell_type = ShellType::detect(shell);
        Self {
            shell_path: PathBuf::from(shell),
            shell_type,
            shell_args: Vec::new(),
            env_overrides: default_env(),
            enable_integration: true,
        }
    }
}

/// Standard environment variables set for all Gaviero terminal instances.
fn default_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("TERM_PROGRAM".into(), "gaviero".into());
    env.insert("GAVIERO_TERMINAL".into(), "1".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env
}

/// Global terminal configuration (from workspace settings).
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Override the default shell (None = use $SHELL).
    pub default_shell: Option<String>,
    /// Scrollback buffer size in lines.
    pub scrollback_lines: u32,
    /// Bounded channel capacity for PTY output events.
    pub channel_capacity: usize,
    /// Resize debounce in milliseconds.
    pub resize_debounce_ms: u64,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            default_shell: None,
            scrollback_lines: 10_000,
            channel_capacity: 256,
            resize_debounce_ms: 50,
        }
    }
}
