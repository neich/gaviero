//! Host side of restart-on-new-build (see [`crate::supervisor`]).
//!
//! The host stats its own executable like a TUI does ([`BuildWatch`]). Once
//! a new build has settled:
//!
//! - **Supervised** (`supervisor::supervise_host`): the host waits until no
//!   tab has work in progress (an agent, swarm or shell command running,
//!   unsaved files), no host dialog is open and the keyboard and mouse have
//!   been quiet for [`INPUT_QUIET`]. Then it records its tabs in the
//!   supervisor's restart file ([`RestartRecord`]), asks every tab to exit
//!   (each saves its session state) and exits with the restart code. The
//!   new host reopens the same tabs, in order, with the same current tab and
//!   each tab's answer to the "open the workspace?" launch question.
//!   Its tabs carry [`super::HOST_RESTARTS_ENV`] and never restart on their
//!   own, so a deploy restarts each of them once, with the host.
//! - **Not supervised**: nothing would start it again, so the tab bar says
//!   once that a new build is installed and the host must be restarted.

use crate::supervisor::BuildWatch;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// No restart within this long of the last key or click on the host.
const INPUT_QUIET: Duration = Duration::from_secs(15);

/// Repeat the "restart pending" bar message this often while it waits.
const NOTICE_EVERY: Duration = Duration::from_secs(30);

/// One tab of a host that is restarting.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestartTab {
    pub path: PathBuf,
    /// The tab's launch-state file (`supervisor::remember_launch_choice`),
    /// verbatim, so the restarted tab does not ask again.
    #[serde(default)]
    pub launch: Option<String>,
}

/// What a restarting host hands to the next one.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RestartRecord {
    pub tabs: Vec<RestartTab>,
    #[serde(default)]
    pub active: usize,
}

/// Write `record` through a temporary file and a rename.
pub(crate) fn save_record(path: &Path, record: &RestartRecord) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(record).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Read and delete the record a restarting host left; `None` when there is
/// none (the first run under this supervisor) or it is unreadable. Tabs
/// whose path no longer exists are dropped.
pub(crate) fn take_record(path: &Path) -> Option<RestartRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    let mut record: RestartRecord = serde_json::from_str(&text)
        .map_err(|e| tracing::warn!("ignoring corrupt restart record {}: {e}", path.display()))
        .ok()?;
    let active = record.tabs.get(record.active).map(|t| t.path.clone());
    record.tabs.retain(|t| super::state::still_openable(&t.path));
    record.active = active
        .and_then(|p| record.tabs.iter().position(|t| t.path == p))
        .unwrap_or(0);
    (!record.tabs.is_empty()).then_some(record)
}

/// What the host knows about one tab when it decides.
pub(crate) struct TabWork<'a> {
    pub label: &'a str,
    pub work: &'a [&'static str],
}

/// Why the host cannot restart yet; `None` when it can.
pub(crate) fn blocker(
    tabs: &[TabWork<'_>],
    modal_open: bool,
    since_input: Option<Duration>,
) -> Option<String> {
    if let Some(tab) = tabs.iter().find(|t| !t.work.is_empty()) {
        return Some(format!("{}: {}", tab.label, tab.work.join(", ")));
    }
    if modal_open {
        return Some("a dialog is open".to_string());
    }
    if since_input.is_some_and(|d| d < INPUT_QUIET) {
        return Some("you are using it".to_string());
    }
    None
}

pub(crate) struct HostRestart {
    watch: BuildWatch,
    /// The supervisor's restart file; `None` when unsupervised.
    pub record: Option<PathBuf>,
    last_input: Option<Instant>,
    last_notice: Option<Instant>,
    /// Unsupervised: the "restart by hand" message was shown.
    told_manual: bool,
    /// Saving the record failed: this host stays on the old build.
    gave_up: bool,
    /// The host is quitting to restart; `run` exits with the restart code.
    pub requested: bool,
}

/// What the host should do on this check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Nothing,
    /// Show this message on the tab bar.
    Notify(String),
    Restart,
}

impl HostRestart {
    pub(crate) fn new() -> Self {
        Self {
            watch: BuildWatch::for_current_exe(),
            record: crate::supervisor::host_restart_record(),
            last_input: None,
            last_notice: None,
            told_manual: false,
            gave_up: false,
            requested: false,
        }
    }

    pub(crate) fn supervised(&self) -> bool {
        self.record.is_some()
    }

    pub(crate) fn note_input(&mut self) {
        self.last_input = Some(Instant::now());
    }

    pub(crate) fn give_up(&mut self) {
        self.gave_up = true;
    }

    /// Poll the executable and decide. `blocker` is only evaluated once a
    /// new build is installed.
    pub(crate) fn check(
        &mut self,
        now: Instant,
        blocker: impl FnOnce(Option<Duration>) -> Option<String>,
    ) -> Step {
        self.watch.poll(now);
        if !self.watch.ready() || self.requested || self.gave_up {
            return Step::Nothing;
        }
        if !self.supervised() {
            if self.told_manual {
                return Step::Nothing;
            }
            self.told_manual = true;
            return Step::Notify(
                "New gaviero build installed: quit (Ctrl+Q) and start gaviero --sessions again to use it"
                    .to_string(),
            );
        }
        let since_input = self.last_input.map(|at| now.duration_since(at));
        match blocker(since_input) {
            None => Step::Restart,
            Some(reason) => {
                let due = self
                    .last_notice
                    .is_none_or(|last| now.duration_since(last) >= NOTICE_EVERY);
                if !due {
                    return Step::Nothing;
                }
                self.last_notice = Some(now);
                Step::Notify(format!(
                    "New gaviero build installed: restarting once idle ({reason})"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_in_any_tab_blocks_the_restart() {
        let busy = ["an agent is running"];
        let tabs = [
            TabWork { label: "alpha", work: &[] },
            TabWork { label: "beta", work: &busy },
        ];
        assert_eq!(
            blocker(&tabs, false, None).as_deref(),
            Some("beta: an agent is running")
        );
    }

    #[test]
    fn dialogs_and_recent_input_delay_the_restart() {
        let idle = [TabWork { label: "alpha", work: &[] }];
        assert_eq!(blocker(&idle, true, None).as_deref(), Some("a dialog is open"));
        assert_eq!(
            blocker(&idle, false, Some(Duration::from_secs(3))).as_deref(),
            Some("you are using it")
        );
        assert_eq!(blocker(&idle, false, Some(INPUT_QUIET)), None);
        // No input since the host started.
        assert_eq!(blocker(&idle, false, None), None);
    }

    #[test]
    fn the_record_round_trips_and_is_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let path = dir.path().join("cache").join("restart.json");
        let record = RestartRecord {
            tabs: vec![
                RestartTab { path: a.clone(), launch: None },
                RestartTab { path: b.clone(), launch: Some("x\nfolder\n".into()) },
            ],
            active: 1,
        };
        save_record(&path, &record).unwrap();
        assert_eq!(take_record(&path), Some(record));
        assert!(!path.exists(), "a record is read once");
        assert_eq!(take_record(&path), None);
    }

    #[test]
    fn vanished_tabs_are_dropped_and_the_current_tab_follows() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let c = dir.path().join("c");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&c).unwrap();
        let path = dir.path().join("restart.json");
        let tab = |p: &Path| RestartTab { path: p.to_path_buf(), launch: None };
        let record = RestartRecord {
            tabs: vec![tab(&a), tab(&dir.path().join("gone")), tab(&c)],
            active: 2,
        };
        save_record(&path, &record).unwrap();
        let got = take_record(&path).unwrap();
        assert_eq!(got.tabs, vec![tab(&a), tab(&c)]);
        assert_eq!(got.active, 1, "still c");

        // Nothing left to open: no record.
        save_record(&path, &RestartRecord { tabs: vec![tab(&dir.path().join("gone"))], active: 0 }).unwrap();
        assert_eq!(take_record(&path), None);
    }

    #[test]
    fn a_corrupt_record_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("restart.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(take_record(&path), None);
    }
}
