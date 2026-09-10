//! Crash-durability journal for chat prompts.
//!
//! [`session_state`](crate::session_state) only reaches disk when the TUI
//! exits cleanly — `gaviero-tui/src/main.rs` calls `save_session` exactly
//! once, after the event loop breaks on `should_quit`. A panic, a closed
//! terminal window, a dropped SSH connection, or a lost host therefore
//! discards every prompt typed since the last clean quit.
//!
//! This module closes that window with an append-only NDJSON log written
//! *before* the prompt is dispatched to the agent:
//!
//! ```text
//! <state_dir>/conversations/journal.ndjson
//! ```
//!
//! Contract:
//!
//! * **Append + fsync per prompt.** A prompt is durable the moment the user
//!   presses Enter, not when the turn completes. One prompt is one physical
//!   line: `serde_json` escapes embedded newlines, so a multi-line prompt
//!   cannot break the NDJSON framing.
//! * **Checkpointed on clean save.** [`checkpoint`] truncates the log once
//!   `save_conversations` has written every conversation successfully. So
//!   whatever survives to the next startup is exactly the set of prompts the
//!   previous session failed to persist — replay needs no de-duplication
//!   against the saved JSON, and no stable message id.
//! * **Tolerant reads.** A crash mid-append can leave a torn final line;
//!   [`load`] drops lines that don't parse instead of failing the load.
//!
//! Known limitation: closing a conversation is an in-memory action, so a
//! crash between the close and the next clean save replays that
//! conversation's journalled prompts into a recovered conversation. The
//! entries are labelled as recovered, and closing it again is harmless. If
//! that becomes annoying, the fix is to give [`JournalEntry`] an `op` field
//! and write a `close` tombstone from the close path.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Filename of the journal, inside the workspace's `conversations/` dir.
pub const JOURNAL_FILE: &str = "journal.ndjson";

/// One journalled prompt.
///
/// `conv_title` is carried so a conversation that crashed before its *first*
/// save can be rebuilt with the title the user actually saw, rather than a
/// generic placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Unix timestamp (seconds) at which the prompt was dispatched.
    pub ts: u64,
    /// Conversation the prompt belongs to.
    pub conv_id: String,
    /// Conversation title at dispatch time.
    #[serde(default)]
    pub conv_title: String,
    /// Message role. Only `"user"` is written today; the field exists so a
    /// later revision can journal assistant turns without a format break.
    #[serde(default = "default_role")]
    pub role: String,
    /// Verbatim prompt text.
    pub text: String,
}

fn default_role() -> String {
    "user".to_string()
}

/// Path of the journal for a workspace, or `None` when no data directory is
/// available (same failure mode as [`crate::session_state::state_dir_for`]).
pub fn journal_path(workspace_key: &Path) -> Option<PathBuf> {
    Some(
        crate::session_state::state_dir_for(workspace_key)?
            .join("conversations")
            .join(JOURNAL_FILE),
    )
}

/// Append one user prompt to the journal and flush it to stable storage.
///
/// Called on the dispatch path, so it is deliberately synchronous: the point
/// is that the prompt has hit the disk before the agent is spawned. The cost
/// is one small append plus an `fsync` — a few milliseconds against a turn
/// that is about to take seconds.
///
/// Callers must treat failure as non-fatal: a journal that cannot be written
/// must never block the user's turn.
pub fn append_prompt(
    workspace_key: &Path,
    conv_id: &str,
    conv_title: &str,
    text: &str,
) -> Result<()> {
    let entry = JournalEntry {
        ts: crate::session_state::now_unix(),
        conv_id: conv_id.to_string(),
        conv_title: conv_title.to_string(),
        role: "user".to_string(),
        text: text.to_string(),
    };
    append_entry(workspace_key, &entry)
}

/// Append an already-built entry. Split out so tests can control `ts`.
pub fn append_entry(workspace_key: &Path, entry: &JournalEntry) -> Result<()> {
    let path = journal_path(workspace_key).context("could not determine journal path")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating journal dir {}", parent.display()))?;
    }

    // `to_string` (not `to_string_pretty`) keeps the entry on one line, and
    // escapes any newline inside `text` — the NDJSON framing invariant.
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening journal {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to journal {}", path.display()))?;
    file.flush()?;
    // Durability across a host crash / power loss, not just a process crash.
    file.sync_data()
        .with_context(|| format!("syncing journal {}", path.display()))?;
    Ok(())
}

/// Read every intact entry, in append order.
///
/// Unparseable lines are skipped rather than fatal: the last line of the file
/// may be torn if the process died mid-append, and a partially written entry
/// must not cost us the intact ones before it.
pub fn load(workspace_key: &Path) -> Vec<JournalEntry> {
    let Some(path) = journal_path(workspace_key) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    let mut skipped = 0usize;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::warn!(
            "prompt journal {}: skipped {} unparseable line(s) (expected if the last session was killed mid-append)",
            path.display(),
            skipped
        );
    }
    entries
}

/// Whether the journal holds anything to replay.
pub fn is_empty(workspace_key: &Path) -> bool {
    journal_path(workspace_key)
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() == 0)
        .unwrap_or(true)
}

/// Truncate the journal after a fully successful save.
///
/// Only call this once every conversation *and* the index have been written:
/// the invariant that makes replay de-duplication-free is "the journal holds
/// exactly what the saved JSON does not".
///
/// Residual window: a crash *between* the last successful save and this
/// truncate replays prompts that are already in the JSON, so they appear
/// twice — once normally, once under a recovery marker. That is the safe side
/// of the trade (duplicated over lost), and the window is a single metadata
/// operation wide.
pub fn checkpoint(workspace_key: &Path) -> Result<()> {
    let Some(path) = journal_path(workspace_key) else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    std::fs::write(&path, b"").with_context(|| format!("truncating journal {}", path.display()))?;
    Ok(())
}

/// Drop every entry belonging to `conv_id`, keeping the rest.
///
/// Rewrites the file, so it is not append-atomic — reserved for explicit user
/// actions (deleting a conversation), never the dispatch path.
pub fn forget_conversation(workspace_key: &Path, conv_id: &str) -> Result<()> {
    let Some(path) = journal_path(workspace_key) else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let kept: Vec<JournalEntry> = load(workspace_key)
        .into_iter()
        .filter(|e| e.conv_id != conv_id)
        .collect();

    let mut body = String::new();
    for entry in &kept {
        body.push_str(&serde_json::to_string(entry)?);
        body.push('\n');
    }
    std::fs::write(&path, body.as_bytes())
        .with_context(|| format!("rewriting journal {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point the journal at a scratch dir by using the tempdir itself as the
    /// workspace key — `state_dir_for` hashes the path, so each test gets a
    /// distinct slot under the real data dir. Cleaned up at the end.
    struct Scratch {
        _dir: tempfile::TempDir,
        key: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let key = dir.path().to_path_buf();
            Self { _dir: dir, key }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(parent) = journal_path(&self.key).as_deref().and_then(Path::parent) {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    fn entry(conv: &str, text: &str) -> JournalEntry {
        JournalEntry {
            ts: 1_700_000_000,
            conv_id: conv.to_string(),
            conv_title: "T".to_string(),
            role: "user".to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn append_then_load_round_trips_in_order() {
        let s = Scratch::new();
        append_entry(&s.key, &entry("c1", "first")).unwrap();
        append_entry(&s.key, &entry("c1", "second")).unwrap();
        append_entry(&s.key, &entry("c2", "other")).unwrap();

        let loaded = load(&s.key);
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].text, "first");
        assert_eq!(loaded[1].text, "second");
        assert_eq!(loaded[2].conv_id, "c2");
    }

    #[test]
    fn load_on_missing_journal_is_empty_not_an_error() {
        let s = Scratch::new();
        assert!(load(&s.key).is_empty());
        assert!(is_empty(&s.key));
    }

    #[test]
    fn multi_line_prompt_stays_on_one_physical_line() {
        // The NDJSON framing invariant: a prompt containing newlines must not
        // be readable as several entries.
        let s = Scratch::new();
        append_entry(&s.key, &entry("c1", "line one\nline two\nline three")).unwrap();

        let raw = std::fs::read_to_string(journal_path(&s.key).unwrap()).unwrap();
        assert_eq!(raw.lines().count(), 1, "entry must occupy exactly one line");

        let loaded = load(&s.key);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "line one\nline two\nline three");
    }

    #[test]
    fn torn_final_line_is_dropped_and_earlier_entries_survive() {
        // Simulates a crash mid-append: the process died partway through
        // writing the last record. Everything before it must still load.
        let s = Scratch::new();
        append_entry(&s.key, &entry("c1", "durable one")).unwrap();
        append_entry(&s.key, &entry("c1", "durable two")).unwrap();

        let path = journal_path(&s.key).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(br#"{"ts":1700000000,"conv_id":"c1","conv_ti"#)
            .unwrap();
        drop(f);

        let loaded = load(&s.key);
        assert_eq!(loaded.len(), 2, "torn tail must not cost us intact entries");
        assert_eq!(loaded[1].text, "durable two");
    }

    #[test]
    fn checkpoint_empties_the_journal() {
        let s = Scratch::new();
        append_entry(&s.key, &entry("c1", "gone after checkpoint")).unwrap();
        assert!(!is_empty(&s.key));

        checkpoint(&s.key).unwrap();
        assert!(is_empty(&s.key));
        assert!(load(&s.key).is_empty());
    }

    #[test]
    fn checkpoint_on_missing_journal_is_a_noop() {
        let s = Scratch::new();
        checkpoint(&s.key).unwrap();
    }

    #[test]
    fn forget_conversation_keeps_other_conversations() {
        let s = Scratch::new();
        append_entry(&s.key, &entry("c1", "keep me")).unwrap();
        append_entry(&s.key, &entry("c2", "drop me")).unwrap();
        append_entry(&s.key, &entry("c1", "keep me too")).unwrap();

        forget_conversation(&s.key, "c2").unwrap();

        let loaded = load(&s.key);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().all(|e| e.conv_id == "c1"));
    }

    #[test]
    fn entry_without_role_or_title_still_parses() {
        // Forward/backward compatibility: a reader must tolerate a record
        // written by a build that predates these fields.
        let json = r#"{"ts":1,"conv_id":"c1","text":"hi"}"#;
        let e: JournalEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.role, "user");
        assert!(e.conv_title.is_empty());
        assert_eq!(e.text, "hi");
    }
}
