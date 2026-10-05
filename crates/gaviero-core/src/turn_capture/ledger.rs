//! Writes the host performs itself (editor saves, review reverts).
//!
//! A turn's end scan sees every change in the tree, including ones the user
//! made in the editor while the agent ran and reverts applied by another
//! conversation's review. The host records those writes here, and a change
//! whose post-turn content matches a host write made during the turn is not
//! attributed to the agent.

use std::path::Path;
use std::sync::Mutex;

use super::now_ns;
use super::store::hash_bytes;

/// How long an entry stays relevant. Turns longer than this lose attribution
/// for older host writes, which only means the change shows up in review.
const LEDGER_TTL_NS: i64 = 6 * 60 * 60 * 1_000_000_000;

#[derive(Debug, Clone)]
struct Entry {
    key: String,
    /// Content hash written; `None` = the host deleted the file.
    sha256: Option<String>,
    at_ns: i64,
}

#[derive(Debug, Default)]
pub struct HostWriteLedger {
    entries: Mutex<Vec<Entry>>,
}

impl HostWriteLedger {
    /// Record that the host wrote `content` to `path` (`None` = removed it).
    pub fn record(&self, path: &Path, content: Option<&[u8]>) {
        let entry = Entry {
            key: path_key(path),
            sha256: content.map(hash_bytes),
            at_ns: now_ns(),
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let cutoff = entry.at_ns - LEDGER_TTL_NS;
        entries.retain(|e| e.at_ns >= cutoff);
        entries.push(entry);
    }

    /// Did the host itself leave `path` holding `sha256` at or after `since_ns`?
    pub fn attributes(&self, path: &Path, sha256: Option<&str>, since_ns: i64) -> bool {
        let key = path_key(path);
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        // The *latest* host write to the path decides: an older matching write
        // followed by a different one does not explain the current bytes.
        entries
            .iter()
            .rev()
            .find(|e| e.key == key && e.at_ns >= since_ns)
            .is_some_and(|e| e.sha256.as_deref() == sha256)
    }
}

/// Normalized comparison key: `/` separators, case-folded on Windows.
pub(crate) fn path_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) { s.to_lowercase() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_write_decides() {
        let ledger = HostWriteLedger::default();
        let p = Path::new("/ws/a.txt");
        let t0 = now_ns() - 1;
        ledger.record(p, Some(b"one"));
        assert!(ledger.attributes(p, Some(&hash_bytes(b"one")), t0));
        ledger.record(p, Some(b"two"));
        assert!(!ledger.attributes(p, Some(&hash_bytes(b"one")), t0));
        assert!(ledger.attributes(p, Some(&hash_bytes(b"two")), t0));
        ledger.record(p, None);
        assert!(ledger.attributes(p, None, t0));
        assert!(!ledger.attributes(Path::new("/ws/b.txt"), None, t0));
    }
}
