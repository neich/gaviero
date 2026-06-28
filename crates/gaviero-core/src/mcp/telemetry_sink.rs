//! MCP tool-call telemetry sink (KB-efficiency plan, Phase 1).
//!
//! A host-side [`McpToolCallObserver`] that appends each tool call as one
//! JSON line to `<workspace>/.gaviero/mcp_calls.ndjson`. The file is
//! size-rotated: once it would exceed `max_bytes` (default 10 MB) the
//! current file is moved aside to `<name>.1` (replacing any previous
//! `.1`) and a fresh file is started, so on-disk telemetry never grows
//! without bound. Exactly one prior generation is retained.
//!
//! **Read-only-boundary invariant.** This sink is deliberately decoupled
//! from the memory writer task: MCP observability must never flow through
//! SQLite-via-writer, which would couple the read-only MCP boundary to
//! the memory write path. It is a plain host-side file append behind the
//! existing observer seam. Content is whatever the read-only tools
//! returned — already post-redaction at the store level — kept local and
//! bounded.
//!
//! The `gaviero-cli --mcp-stats` reporter reads this file back via
//! [`compute_stats`].

use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::observer::{McpCallLogEntry, McpToolCallObserver};

/// Default rotation threshold: 10 MB.
pub const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// NDJSON filename under `<workspace>/.gaviero/`.
pub const TELEMETRY_FILENAME: &str = "mcp_calls.ndjson";

/// Default sink path for a workspace root: `<root>/.gaviero/mcp_calls.ndjson`.
pub fn default_telemetry_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".gaviero").join(TELEMETRY_FILENAME)
}

/// One serialized telemetry record. Mirrors [`McpCallLogEntry`] but with
/// a serializable microsecond duration and a capture timestamp, so the
/// file is self-describing for the `--mcp-stats` reader and ad-hoc
/// inspection. Microseconds (not millis) preserve sub-millisecond
/// resolution — `memory_search`/`node_doc` calls are frequently <1 ms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallRecord {
    /// RFC3339 capture time (UTC).
    pub ts: String,
    pub tool_name: String,
    pub duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// True when the tool returned no result rows. Precomputed at capture
    /// so the reporter need not re-derive output shape per tool.
    pub empty_result: bool,
    /// PUSH→PULL Phase 0: first tool call on the MCP connection (session)
    /// that produced it. Lets the offline reader count sessions that issued
    /// at least one read-only tool call, per tier.
    #[serde(default)]
    pub first_tool_call_initiated: bool,
    /// Session id / turn for per-tier analysis. Currently absent (not yet
    /// wired); persisted only when present so a later phase can populate them
    /// without breaking the append-only format (old records read back as
    /// `null` via `#[serde(default)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    /// Raw tool input JSON.
    pub input: serde_json::Value,
    /// Raw tool output JSON.
    pub output: serde_json::Value,
}

impl McpCallRecord {
    fn from_entry(entry: &McpCallLogEntry) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            tool_name: entry.tool_name.clone(),
            duration_us: entry.duration.as_micros() as u64,
            error: entry.error.clone(),
            empty_result: output_is_empty(&entry.output),
            first_tool_call_initiated: entry.first_tool_call_initiated,
            session_id: entry.session_id.clone(),
            turn: entry.turn,
            input: entry.input.clone(),
            output: entry.output.clone(),
        }
    }
}

/// Heuristic "did this call return anything useful?" check over the raw
/// output JSON. Inspects the first known result-bearing array field
/// across the read-only tools (`results` for `memory_search`, `nodes`
/// for `blast_radius`, `signatures`/`impls` for `node_doc`/symbol
/// tools). A non-object output, or one with no recognised array, counts
/// as non-empty.
fn output_is_empty(output: &serde_json::Value) -> bool {
    let Some(obj) = output.as_object() else {
        return false;
    };
    for key in ["results", "nodes", "signatures", "impls"] {
        if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
            return arr.is_empty();
        }
    }
    false
}

/// Size-rotated NDJSON telemetry sink. Cheap synchronous append per
/// tool call; writes from concurrent MCP connections are serialized by
/// an internal mutex so lines never interleave.
pub struct NdjsonTelemetrySink {
    path: PathBuf,
    max_bytes: u64,
    /// Serializes concurrent appends. The guarded data is `()` — the
    /// file itself is the shared resource.
    lock: Mutex<()>,
}

impl NdjsonTelemetrySink {
    /// Sink writing to an explicit path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            max_bytes: DEFAULT_MAX_BYTES,
            lock: Mutex::new(()),
        }
    }

    /// Sink at the default location under `<workspace_root>/.gaviero/`.
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self::new(default_telemetry_path(workspace_root))
    }

    /// Override the rotation threshold (bytes). Clamped to ≥ 1.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes.max(1);
        self
    }

    /// The active NDJSON path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record. Best-effort: callers swallow the error so a
    /// telemetry I/O failure never fails the underlying tool call.
    fn append(&self, record: &McpCallRecord) -> std::io::Result<()> {
        let mut line = serde_json::to_string(record).unwrap_or_default();
        line.push('\n');

        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.rotate_if_needed(line.len() as u64)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(line.as_bytes())
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

impl McpToolCallObserver for NdjsonTelemetrySink {
    fn on_tool_call(&self, entry: &McpCallLogEntry) {
        let record = McpCallRecord::from_entry(entry);
        if let Err(e) = self.append(&record) {
            tracing::warn!(
                target: "mcp_telemetry",
                error = %e,
                path = %self.path.display(),
                "failed to append MCP tool-call telemetry"
            );
        }
    }
}

/// `<name>.1` — the single retained prior generation.
fn rotated_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".1");
    PathBuf::from(s)
}

/// Per-tool aggregate intrinsic metrics computed from the NDJSON sink.
/// Intrinsic only (no task-success correlation): call count, p50/p95
/// latency, error rate, and empty-result rate.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStats {
    pub tool_name: String,
    pub calls: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub error_rate: f64,
    pub empty_result_rate: f64,
}

/// Streaming accumulator (kept private — `compute_stats` is the public
/// seam).
struct Acc {
    durations_us: Vec<u64>,
    errors: usize,
    empties: usize,
}

/// Read the NDJSON telemetry file (plus its one rotated generation) and
/// aggregate per-tool intrinsic metrics. Lines are streamed via a
/// `BufReader` so memory stays bounded regardless of file size, and
/// unparseable lines are skipped (the file may carry a partially-written
/// tail or a mid-rotation remnant). Returns tools sorted by call count
/// descending, then name. A missing file yields an empty report.
pub fn compute_stats(path: &Path) -> std::io::Result<Vec<ToolStats>> {
    let mut by_tool: BTreeMap<String, Acc> = BTreeMap::new();
    // Older generation first so the (unused-for-stats) chronological
    // order is roughly preserved for any future time-windowed reader.
    fold_file(&rotated_path(path), &mut by_tool)?;
    fold_file(path, &mut by_tool)?;

    let mut out: Vec<ToolStats> = by_tool
        .into_iter()
        .map(|(tool_name, mut acc)| {
            acc.durations_us.sort_unstable();
            let calls = acc.durations_us.len();
            let denom = calls.max(1) as f64;
            ToolStats {
                tool_name,
                calls,
                p50_ms: percentile_us(&acc.durations_us, 50) / 1000.0,
                p95_ms: percentile_us(&acc.durations_us, 95) / 1000.0,
                error_rate: acc.errors as f64 / denom,
                empty_result_rate: acc.empties as f64 / denom,
            }
        })
        .collect();
    out.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.tool_name.cmp(&b.tool_name)));
    Ok(out)
}

/// Fold one NDJSON file's records into the accumulator. A missing file
/// is not an error (the rotated generation may not exist yet).
fn fold_file(path: &Path, by_tool: &mut BTreeMap<String, Acc>) -> std::io::Result<()> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<McpCallRecord>(trimmed) else {
            continue;
        };
        let acc = by_tool.entry(rec.tool_name).or_insert_with(|| Acc {
            durations_us: Vec::new(),
            errors: 0,
            empties: 0,
        });
        acc.durations_us.push(rec.duration_us);
        if rec.error.is_some() {
            acc.errors += 1;
        }
        if rec.empty_result {
            acc.empties += 1;
        }
    }
    Ok(())
}

/// Nearest-rank percentile (1-indexed `ceil(p/100 * n)`) over a
/// pre-sorted slice of microsecond durations. Returns 0.0 for an empty
/// slice.
fn percentile_us(sorted: &[u64], p: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p as f64 / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    let idx = rank.min(sorted.len()) - 1;
    sorted[idx] as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(tool: &str, dur: Duration, error: Option<&str>, output: serde_json::Value) -> McpCallLogEntry {
        McpCallLogEntry {
            tool_name: tool.to_string(),
            input: serde_json::json!({"query": "q"}),
            output,
            duration: dur,
            error: error.map(str::to_string),
            first_tool_call_initiated: false,
            session_id: None,
            turn: None,
        }
    }

    #[test]
    fn output_is_empty_detects_known_arrays() {
        assert!(output_is_empty(&serde_json::json!({"results": []})));
        assert!(!output_is_empty(&serde_json::json!({"results": [1]})));
        assert!(output_is_empty(&serde_json::json!({"nodes": []})));
        assert!(!output_is_empty(&serde_json::json!({"nodes": [{}]})));
        // No recognised array → treated as non-empty.
        assert!(!output_is_empty(&serde_json::json!({"path": "src/lib.rs"})));
        assert!(!output_is_empty(&serde_json::json!("scalar")));
    }

    #[test]
    fn append_then_compute_stats_aggregates_per_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gaviero").join(TELEMETRY_FILENAME);
        let sink = NdjsonTelemetrySink::new(path.clone());

        sink.on_tool_call(&entry(
            "memory_search",
            Duration::from_micros(1000),
            None,
            serde_json::json!({"results": [{"id": 1}]}),
        ));
        sink.on_tool_call(&entry(
            "memory_search",
            Duration::from_micros(3000),
            None,
            serde_json::json!({"results": []}),
        ));
        sink.on_tool_call(&entry(
            "blast_radius",
            Duration::from_micros(5000),
            Some("boom"),
            serde_json::json!({"nodes": []}),
        ));

        let stats = compute_stats(&path).unwrap();
        assert_eq!(stats.len(), 2);
        // Sorted by call count desc → memory_search (2) first.
        assert_eq!(stats[0].tool_name, "memory_search");
        assert_eq!(stats[0].calls, 2);
        assert_eq!(stats[0].error_rate, 0.0);
        assert_eq!(stats[0].empty_result_rate, 0.5);
        // p50 over [1000us, 3000us] nearest-rank rank-1 → 1000us = 1.0ms.
        assert!((stats[0].p50_ms - 1.0).abs() < 1e-9);

        assert_eq!(stats[1].tool_name, "blast_radius");
        assert_eq!(stats[1].calls, 1);
        assert_eq!(stats[1].error_rate, 1.0);
        assert_eq!(stats[1].empty_result_rate, 1.0);
    }

    #[test]
    fn rotation_keeps_one_prior_generation_and_stats_span_both() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TELEMETRY_FILENAME);
        // Cap below one serialized line forces a rotation on every write
        // after the first, so the steady state is exactly one record in
        // the rotated `.1` and one in the current file.
        let sink = NdjsonTelemetrySink::new(path.clone()).with_max_bytes(120);

        for _ in 0..6 {
            sink.on_tool_call(&entry(
                "node_doc",
                Duration::from_micros(200),
                None,
                serde_json::json!({"signatures": ["fn f()"]}),
            ));
        }
        // Current file plus exactly one rotated generation exist.
        assert!(path.exists());
        assert!(rotated_path(&path).exists());

        // Stats fold both surviving generations. By design only one prior
        // generation is retained, so older records are discarded: 1 in
        // `.1` + 1 current = 2, not all 6.
        let stats = compute_stats(&path).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].tool_name, "node_doc");
        assert_eq!(stats[0].calls, 2);
    }

    #[test]
    fn compute_stats_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let stats = compute_stats(&dir.path().join("absent.ndjson")).unwrap();
        assert!(stats.is_empty());
    }

    #[test]
    fn compute_stats_skips_unparseable_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TELEMETRY_FILENAME);
        std::fs::write(
            &path,
            "not json\n{\"ts\":\"t\",\"tool_name\":\"memory_search\",\"duration_us\":500,\"empty_result\":false,\"input\":{},\"output\":{}}\n\n",
        )
        .unwrap();
        let stats = compute_stats(&path).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].calls, 1);
    }
}
