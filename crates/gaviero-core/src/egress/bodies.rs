//! Post-turn scan of Claude's raw API request bodies.
//!
//! Claude Code (`OTEL_LOG_RAW_API_BODIES=file:<dir>`) writes
//! `<dir>/<uuid>.request.json`, `<dir>/<request_id>.response.json` and one
//! `<dir>/index.jsonl` line per response (`session_id`, `model`,
//! `request_id`, `request_file`, `response_file`, …). This is the most
//! direct record of what left the machine. Raw bodies hold whatever the
//! agent read, so they are never kept: each request is scanned with
//! [`crate::secret_scan`], one row (size, SHA-256, findings with redacted
//! excerpts) is appended to `claude-requests.ndjson`, and both body files
//! are deleted.
//!
//! Every request repeats the conversation so far, so a secret the agent saw
//! once would be reported on every later request; findings are deduplicated
//! per session by the hash of the matched bytes (`seen`).

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Index files above this size are truncated once fully consumed.
const INDEX_TRUNCATE_BYTES: u64 = 1024 * 1024;
const OFFSET_FILE: &str = ".scan-offset";

/// One secret-looking value found in a request body.
#[derive(Debug, Clone, Serialize)]
pub struct BodyFinding {
    pub rule: &'static str,
    /// Context around the value, with the value itself replaced.
    pub excerpt: String,
}

/// One scanned request.
#[derive(Debug, Clone, Serialize)]
pub struct BodyScanRow {
    pub ts: String,
    pub session_id: String,
    pub model: String,
    pub request_id: String,
    pub bytes: u64,
    pub sha256: String,
    pub findings: Vec<BodyFinding>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(error = %e, "egress bodies: could not delete {}", path.display());
    }
}

/// Scan every request indexed since the last call, log one row per request
/// to `log_path`, delete the raw bodies, and return the rows that carry new
/// findings.
pub fn scan_claude_bodies(dir: &Path, log_path: &Path, seen: &mut HashSet<String>) -> Vec<BodyScanRow> {
    let index_path = dir.join("index.jsonl");
    let Ok(index) = std::fs::read(&index_path) else {
        return Vec::new();
    };
    let offset_path = dir.join(OFFSET_FILE);
    let offset = std::fs::read_to_string(&offset_path)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(index.len());

    // Only complete lines: a line still being written is picked up next time.
    let tail = &index[offset..];
    let consumed = tail.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut rows: Vec<BodyScanRow> = Vec::new();
    let mut reported: Vec<BodyScanRow> = Vec::new();
    for line in tail[..consumed].split(|b| *b == b'\n') {
        let Ok(entry) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let field = |k: &str| entry.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let request_file = field("request_file");
        let response_file = field("response_file");
        if request_file.is_empty() {
            continue;
        }
        let request_path = dir.join(&request_file);
        let Ok(body) = std::fs::read(&request_path) else {
            continue;
        };
        let session_id = field("session_id");
        let text = String::from_utf8_lossy(&body);
        let mut findings: Vec<BodyFinding> = Vec::new();
        for hit in crate::secret_scan::find_secrets(&text) {
            let matched = &text.as_bytes()[hit.start..hit.end];
            let key = format!("{session_id}:{}:{}", hit.rule, hex(&Sha256::digest(matched)));
            if seen.insert(key) {
                findings.push(BodyFinding {
                    rule: hit.rule,
                    excerpt: crate::secret_scan::excerpt(&text, &hit),
                });
            }
        }
        let row = BodyScanRow {
            ts: chrono::Utc::now().to_rfc3339(),
            session_id,
            model: field("model"),
            request_id: field("request_id"),
            bytes: body.len() as u64,
            sha256: hex(&Sha256::digest(&body)),
            findings,
        };
        remove_quietly(&request_path);
        if !response_file.is_empty() {
            remove_quietly(&dir.join(&response_file));
        }
        if !row.findings.is_empty() {
            reported.push(row.clone());
        }
        rows.push(row);
    }

    if !rows.is_empty() {
        let appended = (|| -> std::io::Result<()> {
            if let Some(parent) = log_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?;
            for row in &rows {
                writeln!(f, "{}", serde_json::to_string(row).unwrap_or_default())?;
            }
            Ok(())
        })();
        if let Err(e) = appended {
            tracing::warn!(error = %e, "egress bodies: could not append to {}", log_path.display());
        }
    }

    let new_offset = offset + consumed;
    if new_offset as u64 == index.len() as u64 && new_offset as u64 > INDEX_TRUNCATE_BYTES {
        let _ = std::fs::write(&index_path, b"");
        let _ = std::fs::write(&offset_path, "0");
    } else {
        let _ = std::fs::write(&offset_path, new_offset.to_string());
    }
    reported
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_line(session: &str, request_id: &str, req: &str, resp: &str) -> String {
        format!(
            "{{\"timestamp\":\"t\",\"session_id\":\"{session}\",\"model\":\"claude-haiku\",\
             \"request_id\":\"{request_id}\",\"request_file\":\"{req}\",\"response_file\":\"{resp}\"}}\n"
        )
    }

    #[test]
    fn scan_claude_bodies_reports_once_logs_all_and_deletes_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let bodies = dir.path().join("claude-bodies");
        std::fs::create_dir_all(&bodies).unwrap();
        let secret = "AKIAABCDEFGHIJKLMNOP"; // pragma: allowlist secret
        std::fs::write(bodies.join("a.request.json"), format!("{{\"m\":\"key {secret}\"}}")).unwrap();
        std::fs::write(bodies.join("r1.response.json"), "{}").unwrap();
        std::fs::write(
            bodies.join("b.request.json"),
            format!("{{\"m\":\"key {secret} again\"}}"),
        )
        .unwrap();
        std::fs::write(bodies.join("r2.response.json"), "{}").unwrap();
        std::fs::write(
            bodies.join("index.jsonl"),
            index_line("s1", "r1", "a.request.json", "r1.response.json")
                + &index_line("s1", "r2", "b.request.json", "r2.response.json"),
        )
        .unwrap();

        let log = dir.path().join("claude-requests.ndjson");
        let mut seen = HashSet::new();
        let rows = scan_claude_bodies(&bodies, &log, &mut seen);
        assert_eq!(rows.len(), 1, "the second request repeats the same secret");
        assert_eq!(rows[0].findings.len(), 1);
        assert_eq!(rows[0].findings[0].rule, "aws-access-key-id");
        assert!(!rows[0].findings[0].excerpt.contains(secret));

        for f in ["a.request.json", "b.request.json", "r1.response.json", "r2.response.json"] {
            assert!(!bodies.join(f).exists(), "{f} not deleted");
        }
        let logged = std::fs::read_to_string(&log).unwrap();
        assert_eq!(logged.lines().count(), 2);
        assert!(!logged.contains(secret));
        assert!(bodies.join(OFFSET_FILE).is_file());

        assert!(scan_claude_bodies(&bodies, &log, &mut seen).is_empty());
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
    }

    #[test]
    fn scan_claude_bodies_without_index_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut seen = HashSet::new();
        assert!(scan_claude_bodies(dir.path(), &dir.path().join("log"), &mut seen).is_empty());
        assert!(!dir.path().join("log").exists());
    }

    #[test]
    fn scan_claude_bodies_leaves_a_partial_index_line_for_later() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.request.json"), "{}").unwrap();
        std::fs::write(dir.path().join("index.jsonl"), "{\"request_file\":\"x.request.json\"").unwrap();
        let mut seen = HashSet::new();
        assert!(scan_claude_bodies(dir.path(), &dir.path().join("log"), &mut seen).is_empty());
        assert!(dir.path().join("x.request.json").exists(), "partial line consumed");
    }
}
