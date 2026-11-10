//! Restart-on-new-build: a running `gaviero` moves onto a freshly installed
//! build by itself.
//!
//! Two halves:
//!
//! - **Supervisor** ([`supervise`]). The `gaviero` a user starts becomes a
//!   thin parent that runs the real TUI as a child — same arguments, working
//!   directory and console — and starts it again whenever the child exits
//!   with [`RESTART_EXIT_CODE`]. The pane (psmux, tmux, a terminal tab) keeps
//!   its process, so a restart lands in place and the session restore brings
//!   back tabs, layout and conversations. Any other exit code ends the
//!   supervisor with that code. `GAVIERO_NO_SUPERVISOR=1` opts out; without
//!   a terminal on stdin/stdout the TUI runs directly.
//! - **Build watch** ([`BuildWatch`]). The TUI stats its own executable every
//!   few seconds. A size or mtime that differs from the startup snapshot and
//!   has stopped changing means a new build was installed. The app side
//!   (`app/restart.rs`) waits until nothing is running and then quits through
//!   the normal save path, exiting with the restart code.
//!
//! The session host (`gaviero --sessions`) gets the same treatment from
//! [`supervise_host`]: it watches its executable too, and once every tab is
//! idle it records its tabs, ends them and exits with the restart code; the
//! new host reopens the same tabs on the new build (`session_host/restart.rs`).
//! Its tabs then leave restarting to the host.
//!
//! Windows refuses to overwrite a running `.exe` but allows renaming it, so
//! an installer has to move the old file aside first — `scripts/deploy.ps1`
//! does that. The supervisor keeps running its old image the whole time; it
//! only ever spawns the path it was started from.

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant, SystemTime};

/// Exit code the TUI uses to ask its supervisor for a restart
/// (`EX_TEMPFAIL`: "try again").
pub const RESTART_EXIT_CODE: i32 = 75;

/// Set on the TUI child so it knows a supervisor will relaunch it (and so it
/// never supervises itself).
const CHILD_ENV: &str = "GAVIERO_SUPERVISED";

/// Run the TUI directly, without a supervisor parent.
const OPT_OUT_ENV: &str = "GAVIERO_NO_SUPERVISOR";

/// Set on the TUI child: a file, private to this supervisor, where the child
/// records answers to launch prompts so a restart reuses them instead of
/// stopping at the prompt again ([`recall_launch_choice`]).
const LAUNCH_STATE_ENV: &str = "GAVIERO_LAUNCH_STATE";

/// How long to wait for the executable to reappear: an installer moves the
/// old file aside before the new one is in place.
const EXE_WAIT: Duration = Duration::from_secs(30);

/// A child that asks for a restart sooner than this after starting is backed
/// off by [`RESTART_BACKOFF`], so a misbehaving build cannot spin the pane.
/// The session host (`session_host`) applies the same limits to its tabs.
pub(crate) const MIN_RUN: Duration = Duration::from_secs(10);
pub(crate) const RESTART_BACKOFF: Duration = Duration::from_secs(5);

/// The environment a supervised TUI child gets: marked as supervised, with
/// `launch_state` as its private launch-state file. Used by the session host,
/// which supervises each of its tabs the way [`supervise`] does one pane.
pub(crate) fn child_env(launch_state: &Path) -> [(&'static str, OsString); 2] {
    [
        (CHILD_ENV, OsString::from("1")),
        (LAUNCH_STATE_ENV, launch_state.as_os_str().to_owned()),
    ]
}

/// Set on a supervised session host (`gaviero --sessions`): a file, private
/// to its supervisor, where the host records its tabs before it exits to
/// restart, so the new build reopens exactly those tabs
/// ([`host_restart_record`]).
pub(crate) const HOST_RESTART_ENV: &str = "GAVIERO_SESSION_HOST_RESTART";

/// True in the TUI child of a supervisor — only then may the TUI exit with
/// [`RESTART_EXIT_CODE`] and expect to come back.
pub fn is_supervised() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Whether a supervisor may start here: not opted out, on a terminal.
fn may_supervise() -> bool {
    std::env::var_os(OPT_OUT_ENV).is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

/// Act as the supervisor when this process should be one.
///
/// Returns `Some(exit code)` after supervising (the caller exits with it), or
/// `None` when the caller should run the TUI itself: already a child, opted
/// out, no terminal, or the executable path is unknown.
pub fn supervise() -> Option<i32> {
    if is_supervised() || !may_supervise() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let launch_state = launch_state_path("launch", "state");
    let mut env = vec![(CHILD_ENV, OsString::from("1"))];
    if let Some(path) = &launch_state {
        env.push((LAUNCH_STATE_ENV, path.as_os_str().to_owned()));
    }
    let code = run_supervised(&exe, &args, &env);
    if let Some(path) = &launch_state {
        let _ = std::fs::remove_file(path);
    }
    Some(code)
}

/// [`supervise`] for the session host: the `gaviero --sessions` a user
/// starts runs the real host as a child (same arguments) and starts it again
/// when it exits with [`RESTART_EXIT_CODE`] after a new build is installed.
/// The child records its tabs in the [`host_restart_record`] file first and
/// the new host reopens them. `None` when the caller should run the host
/// itself: already the child, opted out, or no terminal.
pub fn supervise_host() -> Option<i32> {
    if host_restart_record().is_some() || !may_supervise() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let record = launch_state_path("session-host-restart", "json")?;
    // Ours by pid: anything already there was left by a dead supervisor.
    let _ = std::fs::remove_file(&record);
    let env = [(HOST_RESTART_ENV, record.as_os_str().to_owned())];
    let code = run_supervised(&exe, &args, &env);
    let _ = std::fs::remove_file(&record);
    Some(code)
}

/// In a supervised session host: the file to record its tabs in before it
/// exits to restart, and to reopen them from at startup. `None` when the
/// host is not supervised — then nothing would start it again.
pub(crate) fn host_restart_record() -> Option<PathBuf> {
    std::env::var_os(HOST_RESTART_ENV).map(PathBuf::from)
}

/// Run `exe` with `args` and `env` until it exits with something other
/// than [`RESTART_EXIT_CODE`]; that exit code.
fn run_supervised(exe: &Path, args: &[OsString], env: &[(&str, OsString)]) -> i32 {
    // The child owns the keyboard. A Ctrl+C / Ctrl+Break console event must
    // not kill this parent out from under it (closing the window still does).
    crate::platform::ignore_console_interrupts();

    loop {
        let started = Instant::now();
        let status = match run_child(exe, args, env) {
            Ok(status) => status,
            Err(e) => {
                eprintln!("gaviero: could not start {}: {e}", exe.display());
                return 1;
            }
        };
        match status.code() {
            Some(RESTART_EXIT_CODE) => {
                eprintln!("gaviero: new build installed, restarting");
                if started.elapsed() < MIN_RUN {
                    std::thread::sleep(RESTART_BACKOFF);
                }
            }
            Some(code) => return code,
            // Killed by a signal (Unix): report failure like a shell would.
            None => return 1,
        }
    }
}

/// A file private to this supervisor: under the cache dir, keyed by pid.
/// A stale launch-state file left by a killed supervisor whose pid is later
/// reused is harmless — every record names the launch path it answers for.
fn launch_state_path(stem: &str, ext: &str) -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("gaviero");
    Some(dir.join(format!("{stem}-{}.{ext}", std::process::id())))
}

fn run_child(exe: &Path, args: &[OsString], env: &[(&str, OsString)]) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + EXE_WAIT;
    loop {
        let mut cmd = Command::new(exe);
        cmd.args(args);
        for (key, value) in env {
            cmd.env(key, value);
        }
        match cmd.status() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(500));
            }
            other => return other,
        }
    }
}

/// The answer this supervisor's earlier child gave for `launch` (the
/// resolved launch path), if any. `None` outside a supervisor.
pub fn recall_launch_choice(launch: &Path) -> Option<crate::setup::FolderLaunch> {
    let state = std::env::var_os(LAUNCH_STATE_ENV)?;
    read_launch_choice(Path::new(&state), launch)
}

/// Record the answer for `launch` so a restart under this supervisor reuses
/// it. A no-op outside a supervisor; failures only cost a repeated prompt.
pub fn remember_launch_choice(launch: &Path, choice: &crate::setup::FolderLaunch) {
    if let Some(state) = std::env::var_os(LAUNCH_STATE_ENV) {
        write_launch_choice(Path::new(&state), launch, choice);
    }
}

fn read_launch_choice(state: &Path, launch: &Path) -> Option<crate::setup::FolderLaunch> {
    use crate::setup::FolderLaunch;
    let text = std::fs::read_to_string(state).ok()?;
    let mut lines = text.lines();
    if Path::new(lines.next()?) != launch {
        return None;
    }
    match lines.next()? {
        "folder" => Some(FolderLaunch::Folder),
        "workspace" => {
            let file = PathBuf::from(lines.next()?);
            file.is_file().then_some(FolderLaunch::Workspace(file))
        }
        _ => None,
    }
}

fn write_launch_choice(state: &Path, launch: &Path, choice: &crate::setup::FolderLaunch) {
    use crate::setup::FolderLaunch;
    let body = match choice {
        FolderLaunch::Folder => format!("{}\nfolder\n", launch.display()),
        FolderLaunch::Workspace(file) => {
            format!("{}\nworkspace\n{}\n", launch.display(), file.display())
        }
    };
    if let Some(dir) = state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(state, body) {
        tracing::warn!("could not record launch choice in {}: {e}", state.display());
    }
}

/// What identifies one build of the executable on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// How often the executable is stat'ed.
const POLL_EVERY: Duration = Duration::from_secs(2);

/// A changed executable must stay unchanged this long before it counts as
/// installed — the copy may still be in progress.
const SETTLE: Duration = Duration::from_secs(3);

/// Notices when the executable this process was started from is replaced.
pub struct BuildWatch {
    exe: Option<PathBuf>,
    baseline: Option<Stamp>,
    /// A differing stamp and when it was first seen.
    pending: Option<(Stamp, Instant)>,
    last_poll: Option<Instant>,
    ready: bool,
}

impl BuildWatch {
    /// Snapshot the running executable. Without a readable path the watch
    /// stays inert.
    pub fn for_current_exe() -> Self {
        let exe = std::env::current_exe().ok();
        let baseline = exe.as_deref().and_then(stamp_of);
        Self {
            exe,
            baseline,
            pending: None,
            last_poll: None,
            ready: false,
        }
    }

    /// A settled new build has been seen. Stays true once set.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Stat the executable (at most every [`POLL_EVERY`]). Returns true on
    /// the poll that first finds a settled new build.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.ready || self.baseline.is_none() {
            return false;
        }
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < POLL_EVERY)
        {
            return false;
        }
        self.last_poll = Some(now);
        let current = self.exe.as_deref().and_then(stamp_of);
        self.observe(current, now)
    }

    fn observe(&mut self, current: Option<Stamp>, now: Instant) -> bool {
        // Missing: the installer is between moving the old file aside and
        // putting the new one in place.
        let Some(current) = current else {
            self.pending = None;
            return false;
        };
        if Some(current) == self.baseline {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((seen, since)) if seen == current => {
                if now.duration_since(since) >= SETTLE {
                    self.ready = true;
                    return true;
                }
            }
            _ => self.pending = Some((current, now)),
        }
        false
    }

    #[cfg(test)]
    fn with_baseline(baseline: Stamp) -> Self {
        Self {
            exe: None,
            baseline: Some(baseline),
            pending: None,
            last_poll: None,
            ready: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(len: u64, secs: u64) -> Stamp {
        Stamp {
            len,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        }
    }

    #[test]
    fn an_unchanged_executable_is_never_ready() {
        let base = stamp(100, 1);
        let mut watch = BuildWatch::with_baseline(base);
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(!watch.observe(Some(base), t0 + Duration::from_secs(i * 2)));
        }
        assert!(!watch.ready());
    }

    #[test]
    fn a_new_build_is_ready_once_it_has_settled() {
        let mut watch = BuildWatch::with_baseline(stamp(100, 1));
        let t0 = Instant::now();
        let new = stamp(120, 2);
        assert!(!watch.observe(Some(new), t0));
        assert!(!watch.observe(Some(new), t0 + Duration::from_secs(2)));
        assert!(watch.observe(Some(new), t0 + SETTLE));
        assert!(watch.ready());
    }

    #[test]
    fn a_file_still_being_written_restarts_the_settle_clock() {
        let mut watch = BuildWatch::with_baseline(stamp(100, 1));
        let t0 = Instant::now();
        assert!(!watch.observe(Some(stamp(10, 2)), t0));
        // Still growing: the settle window starts over from this sighting.
        assert!(!watch.observe(Some(stamp(60, 3)), t0 + Duration::from_secs(4)));
        assert!(!watch.observe(Some(stamp(60, 3)), t0 + Duration::from_secs(5)));
        assert!(watch.observe(Some(stamp(60, 3)), t0 + Duration::from_secs(4) + SETTLE));
    }

    #[test]
    fn a_missing_executable_mid_swap_is_not_a_new_build() {
        let mut watch = BuildWatch::with_baseline(stamp(100, 1));
        let t0 = Instant::now();
        assert!(!watch.observe(Some(stamp(120, 2)), t0));
        assert!(!watch.observe(None, t0 + Duration::from_secs(2)));
        // The gap resets the clock.
        assert!(!watch.observe(Some(stamp(120, 2)), t0 + Duration::from_secs(4)));
        assert!(watch.observe(Some(stamp(120, 2)), t0 + Duration::from_secs(4) + SETTLE));
    }

    #[test]
    fn without_a_baseline_the_watch_is_inert() {
        let mut watch = BuildWatch {
            exe: None,
            baseline: None,
            pending: None,
            last_poll: None,
            ready: false,
        };
        assert!(!watch.poll(Instant::now()));
        assert!(!watch.ready());
    }

    #[test]
    fn a_launch_choice_round_trips_only_for_its_own_launch_path() {
        use crate::setup::FolderLaunch;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("cache").join("launch-1.state");
        let launch = dir.path().join("parent");
        let ws_file = dir.path().join("parent.gaviero-workspace");
        std::fs::write(&ws_file, "{}").unwrap();

        assert_eq!(read_launch_choice(&state, &launch), None, "nothing recorded");

        write_launch_choice(&state, &launch, &FolderLaunch::Workspace(ws_file.clone()));
        assert_eq!(
            read_launch_choice(&state, &launch),
            Some(FolderLaunch::Workspace(ws_file.clone()))
        );
        assert_eq!(read_launch_choice(&state, &dir.path().join("other")), None);

        write_launch_choice(&state, &launch, &FolderLaunch::Folder);
        assert_eq!(read_launch_choice(&state, &launch), Some(FolderLaunch::Folder));

        // A workspace file deleted since the answer no longer applies.
        write_launch_choice(&state, &launch, &FolderLaunch::Workspace(ws_file.clone()));
        std::fs::remove_file(&ws_file).unwrap();
        assert_eq!(read_launch_choice(&state, &launch), None);
    }
}
