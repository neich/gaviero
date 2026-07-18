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

/// `CREATE_NO_WINDOW` process-creation flag: the child gets its own
/// console (with no window) instead of attaching to gaviero's.
///
/// Everything spawned through this module is pure pipe-driven, but by
/// default `CreateProcess` attaches each child — and every descendant
/// (node, git, bash, cmd, rg, …) — to the parent's console. The console
/// *input mode* is a property of that shared console, not of a process:
/// one descendant flipping `ENABLE_PROCESSED_INPUT` back on (git
/// terminal prompts, MSYS init, any cooked `ReadConsole`) silently
/// reverts the TUI's crossterm raw mode, and the next Ctrl+C is then
/// broadcast as a `CTRL_C_EVENT` to every attached process — terminating
/// the handler-less TUI — instead of arriving as the key event the
/// keymap turns into cancel-stream (W1: "Ctrl+C killed gaviero
/// mid-prompt"). A private console removes both hazards: descendants
/// cannot stomp the host console's modes, and ctrl events raised inside
/// the agent tree cannot propagate out of it. `DETACHED_PROCESS` (no
/// console at all) is deliberately not used — MSYS/cmd descendants
/// expect console APIs to work.
///
/// Headless `gaviero-cli` trade-off: a terminal Ctrl+C now reaches only
/// the CLI itself; agent children exit on their broken stdio pipes
/// instead of the shared-console broadcast (all spawn sites already set
/// `kill_on_drop(true)` for the graceful paths).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply [`CREATE_NO_WINDOW`] to a child about to be spawned. No-op off
/// Windows. Public so spawn sites that build their own `Command` (the
/// tool-agent Bash tool) share the policy.
#[cfg(windows)]
pub fn isolate_console(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn isolate_console(_cmd: &mut tokio::process::Command) {}

/// `std::process::Command` twin of [`isolate_console`].
#[cfg(windows)]
pub fn isolate_console_std(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn isolate_console_std(_cmd: &mut std::process::Command) {}

/// Terminate every descendant process when this process exits — however
/// it exits (Windows).
///
/// Assigns the *current* process to a Job Object with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Job membership is inherited on
/// process creation, so every future descendant — including grandchildren
/// spawned by agent CLIs (node → bash → cargo) — lands in the job with no
/// per-spawn bookkeeping and no assign-after-spawn race. The job handle is
/// deliberately leaked: it must stay open for the process lifetime,
/// because the kernel terminates all members when the last handle closes.
/// That close happens on any death — clean exit, Ctrl+C's default
/// `ExitProcess`, panic, `std::process::exit`, or Task Manager — so this
/// is strictly stronger than a signal handler.
///
/// Exists for `gaviero-cli`: agent children run on private consoles
/// ([`isolate_console`]), so a terminal Ctrl+C no longer reaches them via
/// the shared-console broadcast; the job restores "the tree dies with the
/// CLI" as a kernel guarantee. The TUI deliberately does not call it —
/// its sessions die through cancel paths + `kill_on_drop`, and a job
/// would also hard-kill embedded terminal panes on exit.
///
/// No-op on Unix, where the terminal already delivers SIGINT to the whole
/// foreground process group (children inherit the pgid — nothing in this
/// module calls `setsid`/`process_group`), which is the platform's native
/// tree-teardown contract.
///
/// Idempotent. Nested jobs are fine on Windows 8+ — the process may
/// already sit in a terminal- or CI-managed job.
#[cfg(windows)]
pub fn kill_tree_on_exit() -> std::io::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // First caller arms; the rest return. A failed arm stays "armed" —
    // callers treat failure as log-and-degrade, not retry.
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            // Not yet a member — closing an unarmed, empty job is safe.
            let e = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(e);
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            let e = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(e);
        }
        // Member now — never CloseHandle from here on: closing IS the kill.
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn kill_tree_on_exit() -> std::io::Result<()> {
    Ok(())
}

/// A `tokio::process::Command` for an agent CLI (`claude`, `codex`,
/// `agent`, …), resolved PATHEXT-aware. Unresolvable names pass
/// through verbatim so the eventual spawn error still names the
/// missing binary (the NotFound match arms in the backends stay
/// accurate). Detached from the host console on Windows
/// ([`isolate_console`]).
pub fn agent_command(name: &str) -> tokio::process::Command {
    let mut cmd = match resolve_program(name) {
        Some(path) => tokio::process::Command::new(path),
        None => tokio::process::Command::new(name),
    };
    isolate_console(&mut cmd);
    cmd
}

/// `std::process::Command` twin of [`agent_command`] for the blocking
/// call sites.
pub fn agent_command_std(name: &str) -> std::process::Command {
    let mut cmd = match resolve_program(name) {
        Some(path) => std::process::Command::new(path),
        None => std::process::Command::new(name),
    };
    isolate_console_std(&mut cmd);
    cmd
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
        isolate_console(&mut cmd);
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
            isolate_console(&mut cmd);
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

    /// Arming must succeed and be idempotent. On Windows the first call
    /// really adopts the test process into a kill-on-close job — safe:
    /// membership restricts nothing while running, and at exit the job
    /// only reaps members that are already dying with the harness.
    #[test]
    fn kill_tree_on_exit_arms_and_is_idempotent() {
        kill_tree_on_exit().expect("first arm");
        kill_tree_on_exit().expect("second arm (no-op)");
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
