//! End-to-end CLI tests for the `--history` per-turn reader.
//!
//! A fixture log is written through the same `HistoryRecorder` the TUI uses,
//! into `<tempdir>/.gaviero/history/turns.ndjson`; the binary must read it
//! back through the shared reader in every output mode. No network, no ONNX.

use std::process::{Command, Output};

use gaviero_core::history::{
    HistoryKind, HistoryRecorder, ProviderUsage, ToolOutput, TurnEnd, TurnStart,
    memory_injection_record, tool_call_record,
};

fn gaviero_cli() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set by Cargo for integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gaviero-cli"))
}

fn start(prompt: &str) -> TurnStart {
    TurnStart {
        provider: "claude".into(),
        model: "claude:sonnet".into(),
        conv_title: Some("history cli".into()),
        workspace_root: ".".into(),
        prompt: prompt.into(),
        prompt_bytes: 0,
        prompt_truncated: false,
        input_tokens_est: None,
        estimator: None,
    }
}

/// One complete Claude-shaped turn and one cancelled turn.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let r = HistoryRecorder::for_workspace(tmp.path());
    r.begin_turn("c1", "c1-100", start("explain the token estimator"), false);
    r.tool_started("c1-100", "Read src/tokens.rs");
    r.tool_completed(
        "c1-100",
        tool_call_record(
            "Read",
            Some("toolu_1".into()),
            Some(9),
            Some(serde_json::json!({"file_path": "src/tokens.rs"})),
            ToolOutput::Full {
                content: "pub fn estimate_text_tokens".into(),
                is_error: false,
            },
            Some("Read src/tokens.rs".into()),
        ),
    );
    r.push(
        "c1-100",
        HistoryKind::MemoryInjection(memory_injection_record(
            1,
            4,
            40,
            2000,
            Some("<project_memory>words×1.3</project_memory>".into()),
            None,
        )),
    );
    r.note_usage(
        "c1-100",
        ProviderUsage {
            input_tokens: 5,
            cache_creation_input_tokens: 340,
            cache_read_input_tokens: 12_000,
            output_tokens: 1_204,
        },
    );
    r.end_turn("c1-100", TurnEnd::new(false, None, 0));
    r.begin_turn("c1", "c1-200", start("never mind"), false);
    r.end_turn("c1-200", TurnEnd::new(true, None, 0));
    tmp
}

fn run(repo: &std::path::Path, extra: &[&str]) -> Output {
    Command::new(gaviero_cli())
        .arg("--repo")
        .arg(repo)
        .arg("--history")
        .args(extra)
        .output()
        .expect("spawn gaviero-cli")
}

fn stdout_of(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "gaviero-cli failed:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn summary_lists_turns_with_labelled_numbers() {
    let tmp = fixture();
    let out = stdout_of(&run(tmp.path(), &[]));
    assert!(out.contains("History — 2 turn(s)"), "{out}");
    assert!(out.contains("turn c1-100"), "{out}");
    assert!(out.contains("cancelled"), "{out}");
    assert!(out.contains("1 call(s)"), "{out}");
    // exact prefix = 5 + 340 + 12,000
    assert!(out.contains("exact 12,345 in / 1,204 out"), "{out}");
    assert!(out.contains("~ = estimate"), "{out}");
}

#[test]
fn turn_dump_prints_every_record_of_that_turn() {
    let tmp = fixture();
    let out = stdout_of(&run(tmp.path(), &["--history-turn", "c1-100"]));
    assert!(!out.contains("turn c1-200"), "{out}");
    for kind in ["turn_start", "tool_call", "memory_injection", "turn_end"] {
        assert!(out.contains(kind), "missing {kind}: {out}");
    }
    assert!(out.contains("pub fn estimate_text_tokens"), "{out}");
}

#[test]
fn json_mode_is_raw_ndjson() {
    let tmp = fixture();
    let out = stdout_of(&run(tmp.path(), &["--history-json", "--history-last", "1"]));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "turn_start + turn_end of the newest turn: {out}"
    );
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
        assert_eq!(v["turn_id"], "c1-200");
    }
}

#[test]
fn stats_aggregate_by_provider() {
    let tmp = fixture();
    let out = stdout_of(&run(tmp.path(), &["--history-stats"]));
    assert!(out.contains("History stats — 2 turn(s)"), "{out}");
    assert!(out.contains("1 complete · 1 cancelled"), "{out}");
    assert!(out.lines().any(|l| l.starts_with("claude")), "{out}");
}

#[test]
fn unknown_turn_fails_and_missing_log_does_not() {
    let tmp = fixture();
    let output = run(tmp.path(), &["--history-turn", "nope"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no turn `nope`"));

    let empty = tempfile::tempdir().expect("tempdir");
    let out = stdout_of(&run(empty.path(), &[]));
    assert!(out.contains("No history turns"), "{out}");
}
