//! Program resolution + spawn helpers (Tier W1 / PR-2).
//!
//! `Command::new("claude")` fails on Windows when the CLI is an
//! npm-installed `claude.cmd` shim: `CreateProcess` searches PATH but
//! does not append `PATHEXT` extensions, so the bare name is
//! NotFound even though `claude` works in every shell. The helpers
//! here do the PATHEXT-aware PATH walk in pure Rust — no `sh`, no
//! `where.exe` — and hand `Command` the resolved absolute path.
//! `std` itself safely wraps `.cmd`/`.bat` programs in `cmd.exe /C`
//! once it can see the extension — but its BatBadBut hardening
//! (CVE-2024-24576) *rejects* any argument containing `"`, `\r`, or
//! `\n` with "batch file arguments are invalid", and agent prompts are
//! multi-line by construction. A CLI that ships only a `.cmd` shim can
//! therefore never receive a prompt through it, so
//! [`resolve_agent_invocation`] hops over known batch launcher shims
//! (Cursor's `agent.cmd`) to the real executable they delegate to.
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

/// A resolved agent-CLI invocation: the program to execute plus any
/// arguments and environment its launcher shim would have injected.
pub struct AgentInvocation {
    pub program: PathBuf,
    /// Launcher arguments placed before caller-supplied args (e.g. the
    /// `index.js` entry point of a Node-based CLI).
    pub prepend_args: Vec<PathBuf>,
    /// Environment variables the launcher shim would have exported.
    pub envs: Vec<(&'static str, std::ffi::OsString)>,
}

/// Resolve `name` like [`resolve_program`], then — Windows only — hop
/// over known batch-file launcher shims to the executable they delegate
/// to.
///
/// `std` refuses to pass arguments containing `"`, `\r`, or `\n` to a
/// `.bat`/`.cmd` program (BatBadBut hardening — the error reads "batch
/// file arguments are invalid"), and prompts always contain newlines,
/// so spawning through such a shim can never work for prompt-on-argv
/// CLIs. Spawning the shim's own target is both safe (no `cmd.exe`
/// parsing anywhere in the chain) and faithful (it is exactly what the
/// launcher would have executed).
pub fn resolve_agent_invocation(name: &str) -> AgentInvocation {
    let program = resolve_program(name);
    #[cfg(windows)]
    if let Some(shim) = program.as_deref().filter(|p| is_batch_file(p))
        && let Some(bypass) = cursor_shim_bypass(shim)
    {
        return bypass;
    }
    AgentInvocation {
        program: program.unwrap_or_else(|| PathBuf::from(name)),
        prepend_args: Vec::new(),
        envs: Vec::new(),
    }
}

#[cfg(windows)]
fn is_batch_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// True when `e` is `std`'s batch-argument rejection: spawning a
/// `.bat`/`.cmd` whose argument list contains `"`, `\r`, or `\n`.
/// Spawn sites use it to replace the bare "batch file arguments are
/// invalid" with an actionable explanation.
pub fn is_batch_arg_error(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::InvalidInput
        && e.to_string().contains("batch file arguments")
}

/// Recognize the Cursor CLI's Windows install layout and return the
/// direct `node.exe index.js` invocation its launcher chain performs.
///
/// Layout (`%LOCALAPPDATA%\cursor-agent\`):
/// `agent.cmd` / `cursor-agent.cmd` → `powershell -File cursor-agent.ps1`
/// → `versions\<latest>\node.exe index.js`. The `.ps1` prefers a
/// `node.exe` + `index.js` pair next to itself, otherwise the newest
/// `versions\YYYY.MM.DD[-HH-MM-SS]-<hex>` directory. Mirrored here,
/// additionally requiring the pair to exist so a half-written update
/// falls back to the next-newest complete version. Unrecognized layouts
/// return `None` and keep the shim (a future `agent.exe` install never
/// reaches this — PATHEXT ranks `.exe` above `.cmd`).
#[cfg(windows)]
fn cursor_shim_bypass(shim: &Path) -> Option<AgentInvocation> {
    let dir = shim.parent()?;
    // The .ps1 is the layout discriminator: its presence identifies a
    // cursor-agent install dir regardless of which shim name resolved.
    if !dir.join("cursor-agent.ps1").is_file() {
        return None;
    }

    let node_pair = |d: &Path| {
        let node = d.join("node.exe");
        let index = d.join("index.js");
        (node.is_file() && index.is_file()).then_some((node, index))
    };

    let (node, index) = node_pair(dir).or_else(|| {
        let versions = dir.join("versions");
        let mut dirs: Vec<(u32, String, PathBuf)> = std::fs::read_dir(&versions)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let key = cursor_version_key(&name)?;
                Some((key, name, e.path()))
            })
            .collect();
        // Newest date first; the name tie-break only makes same-day
        // picks deterministic (the launcher can't order those either).
        dirs.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
        dirs.iter().find_map(|(_, _, p)| node_pair(p))
    })?;

    let mut envs: Vec<(&'static str, std::ffi::OsString)> = Vec::new();
    // agent.cmd exports its own file name before chaining to the .ps1;
    // the CLI reads it to know its invoked-as spelling.
    if let Some(invoked_as) = shim.file_name() {
        envs.push(("CURSOR_INVOKED_AS", invoked_as.to_os_string()));
    }
    // The .ps1 arms node's compile cache when the caller hasn't.
    if std::env::var_os("NODE_COMPILE_CACHE").is_none()
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        envs.push((
            "NODE_COMPILE_CACHE",
            Path::new(&local).join("cursor-compile-cache").into_os_string(),
        ));
    }

    Some(AgentInvocation {
        program: node,
        prepend_args: vec![index],
        envs,
    })
}

/// Sort key for a cursor-agent version directory: `YYYY.M.D-<hex>` or
/// `YYYY.M.D-HH-MM-SS-<hex>` → `YYYYMMDD`. Mirrors the validation
/// pattern in `cursor-agent.ps1` (lowercase hex only, exactly as the
/// launcher accepts).
#[cfg_attr(not(windows), allow(dead_code))]
fn cursor_version_key(name: &str) -> Option<u32> {
    fn num(s: &str) -> Option<u32> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }

    let segments: Vec<&str> = name.split('-').collect();
    let (date, hash) = match segments.as_slice() {
        [date, hash] => (*date, *hash),
        [date, h, m, s, hash]
            if [h, m, s]
                .iter()
                .all(|t| t.len() == 2 && t.bytes().all(|b| b.is_ascii_digit())) =>
        {
            (*date, *hash)
        }
        _ => return None,
    };
    if hash.is_empty() || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut parts = date.split('.');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || year.len() != 4
        || !matches!(month.len(), 1..=2)
        || !matches!(day.len(), 1..=2)
    {
        return None;
    }
    Some(num(year)? * 10_000 + num(month)? * 100 + num(day)?)
}

/// A `tokio::process::Command` for an agent CLI (`claude`, `codex`,
/// `agent`, …), resolved PATHEXT-aware with batch launcher shims
/// bypassed ([`resolve_agent_invocation`]). Unresolvable names pass
/// through verbatim so the eventual spawn error still names the
/// missing binary (the NotFound match arms in the backends stay
/// accurate). Detached from the host console on Windows
/// ([`isolate_console`]).
pub fn agent_command(name: &str) -> tokio::process::Command {
    let inv = resolve_agent_invocation(name);
    let mut cmd = tokio::process::Command::new(&inv.program);
    cmd.args(&inv.prepend_args);
    cmd.envs(inv.envs);
    isolate_console(&mut cmd);
    cmd
}

/// `std::process::Command` twin of [`agent_command`] for the blocking
/// call sites.
pub fn agent_command_std(name: &str) -> std::process::Command {
    let inv = resolve_agent_invocation(name);
    let mut cmd = std::process::Command::new(&inv.program);
    cmd.args(&inv.prepend_args);
    cmd.envs(inv.envs);
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

    #[test]
    fn cursor_version_key_parses_both_launcher_forms() {
        assert_eq!(cursor_version_key("2026.07.09-a3815c0"), Some(20_260_709));
        assert_eq!(cursor_version_key("2026.7.9-abc"), Some(20_260_709));
        assert_eq!(
            cursor_version_key("2026.07.16-10-30-00-899851b"),
            Some(20_260_716)
        );
    }

    #[test]
    fn cursor_version_key_rejects_non_version_names() {
        assert_eq!(cursor_version_key("latest"), None);
        assert_eq!(cursor_version_key("2026.07-a3815c0"), None); // missing day
        assert_eq!(cursor_version_key("2026.07.16-ABC"), None); // uppercase hex
        assert_eq!(cursor_version_key("2026.07.16"), None); // no hash
        assert_eq!(cursor_version_key("26.07.16-abc"), None); // 2-digit year
        assert_eq!(cursor_version_key("2026.07.16-1-2-3-abc"), None); // bad timestamp
    }

    /// Build a fake cursor-agent install: shim + .ps1 + the given
    /// version dirs (each optionally missing `index.js`).
    #[cfg(windows)]
    fn fake_cursor_install(root: &Path, versions: &[(&str, bool)]) {
        std::fs::write(root.join("agent.cmd"), "@echo off\r\n").unwrap();
        std::fs::write(root.join("cursor-agent.ps1"), "# launcher\r\n").unwrap();
        for (name, complete) in versions {
            let vdir = root.join("versions").join(name);
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join("node.exe"), "MZ").unwrap();
            if *complete {
                std::fs::write(vdir.join("index.js"), "// entry\n").unwrap();
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn cursor_shim_bypass_picks_newest_complete_version() {
        let dir = tempfile::tempdir().unwrap();
        fake_cursor_install(
            dir.path(),
            &[("2026.07.09-aaa", true), ("2026.07.16-bbb", true)],
        );
        let inv =
            resolve_agent_invocation(&dir.path().join("agent.cmd").to_string_lossy());
        let expected = dir.path().join("versions").join("2026.07.16-bbb");
        assert_eq!(inv.program, expected.join("node.exe"));
        assert_eq!(inv.prepend_args, vec![expected.join("index.js")]);
        assert!(
            inv.envs
                .iter()
                .any(|(k, v)| *k == "CURSOR_INVOKED_AS" && v == "agent.cmd")
        );
    }

    #[cfg(windows)]
    #[test]
    fn cursor_shim_bypass_skips_incomplete_newest_version() {
        let dir = tempfile::tempdir().unwrap();
        fake_cursor_install(
            dir.path(),
            &[("2026.07.09-aaa", true), ("2026.07.16-bbb", false)],
        );
        let inv =
            resolve_agent_invocation(&dir.path().join("agent.cmd").to_string_lossy());
        let expected = dir.path().join("versions").join("2026.07.09-aaa");
        assert_eq!(inv.program, expected.join("node.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn cursor_shim_bypass_prefers_side_by_side_node() {
        let dir = tempfile::tempdir().unwrap();
        fake_cursor_install(dir.path(), &[("2026.07.09-aaa", true)]);
        std::fs::write(dir.path().join("node.exe"), "MZ").unwrap();
        std::fs::write(dir.path().join("index.js"), "// entry\n").unwrap();
        let inv =
            resolve_agent_invocation(&dir.path().join("agent.cmd").to_string_lossy());
        assert_eq!(inv.program, dir.path().join("node.exe"));
        assert_eq!(inv.prepend_args, vec![dir.path().join("index.js")]);
    }

    #[cfg(windows)]
    #[test]
    fn foreign_cmd_shim_is_not_bypassed() {
        // No cursor-agent.ps1 → not the cursor layout → keep the shim
        // (npm shims etc. must not be second-guessed).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.cmd"), "@echo off\r\n").unwrap();
        let shim = dir.path().join("agent.cmd");
        let inv = resolve_agent_invocation(&shim.to_string_lossy());
        assert_eq!(inv.program, shim);
        assert!(inv.prepend_args.is_empty());
        assert!(inv.envs.is_empty());
    }
}
