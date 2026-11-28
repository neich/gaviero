//! End-to-end: `--wait-on-limit` waits out a provider usage limit and
//! resumes the run; without the flag the limit fails the run as before.
//!
//! A fake Ollama answers the first chat request with HTTP 429 ("usage limit
//! reached") and every later one with a turn that writes `out/report.md`.
//! `GAVIERO_LIMIT_CHECK_SECS=1` shortens the 15-minute check interval.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn gaviero_cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gaviero-cli"))
}

const SCRIPT: &str = r#"
agent writer {
    description "write the report"
    tier local
    scope { owned ["out/report.md"] }
    produces ["out/report.md"]
    prompt "Write the report."
    max_retries 0
}

workflow run {
    execution_mode document
    steps [writer]
    max_parallel 1
}
"#;

const SETTINGS: &str = r#"{
  "memory": {
    "embedder": { "model": "null" },
    "reranker": { "enabled": false },
    "extractor": { "enabled": false }
  },
  "mcp": { "gavieroServer": { "http": { "enabled": false } } }
}"#;

const PROFILE: &str = "client local { model \"ollama:qwen\" privacy public }
tier local local
";

/// A fake Ollama whose first `POST /api/chat` is refused with a usage limit.
/// Returns the base URL and the number of chat requests seen so far.
fn limited_ollama() -> (String, Arc<AtomicUsize>) {
    failing_ollama("429 Too Many Requests", "usage limit reached")
}

/// A fake Ollama whose first `POST /api/chat` answers `status` / `text`.
fn failing_ollama(status: &'static str, text: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama");
    let addr = listener.local_addr().unwrap();
    let chats = Arc::new(AtomicUsize::new(0));
    let seen = chats.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line);
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);

            let is_chat = request_line.starts_with("POST") && request_line.contains("/api/chat");
            if is_chat && seen.fetch_add(1, Ordering::SeqCst) == 0 {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let chunk = serde_json::json!({
                "model": "qwen",
                "message": {
                    "role": "assistant",
                    "content": "<file path=\"out/report.md\">\nreport body\n</file>"
                },
                "done": false
            });
            let done = serde_json::json!({
                "model": "qwen", "done": true, "total_duration": 1_000_000,
                "eval_count": 2, "prompt_eval_count": 5
            });
            let payload = format!("{chunk}\n{done}\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
        }
    });
    (format!("http://{addr}"), chats)
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
    std::fs::write(dir.path().join(".gaviero").join("settings.json"), SETTINGS).unwrap();
    std::fs::write(dir.path().join("wf.gaviero"), SCRIPT).unwrap();
    std::fs::write(dir.path().join("local-profile.gaviero"), PROFILE).unwrap();
    dir
}

fn run(dir: &Path, ollama: &str, extra: &[&str]) -> Output {
    Command::new(gaviero_cli())
        .arg("--script")
        .arg(dir.join("wf.gaviero"))
        .arg("--profile")
        .arg(dir.join("local-profile.gaviero"))
        .arg("--workspace")
        .arg(dir)
        .arg("--ollama-base-url")
        .arg(ollama)
        .args(["--model", "ollama:qwen", "--no-mcp"])
        .args(["--skip-mcp-preflight", "--run-timeout", "120"])
        .args(extra)
        .env("GAVIERO_EMBEDDER_MODEL", "null")
        .env("GAVIERO_LIMIT_CHECK_SECS", "1")
        .env(
            "GAVIERO_CLI_RUNS_DIR",
            std::env::temp_dir().join("gaviero-cli-runs-tests"),
        )
        .env(
            "GAVIERO_CLAIMS_DIR",
            std::env::temp_dir()
                .join("gaviero-cli-runs-tests")
                .join("claims"),
        )
        .output()
        .expect("spawn gaviero-cli")
}

#[test]
fn waits_out_a_provider_limit_and_resumes() {
    let dir = workspace();
    let (ollama, chats) = limited_ollama();
    let output = run(dir.path(), &ollama, &["--wait-on-limit"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "gaviero-cli failed:\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains("[limit] 1 agent(s) stopped by a provider limit (ollama:qwen)"), "{stderr}");
    assert!(stderr.contains("limit cleared — resuming"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/report.md"))
            .unwrap()
            .trim(),
        "report body"
    );
    // Refused agent turn, the probe, the resumed agent turn.
    assert!(chats.load(Ordering::SeqCst) >= 3, "chats: {}", chats.load(Ordering::SeqCst));
}

#[test]
fn without_the_flag_a_limit_fails_the_run() {
    let dir = workspace();
    let (ollama, chats) = limited_ollama();
    let output = run(dir.path(), &ollama, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "run should fail:\n{stderr}");
    assert!(!stderr.contains("[limit]"), "{stderr}");
    assert_eq!(chats.load(Ordering::SeqCst), 1);
    assert!(!dir.path().join("out/report.md").exists());
}

#[test]
fn a_non_limit_failure_does_not_wait() {
    let dir = workspace();
    let (ollama, chats) = failing_ollama("500 Internal Server Error", "model crashed");
    let output = run(dir.path(), &ollama, &["--wait-on-limit"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "run should fail:\n{stderr}");
    assert!(!stderr.contains("[limit]"), "{stderr}");
    assert_eq!(chats.load(Ordering::SeqCst), 1);
}
