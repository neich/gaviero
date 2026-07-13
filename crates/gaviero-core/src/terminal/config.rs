//! Terminal and shell configuration types.

use std::collections::HashMap;
use std::path::PathBuf;

/// Shell type detected from the shell binary path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    /// PowerShell 7+ (`pwsh`) — the default and only integrated
    /// PowerShell on Windows (Tier W1 / PR-3). Windows PowerShell 5.1
    /// deliberately detects as `Unknown` (unsupported).
    PowerShell,
    Unknown(String),
}

impl ShellType {
    /// Detect shell type from a shell binary path or name. Matching is
    /// case-insensitive and ignores a `.exe` suffix so `pwsh`,
    /// `pwsh.exe`, and Git Bash's `bash.exe` all detect correctly.
    pub fn detect(shell: &str) -> Self {
        let basename = std::path::Path::new(shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(shell);
        let normalized = basename
            .to_ascii_lowercase()
            .trim_end_matches(".exe")
            .to_string();
        match normalized.as_str() {
            "bash" => ShellType::Bash,
            "zsh" => ShellType::Zsh,
            "fish" => ShellType::Fish,
            "pwsh" => ShellType::PowerShell,
            _ => ShellType::Unknown(basename.to_string()),
        }
    }

    /// Short name for display.
    pub fn name(&self) -> &str {
        match self {
            ShellType::Bash => "bash",
            ShellType::Zsh => "zsh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "pwsh",
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
    ///
    /// Unix: `$SHELL`, falling back to `/bin/sh`. Windows: PowerShell
    /// 7.2+ (`pwsh`) is a hard runtime requirement — resolved from
    /// PATH, then `%ProgramFiles%\PowerShell\7\pwsh.exe`. When neither
    /// resolves, the config keeps the bare `pwsh` name and the version
    /// preflight at spawn time produces the actionable install error.
    pub fn default_for_user() -> Self {
        #[cfg(not(windows))]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            Self::with_shell(&shell)
        }
        #[cfg(windows)]
        {
            let shell_path = crate::util::spawn::resolve_program("pwsh").or_else(|| {
                let pf = std::env::var("ProgramFiles").ok()?;
                let candidate = PathBuf::from(pf).join(r"PowerShell\7\pwsh.exe");
                candidate.is_file().then_some(candidate)
            });
            let shell_path = shell_path.unwrap_or_else(|| PathBuf::from("pwsh"));
            Self {
                shell_path,
                shell_type: ShellType::PowerShell,
                shell_args: Vec::new(),
                env_overrides: default_env_for(&ShellType::PowerShell),
                enable_integration: true,
            }
        }
    }

    /// Build a config with a specific shell path.
    pub fn with_shell(shell: &str) -> Self {
        let shell_type = ShellType::detect(shell);
        let env_overrides = default_env_for(&shell_type);
        Self {
            shell_path: PathBuf::from(shell),
            shell_type,
            shell_args: Vec::new(),
            env_overrides,
            enable_integration: true,
        }
    }
}

/// Standard environment variables for Gaviero terminal instances.
/// PowerShell on Windows skips `TERM`/`COLORTERM` (Unix-isms — pwsh
/// derives ANSI support itself and third-party modules can misread
/// them); the Bash family keeps them, including Git Bash under ConPTY.
fn default_env_for(shell_type: &ShellType) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if !(cfg!(windows) && *shell_type == ShellType::PowerShell) {
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
    }
    env.insert("TERM_PROGRAM".into(), "gaviero".into());
    env.insert("GAVIERO_TERMINAL".into(), "1".into());
    env
}

/// Version preflight for the pwsh hard requirement (Tier W1 / PR-3):
/// run once per process (cached), before the first PowerShell spawn.
/// Anything below 7.2 — or a missing binary — is a hard, actionable
/// error; there is no degraded 5.1/cmd mode.
#[cfg(windows)]
pub fn ensure_pwsh_version(shell_path: &std::path::Path) -> anyhow::Result<()> {
    use std::sync::OnceLock;
    static PREFLIGHT: OnceLock<Result<(), String>> = OnceLock::new();

    const INSTALL_HINT: &str =
        "Gaviero requires PowerShell 7.2+ on Windows — install with `winget install Microsoft.PowerShell`";

    PREFLIGHT
        .get_or_init(|| {
            let output = std::process::Command::new(shell_path)
                .args([
                    "-NoProfile",
                    "-Command",
                    "$PSVersionTable.PSVersion.ToString()",
                ])
                .output()
                .map_err(|e| format!("{INSTALL_HINT} (running {}: {e})", shell_path.display()))?;
            if !output.status.success() {
                return Err(format!(
                    "{INSTALL_HINT} ({} exited with {})",
                    shell_path.display(),
                    output.status
                ));
            }
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
            let major = parts.next().unwrap_or(0);
            let minor = parts.next().unwrap_or(0);
            if (major, minor) < (7, 2) {
                return Err(format!("{INSTALL_HINT} (found {version})"));
            }
            Ok(())
        })
        .clone()
        .map_err(|e| anyhow::anyhow!(e))
}

/// Global terminal configuration (from workspace settings).
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Override the default shell (None = use $SHELL / pwsh).
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
