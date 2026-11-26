//! What the host remembers between runs: the sessions open when it last
//! ended (reopened by `gaviero --sessions` with no path), the recently
//! opened paths offered by the new-session picker, and how each target was
//! opened (a folder that holds a `.gaviero-workspace` file reopens as the
//! workspace or as the plain folder, whichever the user chose last time).

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
    /// How each target was opened, keyed by its string form: the text of
    /// the tab's launch-state file. Seeded back into the tab next time, so
    /// a folder holding a workspace file opens the same way instead of
    /// asking again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub launches: BTreeMap<String, String>,
    /// Custom tab names set by the user (Ctrl+Alt+R), keyed by the target's
    /// string form. Survives quit/reopen, restarts, shell `cd` and SSH folder
    /// reports without reverting to the automatic label.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
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
    if let Some(path) = state_override() {
        return Some(path);
    }
    Some(dirs::data_dir()?.join("gaviero").join("session-host.json"))
}

/// The state file named by `GAVIERO_SESSION_HOST_STATE`, when set.
pub(crate) fn state_override() -> Option<PathBuf> {
    std::env::var_os(STATE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
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
/// `.gaviero-workspace` file; a shell's folder. SSH targets cannot be
/// checked from here and always count as openable.
pub(crate) fn still_openable(target: &Target) -> bool {
    match target {
        Target::Local(path) => local_openable(path),
        Target::Shell(dir) => dir.is_dir(),
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

/// The key a target's per-target host metadata is stored under: its string form.
fn target_key(target: &Target) -> String {
    String::from(target.clone())
}

impl HostState {
    /// `target` was opened in a tab: first in `recent`, and in `open`. A
    /// shell goes in `open` only: the recents are the picker's gaviero
    /// targets, and a shell's folder changes with every `cd`.
    pub fn note_opened(&mut self, target: &Target) {
        if !target.is_shell() {
            self.recent.retain(|p| p != target);
            self.recent.insert(0, target.clone());
            self.recent.truncate(RECENT_CAP);
        }
        if !self.open.iter().any(|p| p == target) {
            self.open.push(target.clone());
        }
    }

    /// `open` becomes exactly the tabs' targets, in tab order, each once —
    /// after a tab's target changed while another tab may share its old one
    /// (a shell that changed folder).
    pub fn set_open(&mut self, tabs: &[Target]) {
        self.open.clear();
        for target in tabs {
            if !self.open.contains(target) {
                self.open.push(target.clone());
            }
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

    /// The launch answer saved for `target`, if any.
    pub fn launch_choice(&self, target: &Target) -> Option<&str> {
        self.launches.get(&target_key(target)).map(String::as_str)
    }

    /// Remember how `target` was opened; true when that changed the saved
    /// answer (a target not seen before, or a different one).
    pub fn remember_launch(&mut self, target: &Target, launch: String) -> bool {
        let key = target_key(target);
        if self.launches.get(&key).is_some_and(|old| *old == launch) {
            return false;
        }
        self.launches.insert(key, launch);
        true
    }

    /// Drop launch answers for targets neither open nor recent, so the file
    /// does not grow with every folder ever opened.
    pub fn prune_launches(&mut self) {
        let keep: Vec<String> = self.open.iter().chain(&self.recent).map(target_key).collect();
        self.launches.retain(|key, _| keep.contains(key));
    }

    /// The custom tab name saved for `target`, if any.
    pub fn name_for(&self, target: &Target) -> Option<&str> {
        self.names
            .get(&target_key(target))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    }

    /// Replace the saved custom tab names from the currently open renamed
    /// tabs (first renamed tab wins if two tabs share a target).
    pub fn set_names(&mut self, entries: &[(&Target, &str)]) {
        self.names.clear();
        for &(target, name) in entries {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                self.names
                    .entry(target_key(target))
                    .or_insert_with(|| trimmed.to_string());
            }
        }
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
    fn launch_answers_are_remembered_per_target() {
        let mut state = HostState::default();
        let folder = l("C:\\work\\proj");
        let answer = "C:\\work\\proj\nworkspace\nC:\\work\\proj\\team.gaviero-workspace\n";
        assert_eq!(state.launch_choice(&folder), None);
        assert!(state.remember_launch(&folder, answer.into()));
        assert_eq!(state.launch_choice(&folder), Some(answer));
        assert!(
            !state.remember_launch(&folder, answer.into()),
            "the same answer is not written again"
        );
        assert!(state.remember_launch(&folder, "C:\\work\\proj\nfolder\n".into()));
        assert_eq!(
            state.launch_choice(&folder),
            Some("C:\\work\\proj\nfolder\n")
        );
        assert_eq!(state.launch_choice(&l("C:\\work\\other")), None);
    }

    #[test]
    fn launch_answers_round_trip_and_follow_open_and_recent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = HostState::default();
        let answer = "C:\\work\\proj\nworkspace\nC:\\work\\proj\\team.gaviero-workspace\n";
        state.note_opened(&l("C:\\work\\proj"));
        state.remember_launch(&l("C:\\work\\proj"), answer.into());
        state.remember_launch(&l("C:\\work\\gone"), "C:\\work\\gone\nfolder\n".into());
        save(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"launches\""), "{text}");
        assert_eq!(load(&path).launch_choice(&l("C:\\work\\proj")), Some(answer));

        // A target neither open nor recent loses its answer; an open one
        // keeps it.
        state.prune_launches();
        assert_eq!(state.launch_choice(&l("C:\\work\\proj")), Some(answer));
        assert_eq!(state.launch_choice(&l("C:\\work\\gone")), None);

        // No answers: the key is left out.
        save(&path, &HostState::default()).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("launches"));
    }

    #[test]
    fn shells_are_reopened_but_never_recent() {
        let dir = tempfile::tempdir().unwrap();
        let shell = Target::Shell(dir.path().to_path_buf());
        let mut state = HostState::default();
        state.note_opened(&l("a"));
        state.note_opened(&shell);
        assert_eq!(state.open, vec![l("a"), shell.clone()]);
        assert_eq!(state.recent, vec![l("a")]);
        assert!(still_openable(&shell));
        assert!(!still_openable(&Target::Shell(dir.path().join("gone"))));
    }

    #[test]
    fn the_open_set_follows_the_tabs_once_each() {
        let mut state = HostState::default();
        let (s1, s2) = (Target::Shell("/a".into()), Target::Shell("/b".into()));
        state.note_opened(&l("x"));
        state.note_opened(&s1);
        // Two shells were in `/a`; one moved to `/b`: both stay open.
        state.set_open(&[l("x"), s2.clone(), s1.clone(), s2.clone()]);
        assert_eq!(state.open, vec![l("x"), s2, s1]);
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

    #[test]
    fn custom_names_round_trip_omit_when_empty_and_ignore_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let a = l("C:\\work\\alpha");
        let sh = Target::Shell("/tmp/proj".into());
        let ssh = Target::Ssh { host: "vps".into(), path: Some("/srv".into()) };

        let mut state = HostState::default();
        state.set_names(&[(&a, "Backend"), (&a, "IgnoredSecond"), (&sh, "Logs"), (&ssh, "  Prod  ")]);
        assert_eq!(state.name_for(&a), Some("Backend"));
        assert_eq!(state.name_for(&sh), Some("Logs"));
        assert_eq!(state.name_for(&ssh), Some("Prod"));
        assert_eq!(state.name_for(&l("other")), None);

        save(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"names\""), "{text}");
        assert_eq!(load(&path), state);

        // Blank entries from hand edits are ignored.
        let mut corrupt = HostState::default();
        corrupt.names.insert(target_key(&a), "   ".into());
        assert_eq!(corrupt.name_for(&a), None);

        // No custom names: the key is left out.
        save(&path, &HostState::default()).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("names"));
    }
}
