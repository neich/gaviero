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
//!
//! The body is walked as Messages-API JSON, so each hit knows where it came
//! from: a `tool_result` is tied to its `tool_use` (`Read` → `file_path`),
//! and a `grep`-style `path:line` names its file. Each finding is then
//! classified — `allowlisted` (an allowlist marker such as
//! `// pragma: allowlist secret` on the same line), `test-path` (test
//! directory / file name, or inside a Rust `#[cfg(test)]` module),
//! `dummy-pattern` (sequential, repetitive or placeholder value) or
//! `unclassified`. Classification only downgrades: every finding is logged.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    /// `allowlisted` | `test-path` | `dummy-pattern` | `unclassified`.
    pub classification: &'static str,
    /// The file the value was read from, when the request shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl BodyFinding {
    /// True for findings worth a warning (nothing marks them as a fixture).
    pub fn is_unclassified(&self) -> bool {
        self.classification == "unclassified"
    }
}

/// One text segment of a request body and the file it came from, if known.
struct Segment {
    text: String,
    source: Option<PathBuf>,
}

fn push_text(out: &mut Vec<Segment>, value: &serde_json::Value, source: Option<&PathBuf>) {
    match value {
        serde_json::Value::String(s) => out.push(Segment {
            text: s.clone(),
            source: source.cloned(),
        }),
        serde_json::Value::Array(items) => {
            for item in items {
                match item.get("text") {
                    Some(text) => push_text(out, text, source),
                    None => push_text(out, item, source),
                }
            }
        }
        serde_json::Value::Object(_) => out.push(Segment {
            text: value.to_string(),
            source: source.cloned(),
        }),
        _ => {}
    }
}

/// Split a Messages-API request into text segments, tying each
/// `tool_result` to the file its `Read` call named. Unparseable bodies are
/// one segment with no source.
fn segments(body: &str, root: &Path) -> Vec<Segment> {
    let whole = || {
        vec![Segment {
            text: body.to_string(),
            source: None,
        }]
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return whole();
    };
    // Not a Messages-API request: scan it as-is rather than miss anything.
    if doc.get("messages").is_none() && doc.get("system").is_none() {
        return whole();
    }
    let mut out = Vec::new();
    if let Some(system) = doc.get("system") {
        push_text(&mut out, system, None);
    }
    let empty = Vec::new();
    let messages = doc
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);
    let mut read_paths: HashMap<String, PathBuf> = HashMap::new();
    for message in messages {
        let Some(blocks) = message.get("content").and_then(|c| c.as_array()) else {
            if let Some(content) = message.get("content") {
                push_text(&mut out, content, None);
            }
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") => {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let path = block.pointer("/input/file_path").and_then(|v| v.as_str());
                    if let Some(path) = path
                        && block.get("name").and_then(|v| v.as_str()) == Some("Read")
                    {
                        let path = PathBuf::from(path);
                        let path = if path.is_absolute() { path } else { root.join(path) };
                        read_paths.insert(id.to_string(), path);
                    }
                    if let Some(input) = block.get("input") {
                        push_text(&mut out, &serde_json::Value::String(input.to_string()), None);
                    }
                }
                Some("tool_result") => {
                    let id = block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(content) = block.get("content") {
                        push_text(&mut out, content, read_paths.get(id));
                    }
                }
                _ => {
                    if let Some(text) = block.get("text") {
                        push_text(&mut out, text, None);
                    }
                }
            }
        }
    }
    out
}

/// The file a `grep`-style `path:rest` line names, when it exists.
fn grep_line_source(line: &str, root: &Path) -> Option<PathBuf> {
    let path = crate::agent_guard::hook::grep_line_path(line.trim_start())?;
    let path = PathBuf::from(path);
    let path = if path.is_absolute() { path } else { root.join(path) };
    path.is_file().then_some(path)
}

/// Is every occurrence of `value` in the Rust file `path` inside its
/// `#[cfg(test)]` part? (Unreadable, non-Rust or value-less files: no.)
fn in_rust_test_module(path: &Path, value: &str) -> bool {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return false;
    }
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_BYTES) {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(test_start) = content.find("#[cfg(test)]") else {
        return false;
    };
    let mut seen_any = false;
    for (at, _) in content.match_indices(value) {
        seen_any = true;
        if at < test_start {
            return false;
        }
    }
    seen_any
}

/// Classify one hit. Order: allowlist marker, test path, dummy value.
fn classify(line: &str, value: &str, rule: &str, source: Option<&Path>) -> &'static str {
    if crate::secret_scan::has_allowlist_marker(line) {
        return "allowlisted";
    }
    if let Some(path) = source
        && (crate::secret_scan::is_test_path(path) || in_rust_test_module(path, value))
    {
        return "test-path";
    }
    if crate::secret_scan::is_dummy_value(rule, value) {
        return "dummy-pattern";
    }
    "unclassified"
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
///
/// `root` is the workspace root, used to resolve relative source paths.
pub fn scan_claude_bodies(
    root: &Path,
    dir: &Path,
    log_path: &Path,
    seen: &mut HashSet<String>,
) -> Vec<BodyScanRow> {
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
        for segment in segments(&text, root) {
            for hit in crate::secret_scan::find_secrets(&segment.text) {
                let value = &segment.text[hit.start..hit.end];
                let key = format!(
                    "{session_id}:{}:{}",
                    hit.rule,
                    hex(&Sha256::digest(value.as_bytes()))
                );
                if !seen.insert(key) {
                    continue;
                }
                let line = crate::secret_scan::line_around(&segment.text, &hit);
                let source = segment
                    .source
                    .clone()
                    .or_else(|| grep_line_source(line, root));
                findings.push(BodyFinding {
                    rule: hit.rule,
                    excerpt: crate::secret_scan::excerpt(&segment.text, &hit),
                    classification: classify(line, value, hit.rule, source.as_deref()),
                    source: source.map(|p| p.to_string_lossy().into_owned()),
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
        let rows = scan_claude_bodies(dir.path(), &bodies, &log, &mut seen);
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

        assert!(scan_claude_bodies(dir.path(), &bodies, &log, &mut seen).is_empty());
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
    }

    /// Scan one request body written into a fresh `claude-bodies` dir.
    fn scan_one(root: &Path, body: &serde_json::Value) -> Vec<BodyFinding> {
        let bodies = root.join("claude-bodies");
        std::fs::create_dir_all(&bodies).unwrap();
        std::fs::write(bodies.join("q.request.json"), body.to_string()).unwrap();
        std::fs::write(
            bodies.join("index.jsonl"),
            index_line("s1", "r1", "q.request.json", "r1.response.json"),
        )
        .unwrap();
        let mut seen = HashSet::new();
        scan_claude_bodies(root, &bodies, &root.join("log.ndjson"), &mut seen)
            .into_iter()
            .flat_map(|r| r.findings)
            .collect()
    }

    /// A request in which the agent `Read` `path` and got `content` back.
    fn read_request(path: &str, content: &str) -> serde_json::Value {
        serde_json::json!({
            "model": "claude-haiku",
            "messages": [
                {"role": "user", "content": "look"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": path}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": content}
                ]}
            ]
        })
    }

    const REAL_LOOKING: &str = "AKIAQ7XZ4M2KD9PL3VWN"; // pragma: allowlist secret

    #[test]
    fn classify_allowlisted_line_from_a_read() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!("1\tlet k = \"{REAL_LOOKING}\"; // pragma: allowlist secret\n");
        let findings = scan_one(dir.path(), &read_request("src/lib.rs", &content));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].classification, "allowlisted");
        assert!(findings[0].source.as_deref().unwrap().ends_with("lib.rs"));
    }

    #[test]
    fn classify_test_path_from_the_read_call() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!("1\tconst K: &str = \"{REAL_LOOKING}\";\n");
        let findings = scan_one(dir.path(), &read_request("tests/auth.rs", &content));
        assert_eq!(findings[0].classification, "test-path");
    }

    #[test]
    fn classify_rust_inline_test_module_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            format!("pub fn f() {{}}\n#[cfg(test)]\nmod tests {{\n    const K: &str = \"{REAL_LOOKING}\";\n}}\n"),
        )
        .unwrap();
        let content = format!("4\t    const K: &str = \"{REAL_LOOKING}\";\n");
        let path = src.join("lib.rs");
        let findings = scan_one(dir.path(), &read_request(&path.to_string_lossy(), &content));
        assert_eq!(findings[0].classification, "test-path");

        // The same value used outside the test module is not a fixture.
        std::fs::write(
            src.join("lib.rs"),
            format!("pub const K: &str = \"{REAL_LOOKING}\";\n#[cfg(test)]\nmod tests {{}}\n"),
        )
        .unwrap();
        let other = tempfile::tempdir().unwrap();
        let findings = scan_one(other.path(), &read_request(&path.to_string_lossy(), &content));
        assert_eq!(findings[0].classification, "unclassified");
    }

    #[test]
    fn classify_grep_output_line_by_its_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("fixtures")).unwrap();
        std::fs::write(dir.path().join("fixtures/creds.txt"), REAL_LOOKING).unwrap();
        let body = serde_json::json!({
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "b1", "name": "Bash", "input": {"command": "grep -r AKIA ."}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "b1",
                     "content": [{"type": "text", "text": format!("fixtures/creds.txt:{REAL_LOOKING}")}]}
                ]}
            ]
        });
        let findings = scan_one(dir.path(), &body);
        assert_eq!(findings[0].classification, "test-path");
    }

    #[test]
    fn classify_dummy_and_unclassified_values() {
        let dir = tempfile::tempdir().unwrap();
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "dummy AKIAABCDEFGHIJKLMNOP"}, // pragma: allowlist secret
                {"role": "user", "content": format!("real-looking {REAL_LOOKING}")}
            ]
        });
        let findings = scan_one(dir.path(), &body);
        let by_class: Vec<&str> = findings.iter().map(|f| f.classification).collect();
        assert_eq!(by_class, vec!["dummy-pattern", "unclassified"]);
        assert!(findings[1].is_unclassified());
        assert!(findings[1].source.is_none());
    }

    #[test]
    fn unparseable_body_is_scanned_as_one_segment() {
        let dir = tempfile::tempdir().unwrap();
        let bodies = dir.path().join("claude-bodies");
        std::fs::create_dir_all(&bodies).unwrap();
        std::fs::write(bodies.join("q.request.json"), format!("not json {REAL_LOOKING}")).unwrap();
        std::fs::write(
            bodies.join("index.jsonl"),
            index_line("s1", "r1", "q.request.json", "r1.response.json"),
        )
        .unwrap();
        let mut seen = HashSet::new();
        let rows = scan_claude_bodies(dir.path(), &bodies, &dir.path().join("log"), &mut seen);
        assert_eq!(rows[0].findings[0].classification, "unclassified");
    }

    #[test]
    fn scan_claude_bodies_without_index_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut seen = HashSet::new();
        assert!(scan_claude_bodies(dir.path(), dir.path(), &dir.path().join("log"), &mut seen).is_empty());
        assert!(!dir.path().join("log").exists());
    }

    #[test]
    fn scan_claude_bodies_leaves_a_partial_index_line_for_later() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.request.json"), "{}").unwrap();
        std::fs::write(dir.path().join("index.jsonl"), "{\"request_file\":\"x.request.json\"").unwrap();
        let mut seen = HashSet::new();
        assert!(scan_claude_bodies(dir.path(), dir.path(), &dir.path().join("log"), &mut seen).is_empty());
        assert!(dir.path().join("x.request.json").exists(), "partial line consumed");
    }
}
