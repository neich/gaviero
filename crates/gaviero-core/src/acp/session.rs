//! Claude Code subprocess management.
//!
//! Spawns `claude --print --output-format stream-json` and reads NDJSON
//! events from stdout line by line.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::protocol::{StreamEvent, parse_stream_line};

/// Options for the Claude agent subprocess.
#[derive(Debug, Clone)]
pub struct AgentOptions {
    /// Effort level for the CLI (off, low, medium, high, max).
    /// "off" means don't pass --effort (use CLI default).
    pub effort: String,
    /// Max output tokens (0 = use default). Reserved for future API-based backends.
    pub max_tokens: u32,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            effort: "off".to_string(),
            max_tokens: 16384,
        }
    }
}

/// A running Claude Code subprocess.
pub struct AcpSession {
    child: Child,
    stdout: BufReader<tokio::process::ChildStdout>,
    line_buf: String,
    /// Captured stderr lines (shared with drain task).
    stderr_buf: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl AcpSession {
    /// Spawn a new Claude Code subprocess.
    ///
    /// Uses `--print --output-format stream-json` for NDJSON streaming.
    /// Tools are restricted to read-only operations.
    pub fn spawn(
        model: &str,
        cwd: &Path,
        prompt: &str,
        system_prompt: &str,
        allowed_tools: &[&str],
        options: &AgentOptions,
        file_attachments: &[&Path],
    ) -> Result<Self> {
        let tools_str = allowed_tools.join(",");

        let mut cmd = Command::new("claude");
        cmd.arg("--print")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--model")
            .arg(model)
            .arg("--no-session-persistence");

        if !options.effort.is_empty() && options.effort != "off" {
            cmd.arg("--effort").arg(&options.effort);
        }

        if !system_prompt.is_empty() {
            cmd.arg("--append-system-prompt").arg(system_prompt);
        }

        if !tools_str.is_empty() {
            cmd.arg("--allowedTools").arg(&tools_str);
        }

        // Attach files (images, documents) via --file flag
        for file_path in file_attachments {
            cmd.arg("--file").arg(file_path);
        }

        cmd.arg("--add-dir").arg(cwd);

        // Pass prompt via stdin (more robust than positional arg for multi-line prompts)
        cmd.current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        tracing::info!("Spawning claude: model={}, cwd={}, prompt_len={}", model, cwd.display(), prompt.len());

        let mut child = cmd.spawn().context("spawning claude subprocess")?;

        // Write prompt to stdin and close it
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let prompt_bytes = prompt.as_bytes().to_vec();
            tokio::spawn(async move {
                let _ = stdin.write_all(&prompt_bytes).await;
                let _ = stdin.shutdown().await;
            });
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture claude stdout"))?;

        // Spawn a task to drain stderr to a shared buffer + tracing
        let stderr_buf = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let buf = stderr_buf.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "claude_stderr", "{}", line);
                    buf.lock().await.push(line);
                }
            });
        }

        Ok(Self {
            child,
            stdout: BufReader::new(stdout),
            line_buf: String::new(),
            stderr_buf,
        })
    }

    /// Read the next NDJSON event from the subprocess stdout.
    ///
    /// Returns `None` when the subprocess has closed stdout (process exiting).
    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>> {
        loop {
            self.line_buf.clear();
            let bytes_read = self
                .stdout
                .read_line(&mut self.line_buf)
                .await
                .context("reading claude stdout")?;

            if bytes_read == 0 {
                return Ok(None); // EOF
            }

            let line = self.line_buf.trim();
            if line.is_empty() {
                continue; // Skip empty lines
            }

            match parse_stream_line(line) {
                Ok(event) => return Ok(Some(event)),
                Err(e) => {
                    tracing::warn!("Failed to parse NDJSON line: {}: {}", line, e);
                    // Capture non-JSON stdout lines (CLI error messages)
                    self.stderr_buf.lock().await.push(line.to_string());
                    continue;
                }
            }
        }
    }

    /// Kill the subprocess (for cancellation).
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }

    /// Wait for the subprocess to exit and return its status.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        self.child.wait().await.context("waiting for claude subprocess")
    }

    /// Return captured stderr lines (useful for diagnosing exit-without-output).
    pub async fn stderr_output(&self) -> String {
        let lines = self.stderr_buf.lock().await;
        lines.join("\n")
    }
}

/// Check if the `claude` CLI binary is available on PATH.
pub fn is_claude_available() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Query `claude --help` for the model options documented by the CLI.
///
/// Parses the `--model` flag description and extracts all single-quoted
/// strings (aliases and full model names). Returns an empty Vec if the
/// CLI is unavailable or the help text format changes.
pub fn discover_model_options() -> Vec<String> {
    let output = std::process::Command::new("claude")
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    let text = String::from_utf8_lossy(&output.stdout);

    // Find the --model line in the help output
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--model") {
            // Extract all single-quoted strings from the description
            let mut models = Vec::new();
            let mut rest = trimmed.as_bytes();
            while let Some(pos) = rest.iter().position(|&b| b == b'\'') {
                rest = &rest[pos + 1..];
                if let Some(end) = rest.iter().position(|&b| b == b'\'') {
                    let name = std::str::from_utf8(&rest[..end]).unwrap_or("");
                    if !name.is_empty() {
                        models.push(name.to_string());
                    }
                    rest = &rest[end + 1..];
                } else {
                    break;
                }
            }
            return models;
        }
    }

    Vec::new()
}
