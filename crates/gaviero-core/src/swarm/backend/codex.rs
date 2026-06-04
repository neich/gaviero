//! Codex CLI subprocess backend.
//!
//! Implements [`AgentBackend`] by spawning OpenAI's `codex exec` non-interactive
//! subprocess and mapping its streaming stdout into the unified
//! [`UnifiedStreamEvent`] stream. File changes are proposed via `<file>` blocks
//! in the response text (detected and routed through the Write Gate), matching
//! the pattern used by the Claude Code backend.
//!
//! Codex is invoked with `--sandbox read-only` and `--config approval_policy=never`
//! so that tool-use stays non-interactive; the model emits proposed writes as
//! `<file>` blocks rather than touching disk directly. `--ask-for-approval` only
//! exists on the top-level `codex` command, not on `codex exec`, so the approval
//! policy must be set via the TOML config override.
//!
//! Workspace-mode multi-folder is plumbed via `request.additional_roots`: every
//! sibling folder beyond the cwd is forwarded as a `--add-dir <path>` flag so
//! the model can read/write across the whole workspace.

use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use futures::Stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::{find_next_file_block, parse_file_blocks};

use super::shared::{build_enriched_prompt, default_editor_system_prompt};
use super::{
    AgentBackend, Capabilities, CompletionRequest, StopReason, TokenUsage, UnifiedStreamEvent,
};

const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// Prompts at or above this size are piped to `codex exec` via stdin
/// instead of appended as a positional argv. Linux's `MAX_ARG_STRLEN`
/// is 128 KB per argument; 32 KB keeps comfortable headroom for the
/// rest of the argv (flags, `--config` keys, `--add-dir` roots) and
/// matches the threshold the ACP/Claude path uses for symmetry.
const ARGV_THRESHOLD: usize = 32_768;

/// Whether a prompt of `len` bytes should be piped via stdin rather
/// than passed as a positional argv. Extracted so tests can exercise
/// the decision without spawning a subprocess.
fn would_use_stdin(len: usize) -> bool {
    len >= ARGV_THRESHOLD
}

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
        for arg in codex_exec_args(
            &self.model,
            request.effort.as_deref(),
            &request.extra,
            &request.additional_roots,
            &request.workspace_root,
        ) {
            cmd.arg(arg);
        }

        // Small prompts ride argv (zero-overhead, simpler); large prompts
        // pipe via stdin so we don't hit MAX_ARG_STRLEN (E2BIG). codex
        // exec reads the prompt from stdin when no positional argument
        // is supplied.
        let use_stdin = would_use_stdin(combined_prompt.len());
        if !use_stdin {
            cmd.arg(&combined_prompt);
        }
        cmd.current_dir(&request.workspace_root)
            .env("NO_COLOR", "1")
            .stdin(if use_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let prompt_len = combined_prompt.len();
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "spawning codex subprocess: {e}\n\
                     The `codex` CLI binary was not found on PATH. \
                     Install it from https://github.com/openai/codex, \
                     or switch provider by setting agent.model to a `claude:...` / `ollama:...` spec."
                )
            } else {
                anyhow::anyhow!(
                    "spawning codex subprocess (prompt {} bytes via {}): {e}",
                    prompt_len,
                    if use_stdin { "stdin" } else { "argv" },
                )
            }
        })?;

        // For the stdin path, hand the prompt to codex and close stdin
        // before we drive stdout — codex won't start streaming until it
        // sees EOF on its input.
        if use_stdin {
            let mut stdin = child.stdin.take().context("codex stdin unavailable")?;
            stdin
                .write_all(combined_prompt.as_bytes())
                .await
                .context("writing codex prompt to stdin")?;
            stdin.shutdown().await.context("closing codex stdin")?;
        }

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

fn codex_exec_args(
    model: &str,
    effort: Option<&str>,
    extra: &[(String, String)],
    additional_roots: &[std::path::PathBuf],
    workspace_root: &std::path::Path,
) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "--skip-git-repo-check".to_string(),
        "--model".to_string(),
        model.to_string(),
    ];

    // Approval / sandbox shape depends on whether any MCP server is
    // configured for this worktree:
    //
    // * **No MCP**: keep the locked-down defaults — `--sandbox read-only`
    //   plus `--config approval_policy=never` so shell tools and writes
    //   never escape the worktree.
    //
    // * **Any MCP (stdio or remote)**: switch to
    //   `--dangerously-bypass-approvals-and-sandbox`. Probed against
    //   `codex-cli 0.131.0` (2026-06-03): every standard approval policy
    //   (`never`, `on-request`, `on-failure`, `untrusted`) auto-cancels
    //   MCP tool calls as `user cancelled MCP tool call` in `codex exec`,
    //   because there's no user to satisfy the elicitation. The bypass
    //   flag is codex's documented escape hatch for "externally
    //   sandboxed" environments — gaviero swarm agents qualify: each
    //   runs in its own per-agent git worktree (read-only branch of
    //   user's repo, cleaned up afterwards) and every file change
    //   merges back through the Write Gate. `--mcp-codex-trust granted`
    //   is the user-facing opt-in to this trade.
    let has_mcp = crate::mcp::codex_synth_has_any_mcp(workspace_root);
    if has_mcp {
        tracing::warn!(
            target: "backend.codex",
            workspace = %workspace_root.display(),
            "MCP servers detected in synthesized .codex/config.toml — using \
             --dangerously-bypass-approvals-and-sandbox so MCP tool calls can fire \
             (codex exec auto-cancels MCP under every standard approval_policy). \
             Per-agent git worktree + Write Gate at merge time bound the blast radius.",
        );
        args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    } else {
        args.push("--config".to_string());
        args.push("approval_policy=never".to_string());
        args.push("--sandbox".to_string());
        args.push("read-only".to_string());
    }

    // Workspace-mode multi-folder: each sibling folder is added as a writable
    // root. The primary cwd reaches codex via `Command::current_dir`; these
    // are the *additional* roots beyond it. Skips empty paths defensively.
    for root in additional_roots {
        if root.as_os_str().is_empty() {
            continue;
        }
        args.push("--add-dir".to_string());
        args.push(root.to_string_lossy().into_owned());
    }

    if let Some(codex_effort) = map_effort_to_codex(effort) {
        args.push("--config".to_string());
        args.push(format!("model_reasoning_effort={codex_effort}"));
    }

    // Replay the synthesized `<worktree>/.codex/config.toml` MCP servers
    // as `--config mcp_servers.X.Y=Z` overrides. Codex's CLI only loads
    // `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`), never
    // the per-worktree file, so without this step the external MCP
    // servers Gaviero synthesizes (e.g. Semantic Scholar, context7) are
    // invisible to `codex exec`.
    let codex_config = workspace_root.join(".codex/config.toml");
    for pair in crate::mcp::codex_mcp_overrides_from_config_file(&codex_config) {
        args.push("--config".to_string());
        args.push(pair);
    }

    // Forward every `extra { k v }` pair as a `-c k=v` override to codex.
    // Codex treats `--config` args as TOML-shaped overrides and silently
    // ignores unknown keys, so this is a safe pass-through: users opt in
    // explicitly via the DSL.
    for (k, v) in extra {
        args.push("--config".to_string());
        args.push(format!("{k}={v}"));
    }

    args
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
        let b = CodexBackend::new("gpt-5.5");
        assert!(b.name().contains("codex"));
        assert!(b.name().contains("gpt-5.5"));
    }

    #[test]
    fn test_empty_model_uses_default() {
        let b = CodexBackend::new("");
        assert!(b.name().ends_with(DEFAULT_CODEX_MODEL));
    }

    #[test]
    fn test_capabilities_file_blocks_supported() {
        let b = CodexBackend::new("gpt-5.5");
        let caps = b.capabilities();
        assert!(caps.supports_file_blocks);
        assert!(caps.supports_system_prompt);
        assert!(caps.streaming);
    }

    #[test]
    fn test_codex_exec_args_force_read_only_review_channel() {
        let args = codex_exec_args("gpt-5.5", Some("high"), &[], &[], std::path::Path::new(""));
        assert!(
            args.windows(2)
                .any(|w| w == ["--config", "approval_policy=never"])
        );
        assert!(args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(!args.iter().any(|a| a == "--ask-for-approval"));
    }

    #[test]
    fn test_codex_exec_args_emits_add_dir_for_each_additional_root() {
        let extras = [
            std::path::PathBuf::from("/tmp/sibling-a"),
            std::path::PathBuf::from("/tmp/sibling-b"),
        ];
        let args = codex_exec_args("gpt-5.5", None, &[], &extras, std::path::Path::new(""));
        let count = args.windows(2).filter(|w| w[0] == "--add-dir").count();
        assert_eq!(count, 2, "one --add-dir per additional root");
        assert!(
            args.windows(2)
                .any(|w| w == ["--add-dir", "/tmp/sibling-a"])
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--add-dir", "/tmp/sibling-b"])
        );
    }

    #[test]
    fn test_codex_exec_args_skips_empty_additional_root() {
        let extras = [std::path::PathBuf::new()];
        let args = codex_exec_args("gpt-5.5", None, &[], &extras, std::path::Path::new(""));
        assert!(!args.iter().any(|a| a == "--add-dir"));
    }

    #[test]
    fn test_codex_exec_args_forwards_synthesized_mcp_servers_as_config_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            r#"
[mcp_servers.gaviero]
command = "gaviero-mcp-shim"
args = ["--socket", "/tmp/mcp.sock"]

[mcp_servers.semantic-scholar]
url = "https://example/mcp/"
"#,
        )
        .unwrap();
        let args = codex_exec_args("gpt-5.5", None, &[], &[], dir.path());
        // Each server table is replayed as `--config mcp_servers.X.Y=value` pairs.
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--config" && w[1] == r#"mcp_servers.gaviero.command="gaviero-mcp-shim""#),
            "missing gaviero.command override in {args:?}",
        );
        assert!(
            args.windows(2).any(|w| w[0] == "--config"
                && w[1] == r#"mcp_servers.gaviero.args=["--socket", "/tmp/mcp.sock"]"#),
            "missing gaviero.args override in {args:?}",
        );
        assert!(
            args.windows(2).any(|w| w[0] == "--config"
                && w[1] == r#"mcp_servers.semantic-scholar.url="https://example/mcp/""#),
            "missing semantic-scholar.url override in {args:?}",
        );
    }

    #[test]
    fn test_codex_exec_args_bypasses_approvals_when_remote_mcp_url_present() {
        // codex 0.131.0 (verified live, 2026-06-03): every standard approval
        // policy auto-cancels MCP tool calls with `user cancelled MCP tool
        // call` in `codex exec`. Remote MCP requires the documented
        // `--dangerously-bypass-approvals-and-sandbox` escape hatch, which
        // also gives the worktree the network access HTTP MCP needs — so
        // we drop the prior `--sandbox workspace-write` + `network_access`
        // upgrade in favour of the single bypass flag.
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.semantic-scholar]\nurl = \"https://example/mcp/\"\n",
        )
        .unwrap();
        let args = codex_exec_args("gpt-5.5", None, &[], &[], dir.path());
        assert!(
            args.iter()
                .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"),
            "expected bypass flag in {args:?}",
        );
        // The old per-MCP sandbox knobs are now redundant — the bypass flag
        // covers both approvals and sandbox in one move.
        assert!(
            !args.windows(2).any(|w| w == ["--sandbox", "workspace-write"]),
            "stale --sandbox workspace-write override leaked into {args:?}",
        );
        assert!(
            !args.iter().any(|a| a == "approval_policy=never"),
            "stale approval_policy=never leaked into {args:?}",
        );
    }

    #[test]
    fn test_codex_exec_args_bypasses_approvals_for_stdio_only_mcp() {
        // The cancellation symptom also bites stdio MCP servers (gaviero
        // shim, context7), not just remote URLs — `codex exec` doesn't
        // distinguish. Any `[mcp_servers.X]` entry triggers the bypass.
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.gaviero]\ncommand = \"gaviero-mcp-shim\"\nargs = [\"--socket\", \"/tmp/mcp.sock\"]\n",
        )
        .unwrap();
        let args = codex_exec_args("gpt-5.5", None, &[], &[], dir.path());
        assert!(
            args.iter()
                .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"),
            "expected bypass flag for stdio MCP in {args:?}",
        );
        assert!(
            !args.windows(2).any(|w| w == ["--sandbox", "read-only"]),
            "stale --sandbox read-only override leaked into {args:?}",
        );
    }

    #[test]
    fn test_codex_exec_args_skips_mcp_overrides_when_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        let args = codex_exec_args("gpt-5.5", None, &[], &[], dir.path());
        assert!(
            !args.iter().any(|a| a.starts_with("mcp_servers.")),
            "expected no mcp_servers overrides when synth file is absent, got {args:?}",
        );
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

    #[test]
    fn small_prompt_passes_via_argv() {
        assert!(!would_use_stdin(0));
        assert!(!would_use_stdin(1_000));
        assert!(!would_use_stdin(ARGV_THRESHOLD - 1));
    }

    #[test]
    fn large_prompt_passes_via_stdin() {
        assert!(would_use_stdin(ARGV_THRESHOLD));
        assert!(would_use_stdin(100_000));
        assert!(would_use_stdin(1_000_000));
    }
}
