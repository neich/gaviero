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

/// How long to wait for the executable to reappear: an installer moves the
/// old file aside before the new one is in place.
const EXE_WAIT: Duration = Duration::from_secs(30);

/// A child that asks for a restart sooner than this after starting is backed
/// off by [`RESTART_BACKOFF`], so a misbehaving build cannot spin the pane.
const MIN_RUN: Duration = Duration::from_secs(10);
const RESTART_BACKOFF: Duration = Duration::from_secs(5);

/// True in the TUI child of a supervisor — only then may the TUI exit with
/// [`RESTART_EXIT_CODE`] and expect to come back.
pub fn is_supervised() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Act as the supervisor when this process should be one.
///
/// Returns `Some(exit code)` after supervising (the caller exits with it), or
/// `None` when the caller should run the TUI itself: already a child, opted
/// out, no terminal, or the executable path is unknown.
pub fn supervise() -> Option<i32> {
    if is_supervised() || std::env::var_os(OPT_OUT_ENV).is_some() {
        return None;
    }
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    // The child owns the keyboard. A Ctrl+C / Ctrl+Break console event must
    // not kill this parent out from under it (closing the window still does).
    crate::platform::ignore_console_interrupts();

    loop {
        let started = Instant::now();
        let status = match run_child(&exe, &args) {
            Ok(status) => status,
            Err(e) => {
                eprintln!("gaviero: could not start {}: {e}", exe.display());
                return Some(1);
            }
        };
        match status.code() {
            Some(RESTART_EXIT_CODE) => {
                eprintln!("gaviero: new build installed, restarting");
                if started.elapsed() < MIN_RUN {
                    std::thread::sleep(RESTART_BACKOFF);
                }
            }
            Some(code) => return Some(code),
            // Killed by a signal (Unix): report failure like a shell would.
            None => return Some(1),
        }
    }
}

fn run_child(exe: &Path, args: &[OsString]) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + EXE_WAIT;
    loop {
        match Command::new(exe).args(args).env(CHILD_ENV, "1").status() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(500));
            }
            other => return other,
        }
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
}
