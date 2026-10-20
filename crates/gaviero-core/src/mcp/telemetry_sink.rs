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
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::observer::{McpCallLogEntry, McpToolCallObserver};
use crate::util::ndjson::{NdjsonAppender, rotated_path};

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
    #[serde(default)]
    pub input: serde_json::Value,
    /// Raw tool output JSON. `null` (or absent, in older lines) on a
    /// failed call or when the output was not captured; such records still
    /// count as calls.
    #[serde(default)]
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
/// across the read-only tools (`results` for `memory_search` /
/// `symbol_search`, `nodes` for `blast_radius`, `symbols` for `node_doc`,
/// `entries` for `repo_outline`, `refs` for `symbol_refs`). `signatures` and `impls` stay last so
/// records written before `node_doc` dropped its legacy `signatures`
/// list keep their verdict. A non-object output, or one with no
/// recognised array, counts as non-empty.
///
/// Public because the history recorder labels an MCP record with the same
/// verdict (`empty_result`) rather than re-deriving it.
pub fn output_is_empty(output: &serde_json::Value) -> bool {
    let Some(obj) = output.as_object() else {
        return false;
    };
    for key in ["results", "nodes", "symbols", "entries", "refs", "signatures", "impls"] {
        if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
            return arr.is_empty();
        }
    }
    false
}

/// Size-rotated NDJSON telemetry sink. Cheap synchronous append per
/// tool call; writes from concurrent MCP connections are serialized by
/// the appender's internal mutex so lines never interleave.
pub struct NdjsonTelemetrySink {
    appender: NdjsonAppender,
}

impl NdjsonTelemetrySink {
    /// Sink writing to an explicit path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            appender: NdjsonAppender::new(path, DEFAULT_MAX_BYTES),
        }
    }

    /// Sink at the default location under `<workspace_root>/.gaviero/`.
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self::new(default_telemetry_path(workspace_root))
    }

    /// Override the rotation threshold (bytes). Clamped to ≥ 1.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.appender = NdjsonAppender::new(self.appender.path().to_path_buf(), max_bytes);
        self
    }

    /// The active NDJSON path.
    pub fn path(&self) -> &Path {
        self.appender.path()
    }
}

impl McpToolCallObserver for NdjsonTelemetrySink {
    fn on_tool_call(&self, entry: &McpCallLogEntry) {
        let record = McpCallRecord::from_entry(entry);
        if let Err(e) = self.appender.append_json(&record) {
            tracing::warn!(
                target: "mcp_telemetry",
                error = %e,
                path = %self.appender.path().display(),
                "failed to append MCP tool-call telemetry"
            );
        }
    }
}

/// Per-tool aggregate intrinsic metrics computed from the NDJSON sink.
/// Intrinsic only (no task-success correlation): call count, p50/p95
/// latency, error rate, empty-result rate, and estimated output size.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStats {
    pub tool_name: String,
    pub calls: usize,
    /// Latency percentiles, nearest rank: sorted index `ceil(q·n) − 1`.
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub error_rate: f64,
    /// Successful calls whose output was empty, over all calls.
    pub empty_result_rate: f64,
    /// Successful calls whose output was measured.
    pub output_samples: usize,
    /// Successful calls with no usable output (`null` or absent in the
    /// record). Unknown size, not zero tokens.
    pub output_missing: usize,
    /// Estimated output tokens of successful calls (see [`super::payload`]),
    /// at sorted index `ceil(q·(n − 1))`. `None` without samples.
    pub output_p50_tokens: Option<usize>,
    pub output_p95_tokens: Option<usize>,
}

/// Streaming accumulator (kept private — `compute_stats` is the public
/// seam).
#[derive(Default)]
struct Acc {
    durations_us: Vec<u64>,
    errors: usize,
    empties: usize,
    output_tokens: Vec<usize>,
    output_missing: usize,
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
            acc.output_tokens.sort_unstable();
            let calls = acc.durations_us.len();
            let denom = calls.max(1) as f64;
            ToolStats {
                tool_name,
                calls,
                p50_ms: percentile_us(&acc.durations_us, 50) / 1000.0,
                p95_ms: percentile_us(&acc.durations_us, 95) / 1000.0,
                error_rate: acc.errors as f64 / denom,
                empty_result_rate: acc.empties as f64 / denom,
                output_samples: acc.output_tokens.len(),
                output_missing: acc.output_missing,
                output_p50_tokens: index_percentile(&acc.output_tokens, 0.50),
                output_p95_tokens: index_percentile(&acc.output_tokens, 0.95),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
    });
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
        let acc = by_tool.entry(rec.tool_name).or_default();
        acc.durations_us.push(rec.duration_us);
        if rec.error.is_some() {
            // A failure has no successful payload: it neither counts as
            // empty nor enters the output-size distribution.
            acc.errors += 1;
            continue;
        }
        if rec.empty_result {
            acc.empties += 1;
        }
        match &rec.output {
            serde_json::Value::Null => acc.output_missing += 1,
            output => match super::payload::compact_json_len(output) {
                Some(bytes) => acc
                    .output_tokens
                    .push(super::payload::estimated_tokens(bytes)),
                None => acc.output_missing += 1,
            },
        }
    }
    Ok(())
}

/// Output-size percentile at sorted index `ceil(q·(n − 1))`, the
/// convention of the adoption report's replay (not the nearest rank used
/// for latency). `None` for an empty slice.
fn index_percentile(sorted: &[usize], q: f64) -> Option<usize> {
    if sorted.is_empty() {
        return None;
    }
    let idx = (q * (sorted.len() - 1) as f64).ceil() as usize;
    Some(sorted[idx.min(sorted.len() - 1)])
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

    fn entry(
        tool: &str,
        dur: Duration,
        error: Option<&str>,
        output: serde_json::Value,
    ) -> McpCallLogEntry {
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
        assert!(output_is_empty(&serde_json::json!({"refs": []})));
        assert!(!output_is_empty(&serde_json::json!({"refs": [{}]})));
        // No recognised array → treated as non-empty.
        assert!(!output_is_empty(&serde_json::json!({"path": "src/lib.rs"})));
        assert!(!output_is_empty(&serde_json::json!("scalar")));
    }

    #[test]
    fn output_is_empty_reads_modern_node_doc_and_outline_payloads() {
        // node_doc without the legacy `signatures` list.
        assert!(output_is_empty(&serde_json::json!({
            "path": "a.md", "qualified_name": "a.md", "symbols": [], "note": "…"
        })));
        assert!(!output_is_empty(&serde_json::json!({
            "path": "a.rs", "qualified_name": "a.rs",
            "symbols": [{"qualified_name": "a.rs::f", "line": 1}]
        })));
        // repo_outline.
        assert!(output_is_empty(&serde_json::json!({"entries": [], "token_estimate": 9})));
        assert!(!output_is_empty(&serde_json::json!({
            "entries": [{"rendered": "  a.rs (s1.00)"}], "token_estimate": 12
        })));
        // Legacy node_doc records carried both lists; the verdict is unchanged.
        assert!(output_is_empty(&serde_json::json!({"signatures": [], "symbols": []})));
        assert!(!output_is_empty(&serde_json::json!({"signatures": ["f"], "symbols": [{}]})));
        assert!(output_is_empty(&serde_json::json!({"signatures": []})));
        assert!(!output_is_empty(&serde_json::json!(null)));
        assert!(!output_is_empty(&serde_json::json!([])));
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
        // A failed call is not an empty *result*, whatever its output held.
        assert_eq!(stats[1].empty_result_rate, 0.0);
        assert_eq!(stats[1].output_samples, 0);
        assert_eq!(stats[1].output_p50_tokens, None);
    }

    #[test]
    fn index_percentile_matches_the_replay_convention() {
        assert_eq!(index_percentile(&[], 0.5), None);
        assert_eq!(index_percentile(&[7], 0.5), Some(7));
        assert_eq!(index_percentile(&[7], 0.95), Some(7));
        assert_eq!(index_percentile(&[1, 9], 0.5), Some(9), "ceil(0.5) = 1");
        assert_eq!(index_percentile(&[1, 9], 0.95), Some(9));
        let v: Vec<usize> = (0..21).collect();
        assert_eq!(index_percentile(&v, 0.5), Some(10));
        assert_eq!(index_percentile(&v, 0.95), Some(19));
        let v: Vec<usize> = (0..22).collect();
        assert_eq!(index_percentile(&v, 0.95), Some(20), "ceil(19.95)");
    }

    #[test]
    fn output_sizes_ignore_failures_and_missing_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TELEMETRY_FILENAME);
        let sink = NdjsonTelemetrySink::new(path.clone());
        let d = Duration::from_micros(10);
        // Two measured successes: 9 bytes → 3 tokens, 11 bytes ("→") → 3.
        sink.on_tool_call(&entry("t", d, None, serde_json::json!({"a": "b"})));
        sink.on_tool_call(&entry("t", d, None, serde_json::json!({"a": "→"})));
        // A large one so p50 would move if the others were counted as 0.
        let big = serde_json::json!({"results": ["x".repeat(400)]});
        let big_tokens = serde_json::to_string(&big).unwrap().len().div_ceil(4);
        sink.on_tool_call(&entry("t", d, None, big));
        // Failures (null and non-null output) and a captured-but-null success.
        sink.on_tool_call(&entry("t", d, Some("denied"), serde_json::Value::Null));
        sink.on_tool_call(&entry("t", d, Some("boom"), serde_json::json!({"results": []})));
        sink.on_tool_call(&entry("t", d, None, serde_json::Value::Null));

        let s = &compute_stats(&path).unwrap()[0];
        assert_eq!(s.calls, 6);
        assert!((s.error_rate - 2.0 / 6.0).abs() < 1e-9);
        assert_eq!(s.output_samples, 3);
        assert_eq!(s.output_missing, 1);
        assert_eq!(s.output_p50_tokens, Some(3));
        assert_eq!(s.output_p95_tokens, Some(big_tokens));
        // A successful empty array is measurable and counts as empty.
        sink.on_tool_call(&entry("u", d, None, serde_json::json!({"results": []})));
        let u = compute_stats(&path)
            .unwrap()
            .into_iter()
            .find(|s| s.tool_name == "u")
            .unwrap();
        assert_eq!(u.output_samples, 1);
        assert_eq!(u.output_p50_tokens, Some(4), "{{\"results\":[]}} is 14 bytes");
        assert_eq!(u.empty_result_rate, 1.0);
    }

    #[test]
    fn legacy_records_without_output_still_count_across_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TELEMETRY_FILENAME);
        // Rotated generation: an older line with no `input`/`output`, a
        // malformed line, and a partial tail.
        std::fs::write(
            rotated_path(&path),
            "{\"ts\":\"t\",\"tool_name\":\"node_doc\",\"duration_us\":5,\"empty_result\":false}\n\
             {broken\n\
             {\"ts\":\"t\",\"tool_name\":\"node_doc\",\"duration_us\":",
        )
        .unwrap();
        std::fs::write(
            &path,
            "{\"ts\":\"t\",\"tool_name\":\"node_doc\",\"duration_us\":7,\"error\":\"x\",\"empty_result\":false}\n\
             {\"ts\":\"t\",\"tool_name\":\"node_doc\",\"duration_us\":9,\"empty_result\":false,\"input\":{},\"output\":{\"symbols\":[]}}\n",
        )
        .unwrap();
        let s = &compute_stats(&path).unwrap()[0];
        assert_eq!(s.calls, 3, "legacy line counted, malformed and partial lines skipped");
        assert!((s.error_rate - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(s.output_missing, 1);
        assert_eq!(s.output_samples, 1);
        assert_eq!(s.output_p50_tokens, Some(4), "{{\"symbols\":[]}} is 14 bytes");
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
