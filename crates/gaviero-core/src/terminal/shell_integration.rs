//! Shell init script generation for OSC 133 / OSC 7 integration.
//!
//! Each script is injected at shell spawn time via shell-specific mechanisms:
//! - Bash: `--init-file`
//! - Zsh: `ZDOTDIR` wrapping
//! - Fish: `--init-command`
//! - PowerShell (pwsh 7.2+): `-NoExit -Command ". '<init.ps1>'"` (dot-source)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::config::{ShellConfig, ShellType};
use super::types::TerminalId;

/// Directory where init scripts are stored.
fn shell_integration_dir() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("gaviero/shell");
    std::fs::create_dir_all(&dir).context("creating shell integration directory")?;
    Ok(dir)
}

/// Create an init script for the given shell type, write it to disk,
/// and return the path.
pub fn create_init_file(
    shell_type: &ShellType,
    tab_id: &TerminalId,
    histfile: &Path,
) -> Result<PathBuf> {
    let dir = shell_integration_dir()?;
    let content = match shell_type {
        ShellType::Bash => generate_bash_init(histfile),
        ShellType::Zsh => generate_zsh_init(histfile),
        ShellType::Fish => generate_fish_init(histfile),
        ShellType::PowerShell => generate_pwsh_init(histfile),
        ShellType::Unknown(_) => return Err(anyhow::anyhow!("unsupported shell for integration")),
    };

    // pwsh only dot-sources scripts with a `.ps1` extension.
    let extension = match shell_type {
        ShellType::PowerShell => "ps1",
        _ => "sh",
    };
    let filename = format!("gaviero-init-{}.{}", tab_id, extension);
    let path = dir.join(filename);
    std::fs::write(&path, content).context("writing shell init script")?;
    Ok(path)
}

/// Modify the shell config to inject the init file at spawn time.
pub fn build_shell_args(config: &mut ShellConfig, init_path: &Path) {
    match &config.shell_type {
        ShellType::Bash => {
            config.shell_args = vec![
                "--init-file".into(),
                init_path.to_string_lossy().into_owned(),
            ];
        }
        ShellType::Zsh => {
            // For zsh, we create a wrapper .zshrc in a temp ZDOTDIR
            // that sources the init, then sources the real .zshrc
            if let Ok(wrapper_dir) = create_zsh_zdotdir(init_path) {
                config
                    .env_overrides
                    .insert("ZDOTDIR".into(), wrapper_dir.to_string_lossy().into_owned());
            }
        }
        ShellType::Fish => {
            config.shell_args = vec![
                "--init-command".into(),
                format!("source {}", init_path.to_string_lossy()),
            ];
        }
        ShellType::PowerShell => {
            // Dot-source so the init runs in the interactive session's
            // scope; `-File` + `-NoExit` doesn't leave an interactive
            // prompt. `-ExecutionPolicy Bypass` is process-scoped.
            config.shell_args = vec![
                "-NoExit".into(),
                "-NoLogo".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                // Single-quoted PowerShell string: `'` escapes by doubling.
                format!(". '{}'", init_path.to_string_lossy().replace('\'', "''")),
            ];
        }
        ShellType::Unknown(_) => {}
    }
}

/// Create a ZDOTDIR wrapper for zsh that sources gaviero init then user's .zshrc.
fn create_zsh_zdotdir(init_path: &Path) -> Result<PathBuf> {
    let dir = shell_integration_dir()?.join("zsh-zdotdir");
    std::fs::create_dir_all(&dir)?;

    let zshrc_content = format!(
        r#"# Gaviero zsh integration wrapper
# Source gaviero shell integration
source "{init_path}"

# Restore original ZDOTDIR and source user's .zshrc
if [ -n "${{USER_ZDOTDIR}}" ]; then
    ZDOTDIR="${{USER_ZDOTDIR}}"
else
    ZDOTDIR="${{HOME}}"
fi

if [ -f "${{ZDOTDIR}}/.zshrc" ]; then
    source "${{ZDOTDIR}}/.zshrc"
fi
"#,
        init_path = init_path.to_string_lossy()
    );

    let zshrc_path = dir.join(".zshrc");
    std::fs::write(&zshrc_path, zshrc_content)?;
    Ok(dir)
}

/// Generate bash init script content.
fn generate_bash_init(histfile: &Path) -> String {
    format!(
        r#"#!/bin/bash
# Gaviero shell integration for bash
# This script is sourced via --init-file
#
# --init-file makes bash skip /etc/profile and ~/.bash_profile, so we
# replicate the login-shell startup chain here to ensure PATH and other
# environment setup from those files is preserved.

# Save original HISTFILE, set gaviero per-tab history
export HISTFILE="{histfile}"

# Shadow tmux/screen during RC sourcing to prevent auto-launch hijack
__gaviero_real_tmux="$(command -v tmux 2>/dev/null)"
__gaviero_real_screen="$(command -v screen 2>/dev/null)"
tmux() {{ :; }}
screen() {{ :; }}
export -f tmux screen 2>/dev/null

# Replicate login-shell startup: source /etc/profile, then the user's
# profile (which typically sources .bashrc itself).
if [ -f /etc/profile ]; then
    source /etc/profile
fi

if [ -f "$HOME/.bash_profile" ]; then
    source "$HOME/.bash_profile"
elif [ -f "$HOME/.bash_login" ]; then
    source "$HOME/.bash_login"
elif [ -f "$HOME/.profile" ]; then
    source "$HOME/.profile"
else
    # No profile found — source .bashrc directly as fallback
    if [ -f "$HOME/.bashrc" ]; then
        source "$HOME/.bashrc"
    fi
fi

# Restore tmux/screen
unset -f tmux screen 2>/dev/null

# Flush history after every command
PROMPT_COMMAND="history -a;${{PROMPT_COMMAND}}"

# --- OSC 133 shell integration ---
__gaviero_osc133_prompt_command() {{
    local exit_code=$?
    # OSC 133;D — command finished (with exit code)
    printf '\e]133;D;%s\a' "$exit_code"
    # OSC 133;A — prompt start
    printf '\e]133;A\a'
    # OSC 7 — report CWD
    printf '\e]7;file://%s%s\a' "$(hostname)" "$(pwd)"
}}

# Prepend our handler to PROMPT_COMMAND
PROMPT_COMMAND="__gaviero_osc133_prompt_command;${{PROMPT_COMMAND}}"

# PS0 — emitted after command is entered, before execution
PS0='\e]133;C\a'

# Wrap PS1 to emit OSC 133;B after the prompt
__gaviero_original_ps1="${{PS1}}"
PS1="${{PS1}}\[\e]133;B\a\]"

# Emit initial prompt markers
printf '\e]133;A\a'
"#,
        histfile = histfile.to_string_lossy()
    )
}

/// Generate zsh init script content.
fn generate_zsh_init(histfile: &Path) -> String {
    format!(
        r#"#!/bin/zsh
# Gaviero shell integration for zsh

# Per-tab history
export HISTFILE="{histfile}"
setopt INC_APPEND_HISTORY

# --- OSC 133 shell integration ---
autoload -Uz add-zsh-hook

__gaviero_precmd() {{
    local exit_code=$?
    # OSC 133;D — command finished
    printf '\e]133;D;%s\a' "$exit_code"
    # OSC 133;A — prompt start
    printf '\e]133;A\a'
    # OSC 7 — report CWD
    printf '\e]7;file://%s%s\a' "$(hostname)" "$(pwd)"
}}

__gaviero_preexec() {{
    # OSC 133;C — command output start
    printf '\e]133;C\a'
}}

add-zsh-hook precmd __gaviero_precmd
add-zsh-hook preexec __gaviero_preexec

# Append OSC 133;B to prompt (command input start)
setopt PROMPT_SUBST
precmd_functions+=(__gaviero_precmd)

# Add OSC 133;B marker after prompt
PS1="${{PS1}}%{{$(printf '\e]133;B\a')%}}"

# Emit initial prompt markers
printf '\e]133;A\a'
"#,
        histfile = histfile.to_string_lossy()
    )
}

/// Generate fish init script content.
fn generate_fish_init(histfile: &Path) -> String {
    format!(
        r#"# Gaviero shell integration for fish

# Per-tab history
set -gx HISTFILE "{histfile}"

# --- OSC 133 shell integration ---

function __gaviero_fish_prompt --on-event fish_prompt
    # OSC 133;A — prompt start
    printf '\e]133;A\a'
    # OSC 7 — report CWD
    printf '\e]7;file://%s%s\a' (hostname) (pwd)
end

function __gaviero_fish_preexec --on-event fish_preexec
    # OSC 133;C — command output start
    printf '\e]133;C\a'
end

function __gaviero_fish_postexec --on-event fish_postexec
    # OSC 133;D — command finished
    printf '\e]133;D;%s\a' $status
end

# Emit initial prompt markers
printf '\e]133;A\a'
"#,
        histfile = histfile.to_string_lossy()
    )
}

/// Generate PowerShell (pwsh 7.2+) init script content.
///
/// One dialect only (Tier W1 / PR-3): `` `e `` escapes, PSReadLine
/// assumed present, no 5.1 compatibility. The user's `prompt` function
/// is wrapped, not replaced; OSC 133;C (command execution start) comes
/// from wrapping `PSConsoleHostReadLine`, which PSReadLine invokes
/// after the user submits a line.
fn generate_pwsh_init(histfile: &Path) -> String {
    // Single-quoted PowerShell strings escape `'` by doubling it.
    let histfile_ps = histfile.to_string_lossy().replace('\'', "''");
    format!(
        r#"# Gaviero shell integration for PowerShell 7.2+
# Dot-sourced at spawn via: pwsh -NoExit -NoLogo -ExecutionPolicy Bypass -Command ". '<this file>'"

# Per-tab command history
Set-PSReadLineOption -HistorySavePath '{histfile_ps}' -ErrorAction SilentlyContinue

# --- OSC 133 / OSC 7 shell integration ---

# Wrap (not replace) the user's prompt function.
$global:__gavieroOriginalPrompt = $function:prompt

function global:prompt {{
    # Capture command outcome before anything here can clobber it.
    $gavieroExit = if ($null -ne $global:LASTEXITCODE) {{ $global:LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }}
    # OSC 133;D — command finished (with exit code)
    [Console]::Write("`e]133;D;$gavieroExit`a")
    # OSC 133;A — prompt start
    [Console]::Write("`e]133;A`a")
    # OSC 7 — report CWD (forward slashes, file://<host>/<path>)
    $gavieroCwd = $PWD.ProviderPath -replace '\\', '/'
    [Console]::Write("`e]7;file://$([System.Environment]::MachineName)/$gavieroCwd`a")
    # Original prompt text, then OSC 133;B — prompt end / input start
    $gavieroPrompt = & $global:__gavieroOriginalPrompt
    "$gavieroPrompt`e]133;B`a"
}}

# OSC 133;C — command execution start. PSReadLine calls
# PSConsoleHostReadLine to obtain the submitted line; emit C after it
# returns, immediately before the host executes the command.
$global:__gavieroOriginalReadLine = $function:PSConsoleHostReadLine
function global:PSConsoleHostReadLine {{
    $gavieroLine = & $global:__gavieroOriginalReadLine
    [Console]::Write("`e]133;C`a")
    $gavieroLine
}}

# Emit initial prompt marker
[Console]::Write("`e]133;A`a")
"#
    )
}
