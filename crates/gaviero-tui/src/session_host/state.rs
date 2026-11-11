//! What the host remembers between runs: the sessions open when it last
//! ended (reopened by `gaviero --sessions` with no path) and the recently
//! opened paths offered by the new-session picker.

use super::target::Target;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Overrides the state file location (tests).
const STATE_ENV: &str = "GAVIERO_SESSION_HOST_STATE";

/// How many recent paths are kept.
const RECENT_CAP: usize = 20;

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostState {
    /// Open sessions, in tab order.
    #[serde(default)]
    pub open: Vec<Target>,
    /// Recently opened paths, most recent first.
    #[serde(default)]
    pub recent: Vec<Target>,
    /// Per-host settings for SSH tabs, edited by hand (`sshHosts`). The
    /// host never changes them: it re-reads them before every save.
    #[serde(default, rename = "sshHosts", skip_serializing_if = "BTreeMap::is_empty")]
    pub ssh_hosts: BTreeMap<String, SshHost>,
}

/// Settings for one SSH host (`sshHosts.<host>` in the state file).
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct SshHost {
    /// Where gaviero is on that machine when it is not on the login PATH
    /// (`~/` is the remote home).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gaviero: Option<String>,
}

/// `GAVIERO_SESSION_HOST_STATE` if set, else
/// `<data dir>/gaviero/session-host.json`.
pub(crate) fn state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(STATE_ENV) {
        return Some(PathBuf::from(path));
    }
    Some(dirs::data_dir()?.join("gaviero").join("session-host.json"))
}

/// The saved state; a missing or unreadable file is an empty state.
pub(crate) fn load(path: &Path) -> HostState {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HostState::default(),
        Err(e) => {
            tracing::warn!("reading {}: {e}", path.display());
            return HostState::default();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!("ignoring corrupt session host state {}: {e}", path.display());
        HostState::default()
    })
}

/// Write the state through a temporary file and a rename.
pub(crate) fn save(path: &Path, state: &HostState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Proof that this process is the only session host using a state file:
/// an exclusive OS lock on `session-host.lock` beside it. The OS drops the
/// lock when the process ends, however it ends, so a crashed host never
/// leaves a stale lock behind.
#[derive(Debug)]
pub(crate) struct InstanceLock {
    _file: std::fs::File,
}

/// The lock file guarding the state file at `state`.
pub(crate) fn lock_path(state: &Path) -> PathBuf {
    state.with_extension("lock")
}

/// Take the instance lock for the state file at `state`: `Ok(Some)` when
/// this process is now the only host, `Ok(None)` when another process
/// holds it, `Err` when the lock could not be tried at all.
pub(crate) fn lock_instance(state: &Path) -> std::io::Result<Option<InstanceLock>> {
    let path = lock_path(state);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // No truncate: another host may hold the lock on this file.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(InstanceLock { _file: file })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// A saved target that can still be opened: a folder, or a
/// `.gaviero-workspace` file. SSH targets cannot be checked from here and
/// always count as openable.
pub(crate) fn still_openable(target: &Target) -> bool {
    match target {
        Target::Local(path) => local_openable(path),
        Target::Ssh { .. } => true,
    }
}

fn local_openable(path: &Path) -> bool {
    path.is_dir()
        || (path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "gaviero-workspace"))
}

impl HostState {
    /// `target` was opened in a tab: first in `recent`, and in `open`.
    pub fn note_opened(&mut self, target: &Target) {
        self.recent.retain(|p| p != target);
        self.recent.insert(0, target.clone());
        self.recent.truncate(RECENT_CAP);
        if !self.open.iter().any(|p| p == target) {
            self.open.push(target.clone());
        }
    }

    /// A tab's target changed from `old` to `new` (an SSH tab learned its
    /// remote folder): `new` takes `old`'s place in `open`, or is added
    /// when `old` is not there. Never listed twice.
    pub fn replace_open(&mut self, old: &Target, new: &Target) {
        if old == new {
            return;
        }
        match self.open.iter().position(|p| p == old) {
            Some(pos) => {
                if self.open.iter().any(|p| p == new) {
                    self.open.remove(pos);
                } else {
                    self.open[pos] = new.clone();
                }
            }
            None => {
                if !self.open.iter().any(|p| p == new) {
                    self.open.push(new.clone());
                }
            }
        }
    }

    /// The tabs were reordered: `open` follows `tabs` (one entry per path,
    /// at its first tab). Paths in `open` with no tab keep their order after
    /// the rest.
    pub fn reorder_open(&mut self, tabs: &[Target]) {
        let mut open: Vec<Target> = Vec::with_capacity(self.open.len());
        for path in tabs.iter().chain(self.open.iter()) {
            if !open.contains(path) {
                open.push(path.clone());
            }
        }
        self.open = open;
    }

    /// The tab for `target` closed.
    pub fn note_closed(&mut self, target: &Target) {
        self.open.retain(|p| p != target);
    }

    /// The tab for `path` ended. During a quit all the open set is frozen,
    /// so the next `gaviero --sessions` reopens what was quit.
    pub fn note_tab_ended(&mut self, target: &Target, quitting_all: bool) {
        if !quitting_all {
            self.note_closed(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local target.
    fn l(path: impl AsRef<Path>) -> Target {
        Target::Local(path.as_ref().to_path_buf())
    }

    #[test]
    fn recents_are_most_recent_first_deduplicated_and_capped() {
        let mut state = HostState::default();
        for i in 0..25 {
            state.note_opened(&l(&format!("p{i}")));
        }
        assert_eq!(state.recent.len(), RECENT_CAP);
        assert_eq!(state.recent[0], l("p24"));
        assert_eq!(state.recent[19], l("p5"));

        state.note_opened(&l("p10"));
        assert_eq!(state.recent[0], l("p10"));
        assert_eq!(state.recent.iter().filter(|p| **p == l("p10")).count(), 1);
        assert_eq!(state.recent.len(), RECENT_CAP);
        assert_eq!(state.open.len(), 25, "open keeps every tab, in order");
        assert_eq!(state.open[0], l("p0"));
    }

    #[test]
    fn closing_removes_from_open_only() {
        let mut state = HostState::default();
        state.note_opened(&l("a"));
        state.note_opened(&l("b"));
        state.note_closed(&l("a"));
        assert_eq!(state.open, vec![l("b")]);
        assert_eq!(state.recent, vec![l("b"), l("a")]);
    }

    #[test]
    fn open_set_is_kept_while_quitting_all() {
        let mut state = HostState::default();
        state.note_opened(&l("a"));
        state.note_opened(&l("b"));
        state.note_tab_ended(&l("b"), true);
        state.note_tab_ended(&l("a"), true);
        assert_eq!(state.open, vec![l("a"), l("b")]);
        state.note_tab_ended(&l("a"), false);
        assert_eq!(state.open, vec![l("b")]);
    }

    #[test]
    fn reordering_follows_the_tabs_once_per_path() {
        let p = |s: &str| l(s);
        let mut state = HostState::default();
        for name in ["a", "b", "c"] {
            state.note_opened(&l(name));
        }
        let recent = state.recent.clone();
        state.reorder_open(&[p("b"), p("a"), p("c")]);
        assert_eq!(state.open, vec![p("b"), p("a"), p("c")]);
        // `a` open in two tabs: listed once, at its first tab.
        state.reorder_open(&[p("c"), p("a"), p("b"), p("a")]);
        assert_eq!(state.open, vec![p("c"), p("a"), p("b")]);
        assert_eq!(state.recent, recent, "recents are untouched");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("state.json");
        let mut state = HostState::default();
        state.note_opened(&l("a"));
        save(&path, &state).unwrap();
        assert_eq!(load(&path), state);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_or_corrupt_state_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert_eq!(load(&path), HostState::default());
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), HostState::default());
    }

    #[test]
    fn a_second_host_cannot_take_the_instance_lock() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("sub").join("session-host.json");
        let first = lock_instance(&state).unwrap();
        assert!(first.is_some(), "the first host gets the lock");
        assert!(lock_path(&state).exists());
        assert!(
            lock_instance(&state).unwrap().is_none(),
            "a second host is refused while the first runs"
        );
        drop(first);
        assert!(
            lock_instance(&state).unwrap().is_some(),
            "the lock is free once the first host is gone"
        );
        let other = dir.path().join("other.json");
        let _held = lock_instance(&state).unwrap();
        assert!(
            lock_instance(&other).unwrap().is_some(),
            "a host with its own state file is independent"
        );
    }

    #[test]
    fn only_folders_and_workspace_files_are_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("w.gaviero-workspace");
        let txt = dir.path().join("x.txt");
        std::fs::write(&ws, "{}").unwrap();
        std::fs::write(&txt, "").unwrap();
        assert!(still_openable(&l(dir.path())));
        assert!(still_openable(&l(&ws)));
        assert!(!still_openable(&l(&txt)));
        assert!(!still_openable(&l(dir.path().join("gone"))));
        let remote = Target::Ssh { host: "vps".into(), path: Some("/gone".into()) };
        assert!(still_openable(&remote), "remote targets cannot be checked here");
    }

    #[test]
    fn a_legacy_state_file_loads_local_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"open":["C:\\a"],"recent":["C:\\a","/b"]}"#).unwrap();
        let state = load(&path);
        assert_eq!(state.open, vec![l("C:\\a")]);
        assert_eq!(state.recent, vec![l("C:\\a"), l("/b")]);
        assert!(state.ssh_hosts.is_empty());
    }

    #[test]
    fn ssh_targets_and_ssh_hosts_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = HostState::default();
        let remote = Target::Ssh { host: "vps".into(), path: Some("/srv/a b".into()) };
        state.note_opened(&remote);
        state.ssh_hosts.insert(
            "vps".into(),
            SshHost { gaviero: Some("~/.cargo/bin/gaviero".into()) },
        );
        save(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"ssh://vps/srv/a b\""), "{text}");
        assert!(text.contains("\"sshHosts\""), "{text}");
        assert_eq!(load(&path), state);
        // No hosts: the key is left out.
        save(&path, &HostState::default()).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("sshHosts"));
    }

    #[test]
    fn replacing_an_open_target_keeps_its_place() {
        let mut state = HostState::default();
        let picking = Target::Ssh { host: "vps".into(), path: None };
        let picked = Target::Ssh { host: "vps".into(), path: Some("/srv".into()) };
        state.note_opened(&l("a"));
        state.note_opened(&picking);
        state.note_opened(&l("b"));
        state.replace_open(&picking, &picked);
        assert_eq!(state.open, vec![l("a"), picked.clone(), l("b")]);
        // Already there: the old entry just goes.
        state.note_opened(&picking);
        state.replace_open(&picking, &picked);
        assert_eq!(state.open, vec![l("a"), picked.clone(), l("b")]);
        // Old one gone: the new one is added once.
        state.replace_open(&l("zzz"), &l("c"));
        assert_eq!(state.open.last(), Some(&l("c")));
    }
}
