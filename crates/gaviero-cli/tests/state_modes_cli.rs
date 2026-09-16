//! End-to-end CLI tests for where a run keeps its state: the discovered
//! workspace (default) or throwaway state (`--isolated`, or no workspace).
//!
//! Each test runs a one-agent document workflow against a fake Ollama
//! server that answers every chat with one `<file>` block. Memory uses the
//! `null` embedder and the reranker is off, so no ONNX model is loaded.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn gaviero_cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gaviero-cli"))
}

const SCRIPT: &str = r#"
client local {
    model "ollama:qwen"
    privacy public
}

agent writer {
    description "write the report"
    client local
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

/// Answer every request with one streamed Ollama chat turn that writes
/// `out/report.md`. Returns the base URL.
fn fake_ollama() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
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
    format!("http://{addr}")
}

fn write_script(dir: &Path) -> PathBuf {
    let path = dir.join("wf.gaviero");
    std::fs::write(&path, SCRIPT).unwrap();
    path
}

fn configure(dir: &Path) {
    std::fs::create_dir_all(dir.join(".gaviero")).unwrap();
    std::fs::write(dir.join(".gaviero").join("settings.json"), SETTINGS).unwrap();
}

fn run(script: &Path, workspace: &Path, extra: &[&str]) -> Output {
    Command::new(gaviero_cli())
        .arg("--script")
        .arg(script)
        .arg("--workspace")
        .arg(workspace)
        .arg("--ollama-base-url")
        .arg(fake_ollama())
        // Every model call — agent and findings extractor — hits the fake.
        .args(["--model", "ollama:qwen"])
        .args(["--skip-mcp-preflight", "--run-timeout", "120"])
        .args(extra)
        // Covers the isolated run with no settings to read.
        .env("GAVIERO_EMBEDDER_MODEL", "null")
        .output()
        .expect("spawn gaviero-cli")
}

fn stderr_of(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "gaviero-cli failed:\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    stderr
}

fn has_workspace_marker(dir: &Path) -> bool {
    let gaviero = dir.join(".gaviero");
    gaviero.join("settings.json").is_file() || gaviero.join("memory.db").is_file()
}

/// True when a gaviero workspace on this machine encloses `dir`, which
/// would turn a "no workspace" fixture into a shared run.
fn enclosed_by_a_workspace(dir: &Path) -> bool {
    let dir = std::fs::canonicalize(dir).unwrap();
    let home = dirs::home_dir().and_then(|h| std::fs::canonicalize(h).ok());
    dir.ancestors()
        .any(|d| Some(d) != home.as_deref() && has_workspace_marker(d))
}

/// A run in a subfolder of a configured workspace writes the workspace's
/// memory, not a new `.gaviero/` in the subfolder.
#[test]
fn a_subfolder_run_shares_the_enclosing_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    configure(&root);
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let script = write_script(&root);

    let stderr = stderr_of(&run(&script, &docs, &["--no-mcp"]));

    assert!(stderr.contains("[state] shared"), "{stderr}");
    assert!(docs.join("out/report.md").is_file(), "{stderr}");
    assert!(
        root.join(".gaviero/memory.db").is_file(),
        "the workspace memory must be opened: {stderr}"
    );
    assert!(
        !docs.join(".gaviero/memory.db").exists(),
        "the subfolder must not get its own memory: {stderr}"
    );
}

/// `--isolated` in a configured workspace leaves its memory and agent
/// configs as they were.
#[test]
fn isolated_leaves_the_workspace_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    configure(&root);
    std::fs::write(root.join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();
    let script = write_script(&root);

    let stderr = stderr_of(&run(&script, &root, &["--isolated"]));

    assert!(stderr.contains("[state] isolated"), "{stderr}");
    assert!(stderr.contains("gaviero server listening"), "{stderr}");
    assert!(root.join("out/report.md").is_file(), "{stderr}");
    assert!(
        !root.join(".gaviero/memory.db").exists(),
        "isolated runs must not open the workspace memory: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".mcp.json")).unwrap(),
        "{\"mcpServers\":{}}\n",
        "the agent config must be restored"
    );
    assert!(!root.join(".cursor").exists(), "{stderr}");
    assert!(!root.join(".claude").exists(), "{stderr}");
    assert!(
        !root.join(".gaviero/mcp-endpoint.json").exists(),
        "{stderr}"
    );
    assert!(!root.join(".gaviero/mcp_calls.ndjson").exists(), "{stderr}");

    let scratch = stderr
        .split("temporary memory at ")
        .nth(1)
        .and_then(|rest| rest.split(" (settings from").next())
        .expect("scratch path in the banner");
    assert!(
        !Path::new(scratch).exists(),
        "isolated state must be removed: {scratch}"
    );
}

/// A live gaviero server on the run root (the TUI) blocks `--isolated`:
/// repointing that folder's agent configs would strand the TUI's agents.
#[tokio::test(flavor = "multi_thread")]
async fn isolated_refuses_a_folder_another_process_serves() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    configure(&root);
    std::fs::write(root.join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();
    let script = write_script(&root);

    let services = gaviero_core::memory::MemoryServices::for_tests_in_memory().unwrap();
    let server =
        gaviero_core::mcp::GavieroMcpServer::with_defaults(services.stores.clone(), root.clone());
    let endpoint = gaviero_core::mcp::McpEndpoint::for_workspace(&root);
    let handle = gaviero_core::mcp::spawn_mcp_server(server, &endpoint).unwrap();
    assert!(endpoint.has_live_server());

    let output = tokio::task::spawn_blocking({
        let (script, root) = (script.clone(), root.clone());
        move || run(&script, &root, &["--isolated"])
    })
    .await
    .unwrap();
    handle.shutdown().await;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("another gaviero process"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(root.join(".mcp.json")).unwrap(),
        "{\"mcpServers\":{}}\n",
        "a refused run must not touch the agent configs"
    );
    assert!(!root.join("out/report.md").exists(), "{stderr}");
}

/// A folder no workspace covers runs isolated and is not turned into one.
#[test]
fn no_workspace_falls_back_to_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    if enclosed_by_a_workspace(&root) {
        eprintln!("skipped: a gaviero workspace encloses {}", root.display());
        return;
    }
    let script = write_script(&root);

    let stderr = stderr_of(&run(&script, &root, &["--no-mcp"]));

    assert!(stderr.contains("running isolated"), "{stderr}");
    assert!(root.join("out/report.md").is_file(), "{stderr}");
    assert!(!has_workspace_marker(&root), "{stderr}");
}
