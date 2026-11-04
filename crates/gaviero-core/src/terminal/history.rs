//! Per-tab history file management and command log.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::config::ShellType;
use super::types::{CommandRecord, TerminalId};

/// Base directory for per-tab shell history files.
pub fn history_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("gaviero/history")
}

/// History file path for a specific terminal tab.
pub fn history_file_for(id: &TerminalId) -> PathBuf {
    history_dir().join(format!("tab-{}", id))
}

/// Ensure the history directory exists.
pub fn ensure_history_dir() -> Result<()> {
    std::fs::create_dir_all(history_dir()).context("creating history directory")?;
    Ok(())
}

/// Bytes read from the end of a history file by [`recent_commands`]. A
/// recall list never needs more, and the file can grow without bound.
const HISTORY_TAIL_BYTES: u64 = 256 * 1024;

/// The most recent distinct commands in a tab's shell history file, oldest
/// first, at most `limit`. A command that ran more than once appears only at
/// its latest position, the way PSReadLine and `HISTCONTROL=erasedups` recall
/// it. Multi-line commands are joined with `\n`.
///
/// The file is the shell's own (`HISTFILE` / PSReadLine `HistorySavePath`,
/// set per tab by the shell integration), so it holds every command that ran
/// in the tab, typed on the desktop or sent remotely. Fish ignores
/// `HISTFILE` and an unknown shell's format is unknown: both read as empty.
/// A missing or unreadable file is empty, never an error.
pub fn recent_commands(path: &Path, shell: &ShellType, limit: usize) -> Vec<String> {
    if limit == 0 || matches!(shell, ShellType::Fish | ShellType::Unknown(_)) {
        return Vec::new();
    }
    let Some(text) = read_tail(path, HISTORY_TAIL_BYTES) else {
        return Vec::new();
    };
    let entries = match shell {
        ShellType::PowerShell => parse_continued(&text, '`'),
        ShellType::Zsh => parse_continued(&text, '\\')
            .into_iter()
            .map(|e| strip_zsh_extended(&e).to_string())
            .collect(),
        ShellType::Bash => text
            .lines()
            .filter(|l| !is_bash_timestamp(l))
            .map(str::to_string)
            .collect(),
        ShellType::Fish | ShellType::Unknown(_) => Vec::new(),
    };
    latest_distinct(entries, limit)
}

/// The last `max` bytes of `path` as text, starting after the first line
/// break when the read began mid-file (that line is partial).
fn read_tail(path: &Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    Some(text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default())
}

/// Lines ending in `marker` continue on the next one (PSReadLine writes a
/// trailing backtick, zsh a trailing backslash); the marker is dropped.
fn parse_continued(text: &str, marker: char) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let (body, continues) = match line.strip_suffix(marker) {
            Some(body) => (body, true),
            None => (line, false),
        };
        let entry = match current.take() {
            Some(mut open) => {
                open.push('\n');
                open.push_str(body);
                open
            }
            None => body.to_string(),
        };
        if continues {
            current = Some(entry);
        } else {
            entries.push(entry);
        }
    }
    entries.extend(current);
    entries
}

/// `: 1700000000:0;git status` → `git status` (zsh `EXTENDED_HISTORY`).
fn strip_zsh_extended(entry: &str) -> &str {
    entry
        .strip_prefix(": ")
        .and_then(|rest| rest.split_once(';'))
        .filter(|(stamp, _)| stamp.chars().all(|c| c.is_ascii_digit() || c == ':'))
        .map(|(_, command)| command)
        .unwrap_or(entry)
}

/// `#1700000000` lines precede entries when bash's `HISTTIMEFORMAT` is set.
fn is_bash_timestamp(line: &str) -> bool {
    line.strip_prefix('#')
        .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
}

fn latest_distinct(entries: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut newest_first: Vec<String> = Vec::new();
    for entry in entries.into_iter().rev() {
        if newest_first.len() == limit {
            break;
        }
        if entry.trim().is_empty() || !seen.insert(entry.clone()) {
            continue;
        }
        newest_first.push(entry);
    }
    newest_first.reverse();
    newest_first
}

/// Bounded ring buffer of recent command records.
#[derive(Debug)]
pub struct CommandLog {
    entries: VecDeque<CommandRecord>,
    max: usize,
}

impl CommandLog {
    pub fn new(max: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max),
            max,
        }
    }

    /// Push a new record, evicting the oldest if at capacity.
    pub fn push(&mut self, record: CommandRecord) {
        if self.entries.len() >= self.max {
            self.entries.pop_front();
        }
        self.entries.push_back(record);
    }

    /// Get the most recent N records.
    pub fn recent(&self, n: usize) -> Vec<&CommandRecord> {
        let start = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(start).collect()
    }

    /// All entries as a slice.
    pub fn entries(&self) -> &VecDeque<CommandRecord> {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tab-1");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn pwsh_history_joins_backtick_continuations_and_keeps_latest_duplicates() {
        let (_dir, path) = write(
            "git status\r\ncargo build\r\nif ($x) {`\r\n  echo hi`\r\n}\r\ngit status\r\n",
        );
        assert_eq!(
            recent_commands(&path, &ShellType::PowerShell, 10),
            vec!["cargo build", "if ($x) {\n  echo hi\n}", "git status"],
        );
        assert_eq!(recent_commands(&path, &ShellType::PowerShell, 1), vec!["git status"]);
    }

    #[test]
    fn bash_skips_timestamps_and_zsh_strips_extended_prefix() {
        let (_dir, bash) = write("#1700000000\nls -la\n#1700000001\necho done\n");
        assert_eq!(recent_commands(&bash, &ShellType::Bash, 10), vec!["ls -la", "echo done"]);
        let (_dir2, zsh) = write(": 1700000000:0;ls\n: 1700000001:3;echo a\\\nb\n");
        assert_eq!(recent_commands(&zsh, &ShellType::Zsh, 10), vec!["ls", "echo a\nb"]);
    }

    #[test]
    fn missing_files_fish_and_unknown_shells_read_empty() {
        let (_dir, path) = write("ls\n");
        assert!(recent_commands(&path, &ShellType::Fish, 10).is_empty());
        assert!(recent_commands(&path, &ShellType::Unknown("cmd".into()), 10).is_empty());
        assert!(recent_commands(&path.with_extension("gone"), &ShellType::Bash, 10).is_empty());
        assert!(recent_commands(&path, &ShellType::Bash, 0).is_empty());
    }

    #[test]
    fn a_tail_read_drops_the_partial_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tab-1");
        let mut text = "x".repeat(HISTORY_TAIL_BYTES as usize);
        text.push_str("\nlast\n");
        std::fs::write(&path, text).unwrap();
        assert_eq!(recent_commands(&path, &ShellType::Bash, 10), vec!["last"]);
    }
}
