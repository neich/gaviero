//! The size-rotated NDJSON append core, shared by every NDJSON sink in
//! the tree.
//!
//! Two sinks write append-only NDJSON under `<workspace>/.gaviero/`:
//! the MCP tool-call telemetry sink (`mcp/telemetry_sink.rs`, since the
//! KB-efficiency plan) and the per-turn history log
//! (`history/writer.rs`). Both need exactly the same three behaviours —
//! serialized appends, `create_dir_all` on the parent, and rotation that
//! keeps exactly one prior generation — so the implementation lives here
//! once instead of twice.
//!
//! The append path is on the tool-response critical path
//! (`mcp/observer.rs`: *"implementations MUST be cheap — the tool
//! response waits on this callback"*), so it stays: no `fsync`, no
//! pretty-printing, one `write_all`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// An append-only, size-rotated NDJSON file.
///
/// Concurrent appends are serialized by an internal mutex, so lines from
/// different tasks never interleave. The mutex guards `()` — the file
/// itself is the shared resource.
pub struct NdjsonAppender {
    path: PathBuf,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl NdjsonAppender {
    /// Appender at an explicit path. `max_bytes` is clamped to ≥ 1 so a
    /// misconfigured cap can never make the sink silently stop writing.
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes: max_bytes.max(1),
            lock: Mutex::new(()),
        }
    }

    /// The active NDJSON path (never the rotated generation).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rotation threshold in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Append one already-serialized line. A trailing newline is added;
    /// the caller must not include one.
    ///
    /// Best-effort at the call site: every caller swallows the error so a
    /// telemetry/history I/O failure can never fail the underlying tool
    /// call or turn.
    pub fn append_line(&self, line: &str) -> std::io::Result<()> {
        let mut buf = String::with_capacity(line.len() + 1);
        buf.push_str(line);
        buf.push('\n');

        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.rotate_if_needed(buf.len() as u64)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(buf.as_bytes())
    }

    /// Serialize `record` and append it as one line.
    pub fn append_json<T: serde::Serialize>(&self, record: &T) -> std::io::Result<()> {
        let line = serde_json::to_string(record)?;
        self.append_line(&line)
    }

    /// Rotate when the current file plus the incoming line would exceed
    /// `max_bytes`. Keeps exactly one prior generation (`<name>.1`).
    fn rotate_if_needed(&self, incoming: u64) -> std::io::Result<()> {
        let cur = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if cur > 0 && cur.saturating_add(incoming) > self.max_bytes {
            let rotated = rotated_path(&self.path);
            let _ = std::fs::remove_file(&rotated);
            std::fs::rename(&self.path, &rotated)?;
        }
        Ok(())
    }
}

/// `<name>.1` — the single retained prior generation.
pub fn rotated_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".1");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_creates_parents_and_writes_one_line_per_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gaviero").join("x.ndjson");
        let appender = NdjsonAppender::new(path.clone(), 1024);

        appender.append_json(&serde_json::json!({"a": 1})).unwrap();
        appender.append_line(r#"{"b":2}"#).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "{\"a\":1}\n{\"b\":2}\n");
        assert!(!rotated_path(&path).exists());
    }

    #[test]
    fn rotation_keeps_exactly_one_prior_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.ndjson");
        // Cap below one serialized line: every write after the first rotates,
        // so the steady state is one record in `.1` and one in the current file.
        let appender = NdjsonAppender::new(path.clone(), 8);

        for _ in 0..6 {
            appender.append_line(r#"{"tool":"node_doc"}"#).unwrap();
        }

        assert!(path.exists());
        assert!(rotated_path(&path).exists());
        assert_eq!(body_lines(&path), 1);
        assert_eq!(body_lines(&rotated_path(&path)), 1);
    }

    fn body_lines(path: &Path) -> usize {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    }
}
