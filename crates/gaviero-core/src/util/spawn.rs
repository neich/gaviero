//! Program resolution + spawn helpers (Tier W1 / PR-2).
//!
//! `Command::new("claude")` fails on Windows when the CLI is an
//! npm-installed `claude.cmd` shim: `CreateProcess` searches PATH but
//! does not append `PATHEXT` extensions, so the bare name is
//! NotFound even though `claude` works in every shell. The helpers
//! here do the PATHEXT-aware PATH walk in pure Rust — no `sh`, no
//! `where.exe` — and hand `Command` the resolved absolute path.
//! `std` itself safely wraps `.cmd`/`.bat` programs in `cmd.exe /C`
//! (with BatBadBut-hardened quoting) once it can see the extension.
//!
//! On Unix the walk checks the executable bit, which also lets MCP
//! preflight drop its `sh -c "command -v …"` dependency.

use std::path::{Path, PathBuf};

/// Resolve `name` to an executable path.
///
/// * Names containing a path separator are checked as-is (no PATH walk).
/// * Bare names walk `PATH`. On Windows each candidate is tried with
///   every `PATHEXT` extension (default `.COM;.EXE;.BAT;.CMD` when
///   unset — W-D3: earlier extensions win, so `.exe` beats `.cmd`),
///   plus verbatim when the name already carries an extension. On
///   Unix a candidate must be a file with an executable bit.
pub fn resolve_program(name: &str) -> Option<PathBuf> {
    let as_path = Path::new(name);
    if name.contains('/') || name.contains('\\') {
        if cfg!(windows) && as_path.extension().is_none() {
            // Explicit path without extension: still try PATHEXT.
            return windows_candidates(as_path).find(|c| c.is_file());
        }
        return is_executable(as_path).then(|| as_path.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if cfg!(windows) {
            if let Some(hit) = windows_candidates(&candidate).find(|c| c.is_file()) {
                return Some(hit);
            }
        } else if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// The candidate spellings for one Windows path: verbatim when it
/// already has an extension, then `candidate + ext` in PATHEXT order.
fn windows_candidates(candidate: &Path) -> impl Iterator<Item = PathBuf> {
    let verbatim = candidate
        .extension()
        .is_some()
        .then(|| candidate.to_path_buf());
    let base = candidate.as_os_str().to_os_string();
    let exts: Vec<String> = std::env::var("PATHEXT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|e| e.starts_with('.'))
        .map(|e| e.to_string())
        .collect();
    verbatim.into_iter().chain(exts.into_iter().map(move |ext| {
        let mut with_ext = base.clone();
        with_ext.push(ext);
        PathBuf::from(with_ext)
    }))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// A `tokio::process::Command` for an agent CLI (`claude`, `codex`,
/// `agent`, …), resolved PATHEXT-aware. Unresolvable names pass
/// through verbatim so the eventual spawn error still names the
/// missing binary (the NotFound match arms in the backends stay
/// accurate).
pub fn agent_command(name: &str) -> tokio::process::Command {
    match resolve_program(name) {
        Some(path) => tokio::process::Command::new(path),
        None => tokio::process::Command::new(name),
    }
}

/// `std::process::Command` twin of [`agent_command`] for the blocking
/// call sites.
pub fn agent_command_std(name: &str) -> std::process::Command {
    match resolve_program(name) {
        Some(path) => std::process::Command::new(path),
        None => std::process::Command::new(name),
    }
}

/// Bytes of prompt allowed on argv before spilling to a tempfile /
/// stdin. Windows caps the *whole* command line (program path + all
/// args, UTF-16) at ~32,767 chars, so the per-prompt allowance must
/// leave generous headroom (W-I12); Unix argv limits are per-arg and
/// far higher.
pub fn argv_threshold() -> usize {
    if cfg!(windows) { 8_192 } else { 32_768 }
}

/// Locate a POSIX `bash` on Windows — Git Bash (Tier W1 / PR-4, W-D5).
/// Order: `GAVIERO_GIT_BASH_PATH` env override → `bash` on PATH
/// (ignoring `System32\bash.exe`, the WSL launcher — POSIX scripts
/// must not silently run inside WSL) → `%ProgramFiles%\Git\bin\bash.exe`.
#[cfg(windows)]
pub fn find_git_bash() -> Option<PathBuf> {
    if let Some(overridden) = std::env::var_os("GAVIERO_GIT_BASH_PATH") {
        let path = PathBuf::from(overridden);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(hit) = resolve_program("bash") {
        let is_wsl_launcher = hit
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n.eq_ignore_ascii_case("system32"));
        if !is_wsl_launcher {
            return Some(hit);
        }
    }
    let program_files = std::env::var("ProgramFiles").ok()?;
    let candidate = PathBuf::from(program_files).join(r"Git\bin\bash.exe");
    candidate.is_file().then_some(candidate)
}

/// A command that runs `script` under a POSIX shell: `sh -c` on Unix,
/// Git Bash `bash -c` on Windows. Errors (actionably) when Windows has
/// no Git Bash — callers own the user-facing framing.
pub fn posix_shell_command(script: &str) -> anyhow::Result<tokio::process::Command> {
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(script);
        Ok(cmd)
    }
    #[cfg(windows)]
    {
        let bash = find_git_bash().ok_or_else(|| {
            anyhow::anyhow!(
                "a POSIX shell is required but Git Bash was not found — \
                 install Git for Windows or set GAVIERO_GIT_BASH_PATH"
            )
        })?;
        let mut cmd = tokio::process::Command::new(bash);
        cmd.arg("-c").arg(script);
        Ok(cmd)
    }
}

/// Like [`posix_shell_command`], but on a Git-Bash-less Windows falls
/// back to `pwsh -NoProfile -Command` (a guaranteed runtime dep,
/// Tier W1 / PR-3) instead of erroring.
///
/// ONLY for user-authored commands — workflow loop-until probes and
/// verify test commands — where the user owns the syntax. The agent
/// Bash tool must NOT use this: its LLM-facing contract is POSIX
/// semantics, and silently running its scripts under another shell
/// would corrupt agent behavior (W-D5).
pub fn shell_command_lenient(script: &str) -> tokio::process::Command {
    match posix_shell_command(script) {
        Ok(cmd) => cmd,
        Err(_) => {
            // Windows-only branch: posix_shell_command is infallible on Unix.
            let pwsh = resolve_program("pwsh").unwrap_or_else(|| PathBuf::from("pwsh"));
            let mut cmd = tokio::process::Command::new(pwsh);
            cmd.arg("-NoProfile").arg("-Command").arg(script);
            cmd
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn resolves_cmd_shim_via_pathext() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fake-agent.cmd"), "@echo off\r\n").unwrap();
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(dir.path().to_path_buf())
                .chain(std::env::split_paths(&old_path)),
        )
        .unwrap();
        // Serialize PATH mutation against other tests in this binary.
        unsafe { std::env::set_var("PATH", &joined) };
        let hit = resolve_program("fake-agent");
        unsafe { std::env::set_var("PATH", &old_path) };
        // PATHEXT extensions are conventionally uppercase; the hit's
        // extension case follows PATHEXT, not the on-disk name.
        assert!(
            hit.is_some_and(|h| h
                .to_string_lossy()
                .eq_ignore_ascii_case(&dir.path().join("fake-agent.cmd").to_string_lossy())),
        );
    }

    #[cfg(windows)]
    #[test]
    fn exe_wins_over_cmd_in_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dual.cmd"), "@echo off\r\n").unwrap();
        std::fs::write(dir.path().join("dual.exe"), "MZ").unwrap();
        // Path-qualified lookup avoids touching PATH.
        let hit = resolve_program(&dir.path().join("dual").to_string_lossy());
        assert!(
            hit.is_some_and(|h| h
                .to_string_lossy()
                .eq_ignore_ascii_case(&dir.path().join("dual.exe").to_string_lossy())),
        );
    }

    #[cfg(unix)]
    #[test]
    fn requires_executable_bit_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("not-exec");
        std::fs::write(&plain, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_program(&plain.to_string_lossy()), None);
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            resolve_program(&plain.to_string_lossy()),
            Some(plain.clone())
        );
    }

    #[test]
    fn unresolvable_name_is_none() {
        assert_eq!(resolve_program("definitely-not-a-real-binary-xyz"), None);
    }

    #[test]
    fn windows_threshold_is_lower() {
        if cfg!(windows) {
            assert_eq!(argv_threshold(), 8_192);
        } else {
            assert_eq!(argv_threshold(), 32_768);
        }
    }
}
