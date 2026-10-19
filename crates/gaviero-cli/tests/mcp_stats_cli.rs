//! End-to-end test for `--mcp-stats`: a hand-written telemetry log is read
//! back through the binary, and the estimated output-size columns must
//! count only successful, measurable payloads. No network, no ONNX.

use std::process::Command;

fn gaviero_cli() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gaviero-cli"))
}

fn tokens(output: serde_json::Value) -> usize {
    serde_json::to_string(&output).unwrap().len().div_ceil(4)
}

#[test]
fn mcp_stats_reports_estimated_successful_payload_sizes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let log = tmp.path().join("calls.ndjson");
    let small = serde_json::json!({"symbols": []});
    let large = serde_json::json!({
        "path": "src/a.rs",
        "symbols": [{"qualified_name": "src/a.rs::alpha", "line": 1}]
    });
    let line = |tool: &str, error: Option<&str>, output: Option<&serde_json::Value>| {
        let mut v = serde_json::json!({
            "ts": "2026-10-17T00:00:00Z",
            "tool_name": tool,
            "duration_us": 1000,
            "empty_result": false,
        });
        if let Some(e) = error {
            v["error"] = serde_json::json!(e);
        }
        if let Some(o) = output {
            v["output"] = o.clone();
            v["input"] = serde_json::json!({});
        }
        v.to_string()
    };
    let lines = [
        line("node_doc", None, Some(&small)),
        line("node_doc", None, Some(&large)),
        line("node_doc", Some("denied"), Some(&serde_json::Value::Null)),
        // A record from before outputs were captured.
        line("memory_search", None, None),
    ];
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();

    let out = Command::new(gaviero_cli())
        .current_dir(tmp.path())
        .args(["--mcp-stats", "--mcp-stats-path"])
        .arg(&log)
        .output()
        .expect("run gaviero-cli");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let row = |tool: &str| -> Vec<String> {
        stdout
            .lines()
            .find(|l| l.split_whitespace().next() == Some(tool))
            .unwrap_or_else(|| panic!("no {tool} row in:\n{stdout}"))
            .split_whitespace()
            .map(str::to_string)
            .collect()
    };
    // tool calls p50 p95 err% empty% out-n out-p50 out-p95
    let node_doc = row("node_doc");
    assert_eq!(node_doc[1], "3", "{stdout}");
    assert_eq!(node_doc[6], "2/2", "the failure is not a successful call: {stdout}");
    let (lo, hi) = (tokens(small), tokens(large));
    assert_eq!(node_doc[7], hi.to_string(), "p50 at index ceil(0.5·1) = 1: {stdout}");
    assert_eq!(node_doc[8], hi.to_string(), "{stdout}");
    assert!(lo < hi);

    let search = row("memory_search");
    assert_eq!(search[6], "0/1", "missing output is unknown, not 0: {stdout}");
    assert_eq!(search[7], "n/a", "{stdout}");
    assert_eq!(search[8], "n/a", "{stdout}");
    assert!(stdout.contains("estimated tokens of successful payloads"), "{stdout}");
}
