//! What the host remembers between runs: the sessions open when it last
//! ended (reopened by `gaviero --sessions` with no path) and the recently
//! opened paths offered by the new-session picker.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Overrides the state file location (tests).
const STATE_ENV: &str = "GAVIERO_SESSION_HOST_STATE";

/// How many recent paths are kept.
const RECENT_CAP: usize = 20;

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostState {
    /// Open sessions, in tab order.
    #[serde(default)]
    pub open: Vec<PathBuf>,
    /// Recently opened paths, most recent first.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
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

/// A saved path that can still be opened: a folder, or a
/// `.gaviero-workspace` file.
pub(crate) fn still_openable(path: &Path) -> bool {
    path.is_dir()
        || (path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "gaviero-workspace"))
}

impl HostState {
    /// `path` was opened in a tab: first in `recent`, and in `open`.
    pub fn note_opened(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(RECENT_CAP);
        if !self.open.iter().any(|p| p == path) {
            self.open.push(path.to_path_buf());
        }
    }

    /// The tabs were reordered: `open` follows `tabs` (one entry per path,
    /// at its first tab). Paths in `open` with no tab keep their order after
    /// the rest.
    pub fn reorder_open(&mut self, tabs: &[PathBuf]) {
        let mut open: Vec<PathBuf> = Vec::with_capacity(self.open.len());
        for path in tabs.iter().chain(self.open.iter()) {
            if !open.contains(path) {
                open.push(path.clone());
            }
        }
        self.open = open;
    }

    /// The tab for `path` closed.
    pub fn note_closed(&mut self, path: &Path) {
        self.open.retain(|p| p != path);
    }

    /// The tab for `path` ended. During a quit all the open set is frozen,
    /// so the next `gaviero --sessions` reopens what was quit.
    pub fn note_tab_ended(&mut self, path: &Path, quitting_all: bool) {
        if !quitting_all {
            self.note_closed(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_are_most_recent_first_deduplicated_and_capped() {
        let mut state = HostState::default();
        for i in 0..25 {
            state.note_opened(Path::new(&format!("p{i}")));
        }
        assert_eq!(state.recent.len(), RECENT_CAP);
        assert_eq!(state.recent[0], PathBuf::from("p24"));
        assert_eq!(state.recent[19], PathBuf::from("p5"));

        state.note_opened(Path::new("p10"));
        assert_eq!(state.recent[0], PathBuf::from("p10"));
        assert_eq!(state.recent.iter().filter(|p| *p == Path::new("p10")).count(), 1);
        assert_eq!(state.recent.len(), RECENT_CAP);
        assert_eq!(state.open.len(), 25, "open keeps every tab, in order");
        assert_eq!(state.open[0], PathBuf::from("p0"));
    }

    #[test]
    fn closing_removes_from_open_only() {
        let mut state = HostState::default();
        state.note_opened(Path::new("a"));
        state.note_opened(Path::new("b"));
        state.note_closed(Path::new("a"));
        assert_eq!(state.open, vec![PathBuf::from("b")]);
        assert_eq!(state.recent, vec![PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn open_set_is_kept_while_quitting_all() {
        let mut state = HostState::default();
        state.note_opened(Path::new("a"));
        state.note_opened(Path::new("b"));
        state.note_tab_ended(Path::new("b"), true);
        state.note_tab_ended(Path::new("a"), true);
        assert_eq!(state.open, vec![PathBuf::from("a"), PathBuf::from("b")]);
        state.note_tab_ended(Path::new("a"), false);
        assert_eq!(state.open, vec![PathBuf::from("b")]);
    }

    #[test]
    fn reordering_follows_the_tabs_once_per_path() {
        let p = |s: &str| PathBuf::from(s);
        let mut state = HostState::default();
        for name in ["a", "b", "c"] {
            state.note_opened(Path::new(name));
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
        state.note_opened(Path::new("a"));
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
    fn only_folders_and_workspace_files_are_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("w.gaviero-workspace");
        let txt = dir.path().join("x.txt");
        std::fs::write(&ws, "{}").unwrap();
        std::fs::write(&txt, "").unwrap();
        assert!(still_openable(dir.path()));
        assert!(still_openable(&ws));
        assert!(!still_openable(&txt));
        assert!(!still_openable(&dir.path().join("gone")));
    }
}
