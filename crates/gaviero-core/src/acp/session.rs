//! Claude Code subprocess management.
//!
//! Spawns `claude --print --output-format stream-json` and reads NDJSON
//! events from stdout line by line.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use super::protocol::{StreamEvent, parse_stream_line};

/// If the enriched prompt + system prompt combined exceed this size, pass the
/// prompt to Claude through a workspace-local tempfile via `@`-reference
/// instead of argv. Linux `ARG_MAX` is ~128 KB; 32 KB leaves ample headroom
/// for the other flag args, environment, and OS overhead. The tempfile path
/// itself has no practical size ceiling.
const ARGV_THRESHOLD: usize = 32_768;

/// Subdirectory under the workspace root where oversized prompt tempfiles live.
/// `--add-dir <cwd>` already lets Claude read files under the workspace; the
/// `.gaviero/tmp` subpath keeps these transient files out of the way of code.
const TEMP_SUBDIR: &str = ".gaviero/tmp";

/// Options for the Claude agent subprocess.
#[derive(Debug, Clone)]
pub struct AgentOptions {
    /// Effort level for the CLI (off, low, medium, high, max).
    /// "off" means don't pass --effort (use CLI default).
    pub effort: String,
    /// Max output tokens (0 = use default). Reserved for future API-based backends.
    pub max_tokens: u32,
    /// When true, pass `--dangerously-skip-permissions` so the subprocess never
    /// pauses for permission prompts. Intended for single-prompt "yes to all" mode.
    pub auto_approve: bool,
    /// When `Some`, resume the Claude session with the given id (Claude's
    /// `--resume <id>` flag) so model context (prior messages, read file
    /// cache) carries across turns. When `None`, a fresh one-shot session
    /// is spawned.
    pub resume_session_id: Option<String>,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            effort: "off".to_string(),
            max_tokens: 16384,
            auto_approve: false,
            resume_session_id: None,
        }
    }
}

/// A running Claude Code subprocess.
pub struct AcpSession {
    child: Child,
    stdout: BufReader<tokio::process::ChildStdout>,
    /// Channel sender for lines written to the subprocess stdin.
    /// Used to send permission responses without closing stdin.
    stdin_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    line_buf: String,
    /// Captured stderr lines (shared with drain task).
    stderr_buf: Arc<tokio::sync::Mutex<Vec<String>>>,
    /// Held only so the tempfile survives until the subprocess exits.
    /// `NamedTempFile::drop` removes the file from disk automatically.
    _prompt_tempfile: Option<tempfile::NamedTempFile>,
}

/// Decide whether a prompt of `prompt_len + system_prompt_len` bytes should
/// be passed via argv or a tempfile. Extracted so tests can exercise the
/// decision without spawning a subprocess.
pub(crate) fn would_use_tempfile(prompt_len: usize, system_prompt_len: usize) -> bool {
    prompt_len + system_prompt_len >= ARGV_THRESHOLD
}

/// Write `prompt` to a workspace-local tempfile and return (NamedTempFile,
/// short argv to use instead of the full prompt). The argv tells Claude to
/// read the file via its `@`-syntax and follow its instructions.
fn spill_prompt_to_tempfile(
    cwd: &Path,
    prompt: &str,
) -> Result<(tempfile::NamedTempFile, String)> {
    let dir: PathBuf = cwd.join(TEMP_SUBDIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating tempdir {}", dir.display()))?;

    let mut file = tempfile::Builder::new()
        .prefix("prompt-")
        .suffix(".md")
        .tempfile_in(&dir)
        .context("creating prompt tempfile")?;
    file.write_all(prompt.as_bytes())
        .context("writing prompt tempfile")?;
    file.flush().context("flushing prompt tempfile")?;

    let rel: PathBuf = file
        .path()
        .strip_prefix(cwd)
        .map(PathBuf::from)
        .unwrap_or_else(|_| file.path().to_path_buf());

    let wrapper = format!(
        "Read the full prompt at @{} and follow its instructions.",
        rel.display()
    );
    Ok((file, wrapper))
}

impl AcpSession {
    /// Spawn a new Claude Code subprocess.
    ///
    /// Uses `--print --output-format stream-json` for NDJSON streaming.
    ///
    /// `available_tools` controls which tools the model can use (`--tools`).
    /// `approved_tools` controls which of those are auto-approved without
    /// a permission prompt (`--allowedTools`). Tools in `available_tools`
    /// but not in `approved_tools` will trigger `PermissionRequest` events.
    pub fn spawn(
        model: &str,
        cwd: &Path,
        prompt: &str,
        system_prompt: &str,
        available_tools: &[&str],
        approved_tools: &[&str],
        options: &AgentOptions,
        file_attachments: &[&Path],
    ) -> Result<Self> {
        // Decide argv vs tempfile for the prompt. Small prompts take the
        // zero-overhead argv path; anything that might approach ARG_MAX is
        // spilled to a workspace-local `.gaviero/tmp/prompt-*.md` file and
        // referenced via `@path` so the argv stays tiny.
        let use_tempfile = would_use_tempfile(prompt.len(), system_prompt.len());
        let (prompt_tempfile, argv_prompt): (Option<tempfile::NamedTempFile>, String) =
            if use_tempfile {
                let (file, wrapper) = spill_prompt_to_tempfile(cwd, prompt)?;
                tracing::info!(
                    "Spilling prompt to tempfile: path={}, prompt_len={}",
                    file.path().display(),
                    prompt.len(),
                );
                (Some(file), wrapper)
            } else {
                (None, prompt.to_string())
            };

        let mut cmd = Command::new("claude");
        cmd.arg("--print")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--model")
            .arg(model);

        // Session reuse: when a prior session_id is known (captured from the
        // first turn's SystemInit event), resume it so Claude's model keeps
        // conversation context, read-file cache, and thinking state. Skipping
        // this flag (and keeping --no-session-persistence) gives a fresh
        // one-shot session — used on turn 1 and for swarm work units.
        // M0 instrumentation: record resume hit/miss so baselines can
        // correlate continuity mode with injection size. `resume_passed`
        // reflects what we asked Claude to do; the CLI's `SystemInit`
        // event confirms whether Claude actually accepted the id
        // (logged separately in AcpPipeline::send_prompt_via_claude).
        let resume_passed = matches!(
            options.resume_session_id.as_deref(),
            Some(id) if !id.is_empty()
        );
        tracing::info!(
            target: "turn_metrics",
            provider = "claude",
            resume_passed,
            "session_resume_attempt"
        );
        match options.resume_session_id.as_deref() {
            Some(id) if !id.is_empty() => {
                cmd.arg("--resume").arg(id);
            }
            _ => {
                cmd.arg("--no-session-persistence");
            }
        }

        if !options.effort.is_empty() && options.effort != "off" {
            cmd.arg("--effort").arg(&options.effort);
        }

        if options.auto_approve {
            cmd.arg("--dangerously-skip-permissions");
        } else if !approved_tools.is_empty() {
            cmd.arg("--allowedTools").arg(approved_tools.join(","));
        }

        if !system_prompt.is_empty() {
            cmd.arg("--append-system-prompt").arg(system_prompt);
        }

        if !available_tools.is_empty() {
            cmd.arg("--tools").arg(available_tools.join(","));
        }

        // Attach files (images, documents) via --file flag
        for file_path in file_attachments {
            cmd.arg("--file").arg(file_path);
        }

        cmd.arg("--add-dir").arg(cwd);

        // Pass prompt as positional arg so stdin stays open for permission responses.
        cmd.arg("--").arg(&argv_prompt);

        cmd.current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        tracing::info!(
            "Spawning claude: model={}, cwd={}, prompt_len={}, via_tempfile={}",
            model,
            cwd.display(),
            prompt.len(),
            use_tempfile,
        );

        let mut child = cmd.spawn().map_err(|e| {
            if matches!(e.kind(), std::io::ErrorKind::NotFound) {
                anyhow::anyhow!(
                    "spawning claude subprocess: {e}\n\
                     The `claude` CLI binary was not found on PATH. \
                     Install it from https://docs.anthropic.com/claude/docs/claude-code, \
                     or switch provider by setting agent.model to a `codex:...` / `ollama:...` spec."
                )
            } else if e.raw_os_error() == Some(7) {
                // E2BIG after the tempfile fallback would mean the system
                // prompt itself is >32 KB. We don't generate anything that
                // size, so this is genuinely pathological — surface the raw
                // error with a pointer at the system prompt as the suspect.
                anyhow::anyhow!(
                    "spawning claude subprocess: argument list too long.\n\
                     This shouldn't happen — user prompts spill to a tempfile above {ARGV_THRESHOLD} B.\n\
                     The system prompt or flag arguments must be pathologically large; report this as a bug."
                )
            } else {
                anyhow::anyhow!("spawning claude subprocess: {e}")
            }
        })?;

        // Keep stdin open for permission responses.
        // When a PermissionRequest arrives, respond_permission() sends
        // JSON via this channel. Dropping the sender closes stdin.
        let stdin_tx = if let Some(mut stdin) = child.stdin.take() {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    if stdin.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
                let _ = stdin.shutdown().await;
            });
            Some(tx)
        } else {
            None
        };

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
            stdin_tx,
            line_buf: String::new(),
            stderr_buf,
            _prompt_tempfile: prompt_tempfile,
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

    /// Check if the subprocess has already exited (non-blocking).
    /// Returns `true` if the process has exited, `false` if still running.
    pub fn try_wait_exited(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => true,
            _ => false,
        }
    }

    /// Kill the subprocess (for cancellation).
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }

    /// Send a permission response back to the Claude subprocess via stdin.
    ///
    /// Called after the pipeline receives a `PermissionRequest` event and
    /// the user approves or denies the action in the TUI.
    pub fn respond_permission(&self, allow: bool, request_id: &str) {
        let Some(ref tx) = self.stdin_tx else { return };
        let decision = if allow { "allow" } else { "deny" };
        let msg = format!(
            "{{\"type\":\"permission_response\",\"decision\":\"{}\",\"permission_request_id\":\"{}\"}}\n",
            decision, request_id
        );
        let _ = tx.send(msg);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_prompt_uses_argv() {
        assert!(!would_use_tempfile(0, 0));
        assert!(!would_use_tempfile(1_000, 500));
        // Right at the boundary — well below ARGV_THRESHOLD.
        assert!(!would_use_tempfile(ARGV_THRESHOLD - 1, 0));
    }

    #[test]
    fn large_prompt_spills_to_tempfile() {
        assert!(would_use_tempfile(ARGV_THRESHOLD, 0));
        assert!(would_use_tempfile(100_000, 0));
        // Combined prompt + system prompt crossing threshold.
        assert!(would_use_tempfile(ARGV_THRESHOLD - 100, 200));
    }

    #[test]
    fn spill_creates_readable_file_and_wrapper_refs_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();

        let big_prompt = "x".repeat(50_000);
        let (file, wrapper) = spill_prompt_to_tempfile(cwd, &big_prompt).expect("spill");

        // File lives under {cwd}/.gaviero/tmp and has the full prompt on disk.
        let on_disk = file.path();
        assert!(on_disk.starts_with(cwd.join(TEMP_SUBDIR)));
        let content = std::fs::read_to_string(on_disk).expect("read tempfile");
        assert_eq!(content, big_prompt);

        // Wrapper argv references the tempfile with `@relative_path` and is tiny.
        assert!(wrapper.contains("@"));
        assert!(wrapper.len() < 500);

        // NamedTempFile drops → file removed from disk.
        let held_path = on_disk.to_path_buf();
        drop(file);
        assert!(!held_path.exists(), "tempfile should be cleaned up on drop");
    }
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
