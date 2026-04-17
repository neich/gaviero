//! Codex CLI subprocess backend.
//!
//! Implements [`AgentBackend`] by spawning OpenAI's `codex exec` non-interactive
//! subprocess and mapping its streaming stdout into the unified
//! [`UnifiedStreamEvent`] stream. File changes are proposed via `<file>` blocks
//! in the response text (detected and routed through the Write Gate), matching
//! the pattern used by the Claude Code backend.
//!
//! Codex is invoked with `sandbox=read-only` and `approval_policy=never` so that
//! tool-use stays non-interactive; the model emits proposed writes as `<file>`
//! blocks rather than touching disk directly.

use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use futures::Stream;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::{find_next_file_block, parse_file_blocks};

use super::shared::{build_enriched_prompt, default_editor_system_prompt};
use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
};

const DEFAULT_CODEX_MODEL: &str = "gpt-5-codex";

/// Backend that spawns the Codex CLI as a subprocess.
pub struct CodexBackend {
    model: String,
    display_name: String,
}

impl CodexBackend {
    pub fn new(model: &str) -> Self {
        let m = if model.is_empty() {
            DEFAULT_CODEX_MODEL
        } else {
            model
        };
        Self {
            model: m.to_string(),
            display_name: format!("codex:{}", m),
        }
    }
}

#[async_trait::async_trait]
impl AgentBackend for CodexBackend {
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let system_prompt = request
            .system_prompt
            .clone()
            .unwrap_or_else(|| default_editor_system_prompt(&self.capabilities()));

        let user_prompt = build_enriched_prompt(
            &request.prompt,
            &request.conversation_history,
            &request.file_refs,
        );

        let combined_prompt = format!("{system_prompt}\n\n{user_prompt}");

        let mut cmd = Command::new("codex");
        cmd.arg("exec")
            .arg("--skip-git-repo-check")
            .arg("--model")
            .arg(&self.model)
            .arg("--config")
            .arg("approval_policy=never")
            .arg("--config")
            .arg("sandbox=read-only");

        if let Some(codex_effort) = map_effort_to_codex(request.effort.as_deref()) {
            cmd.arg("--config")
                .arg(format!("model_reasoning_effort={codex_effort}"));
        }

        // Forward every `extra { k v }` pair as a `-c k=v` override to codex.
        // Codex treats `--config` args as TOML-shaped overrides and silently
        // ignores unknown keys, so this is a safe pass-through: users opt in
        // explicitly via the DSL.
        for (k, v) in &request.extra {
            cmd.arg("--config").arg(format!("{k}={v}"));
        }

        cmd.arg(&combined_prompt)
            .current_dir(&request.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "spawning codex subprocess: {e}\n\
                 The `codex` CLI binary was not found on PATH. \
                 Install it from https://github.com/openai/codex, \
                 or switch provider by setting agent.model to a `claude:...` / `ollama:...` spec."
            )
        })?;
        let stdout = child.stdout.take().context("codex stdout unavailable")?;
        let stderr = child.stderr.take().context("codex stderr unavailable")?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<UnifiedStreamEvent>>(64);

        // Drain stderr concurrently so the buffer doesn't fill and block the subprocess.
        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let result = drive_codex_stdout(stdout, tx_clone.clone()).await;
            let exit_status = child.wait().await;
            let stderr_text = stderr_handle.await.unwrap_or_default();

            let duration_ms = Some(start.elapsed().as_millis() as u64);

            match result {
                Ok(()) => {
                    let ok = exit_status.as_ref().map(|s| s.success()).unwrap_or(false);
                    if ok {
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Usage(TokenUsage {
                                duration_ms,
                                ..Default::default()
                            })))
                            .await;
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)))
                            .await;
                    } else {
                        let msg = format_exit_error(&exit_status, &stderr_text);
                        let _ = tx_clone.send(Ok(UnifiedStreamEvent::Error(msg))).await;
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                            .await;
                    }
                }
                Err(e) => {
                    let _ = tx_clone
                        .send(Ok(UnifiedStreamEvent::Error(format!("{e:#}"))))
                        .await;
                    let _ = tx_clone
                        .send(Ok(UnifiedStreamEvent::Done(StopReason::Error)))
                        .await;
                }
            }
        });

        drop(tx);
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_use: true,
            streaming: true,
            vision: false,
            extended_thinking: false,
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            supports_file_blocks: true,
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let output = Command::new("codex")
            .arg("--version")
            .output()
            .await
            .context("codex binary not found on PATH")?;
        if output.status.success() {
            Ok(())
        } else {
            anyhow::bail!("codex --version exited with {}", output.status)
        }
    }
}

/// Read stdout in chunks and emit TextDelta + FileBlock events.
async fn drive_codex_stdout(
    stdout: tokio::process::ChildStdout,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut buf = [0u8; 4096];
    let mut full_text = String::new();
    let mut file_scan_pos: usize = 0;

    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("reading codex stdout")?;
        if n == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
        full_text.push_str(&chunk);

        if tx
            .send(Ok(UnifiedStreamEvent::TextDelta(chunk)))
            .await
            .is_err()
        {
            return Ok(()); // receiver dropped
        }

        // Detect complete <file> blocks as they arrive.
        while let Some((path, content, end)) = find_next_file_block(&full_text, file_scan_pos) {
            file_scan_pos = end;
            if tx
                .send(Ok(UnifiedStreamEvent::FileBlock { path, content }))
                .await
                .is_err()
            {
                return Ok(());
            }
        }
    }

    // Catch any trailing blocks not detected mid-stream (shouldn't normally happen
    // because scan is incremental, but belt-and-suspenders).
    for (path, content) in parse_file_blocks(&full_text[file_scan_pos..]) {
        let _ = tx
            .send(Ok(UnifiedStreamEvent::FileBlock { path, content }))
            .await;
    }

    Ok(())
}

/// Map the DSL's provider-neutral `effort` vocabulary into Codex's
/// `model_reasoning_effort` config value.
///
/// Gaviero accepts `off`, `auto`, `low`, `medium`, `high`, `xhigh`, `max`
/// (shared with Claude). Codex only understands `minimal`, `low`, `medium`,
/// `high`. `None` means "omit the flag and let Codex use its default".
///
/// `xhigh` / `max` are clamped to `high` — Codex has no tier above that.
/// `off` / `auto` map to `None` so the user can use a single `client` block
/// across providers without forcing Codex into a specific tier.
fn map_effort_to_codex(effort: Option<&str>) -> Option<&'static str> {
    match effort?.trim().to_ascii_lowercase().as_str() {
        "off" | "auto" | "" => None,
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" | "xhigh" | "max" => Some("high"),
        other => {
            tracing::warn!(
                target: "backend.codex",
                effort = other,
                "unknown effort value; not forwarding to codex (supported: minimal|low|medium|high|xhigh|max|off|auto)"
            );
            None
        }
    }
}

fn format_exit_error(
    exit_status: &std::io::Result<std::process::ExitStatus>,
    stderr_text: &str,
) -> String {
    let status_line = match exit_status {
        Ok(s) => format!("codex exited with {s}"),
        Err(e) => format!("failed to wait for codex: {e}"),
    };
    if stderr_text.trim().is_empty() {
        format!("{status_line}\nCheck that the `codex` CLI is installed and OPENAI_API_KEY is set.")
    } else {
        format!("{status_line}\n{}", stderr_text.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_name_contains_model() {
        let b = CodexBackend::new("gpt-5-codex");
        assert!(b.name().contains("codex"));
        assert!(b.name().contains("gpt-5-codex"));
    }

    #[test]
    fn test_empty_model_uses_default() {
        let b = CodexBackend::new("");
        assert!(b.name().ends_with(DEFAULT_CODEX_MODEL));
    }

    #[test]
    fn test_capabilities_file_blocks_supported() {
        let b = CodexBackend::new("gpt-5-codex");
        let caps = b.capabilities();
        assert!(caps.supports_file_blocks);
        assert!(caps.supports_system_prompt);
        assert!(caps.streaming);
    }

    #[test]
    fn test_map_effort_to_codex_known_values() {
        assert_eq!(map_effort_to_codex(Some("low")), Some("low"));
        assert_eq!(map_effort_to_codex(Some("medium")), Some("medium"));
        assert_eq!(map_effort_to_codex(Some("high")), Some("high"));
        assert_eq!(map_effort_to_codex(Some("minimal")), Some("minimal"));
    }

    #[test]
    fn test_map_effort_to_codex_clamps_above_high() {
        assert_eq!(map_effort_to_codex(Some("xhigh")), Some("high"));
        assert_eq!(map_effort_to_codex(Some("max")), Some("high"));
    }

    #[test]
    fn test_map_effort_to_codex_off_and_auto_omit() {
        assert_eq!(map_effort_to_codex(Some("off")), None);
        assert_eq!(map_effort_to_codex(Some("auto")), None);
        assert_eq!(map_effort_to_codex(None), None);
    }

    #[test]
    fn test_map_effort_to_codex_case_insensitive() {
        assert_eq!(map_effort_to_codex(Some("HIGH")), Some("high"));
        assert_eq!(map_effort_to_codex(Some("Medium")), Some("medium"));
    }

    #[test]
    fn test_map_effort_to_codex_unknown_omitted() {
        assert_eq!(map_effort_to_codex(Some("turbo")), None);
    }

    #[test]
    fn test_format_exit_error_with_stderr() {
        let err: std::io::Result<std::process::ExitStatus> =
            Err(std::io::Error::new(std::io::ErrorKind::Other, "bad"));
        let msg = format_exit_error(&err, "auth failure\n");
        assert!(msg.contains("bad"));
        assert!(msg.contains("auth failure"));
    }
}
