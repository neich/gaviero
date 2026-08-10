//! Codex CLI subprocess backend.
//!
//! Implements [`AgentBackend`] by spawning OpenAI's `codex exec --json`
//! non-interactive subprocess and mapping its JSONL event stream into the
//! unified [`UnifiedStreamEvent`] stream. File changes are proposed via
//! `<file>` blocks in the response text (detected and routed through the Write
//! Gate), matching the pattern used by the Claude Code backend.
//!
//! **Why `--json` is mandatory here.** Without it, `codex exec` splits its
//! output: stdout carries *only the final agent message*, emitted after the
//! turn ends, while the entire human-readable progress log (banner, echoed
//! prompt, `exec <command>` lines, command output, token counts) goes to
//! **stderr**. Reading stdout alone therefore produces no observable activity
//! until the turn completes — the parity contract in
//! `agent_session/mod.rs` requires the opposite. Forwarding stderr instead is
//! not an option: it echoes the whole enriched prompt back, which would land
//! in the chat panel and in the `<turn_annotations>` memory extractor.
//! `--json` puts structured, per-item events on stdout and leaves stderr for
//! real diagnostics. Verified against `codex-cli 0.146.0`.
//!
//! Granularity is per *item* (a whole message, a whole command), not
//! token-level deltas. Token-level streaming exists only in the `app-server`
//! protocol — see [`crate::agent_session::codex_app_server`].
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

use std::collections::HashSet;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use futures::Stream;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_stream::wrappers::ReceiverStream;

use crate::acp::protocol::find_next_file_block;

use super::shared::{build_enriched_prompt, default_editor_system_prompt};
use super::{
    AgentBackend, Capabilities, CompletionRequest, RetrievalToolset, StopReason, TokenUsage,
    UnifiedStreamEvent,
};

const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// Whether a prompt of `len` bytes should be piped via stdin rather
/// than passed as a positional argv. The threshold
/// ([`crate::util::spawn::argv_threshold`]) leaves comfortable headroom
/// for the rest of the argv (flags, `--config` keys, `--add-dir` roots)
/// and matches the ACP/Claude path for symmetry. Extracted so tests can
/// exercise the decision without spawning a subprocess.
fn would_use_stdin(len: usize) -> bool {
    len >= crate::util::spawn::argv_threshold()
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

        let mut cmd = crate::util::spawn::agent_command("codex");
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
                Ok(outcome) => {
                    // `turn.failed` is authoritative; an `error` item is not
                    // (codex emits those for warnings too, then exits 0).
                    let exited_ok = exit_status.as_ref().map(|s| s.success()).unwrap_or(false);
                    let ok = exited_ok && outcome.turn_failed.is_none();
                    if ok {
                        for diagnostic in &outcome.diagnostics {
                            tracing::warn!(
                                target: "backend.codex",
                                %diagnostic,
                                "codex reported a non-fatal error item"
                            );
                        }
                        if outcome.saw_json && !outcome.saw_visible_text {
                            tracing::warn!(
                                target: "backend.codex",
                                stderr = %stderr_text.trim(),
                                "codex exec --json produced no assistant text — the event schema \
                                 may have changed; expected `item.completed` with \
                                 item.type=\"agent_message\""
                            );
                        }
                        let mut usage = outcome.usage.unwrap_or_default();
                        usage.duration_ms = duration_ms;
                        let _ = tx_clone.send(Ok(UnifiedStreamEvent::Usage(usage))).await;
                        let _ = tx_clone
                            .send(Ok(UnifiedStreamEvent::Done(StopReason::EndTurn)))
                            .await;
                    } else {
                        let msg =
                            format_exit_error(&exit_status, &merge_diagnostics(&outcome, &stderr_text));
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
            // PUSH→PULL Phase 1: the gaviero MCP server is wired for Codex
            // (.codex/config.toml via config_synth), so retrieval tools are live.
            retrieval: RetrievalToolset {
                graph_and_memory: true,
                symbols: false,
            },
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn health_check(&self) -> Result<()> {
        let output = crate::util::spawn::agent_command("codex")
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
        // Structured event stream on stdout. Without this, stdout carries only
        // the final message and every progress event goes to stderr — see the
        // module docs. The whole observability contract depends on this flag.
        "--json".to_string(),
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

    if let Some(codex_effort) = map_effort_to_codex(effort, model) {
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

/// Whether an `item.*` event describes a starting or a finished item.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemPhase {
    Started,
    Completed,
}

/// What the stdout drain learned beyond the events it already forwarded.
#[derive(Debug, Default)]
struct CodexStdoutOutcome {
    /// Token counts from `turn.completed`. `duration_ms` is filled in by the
    /// caller, which owns the wall clock.
    usage: Option<TokenUsage>,
    /// `turn.failed` — an unambiguous, authoritative failure signal.
    turn_failed: Option<String>,
    /// `error` items. These are *not* authoritative: codex also uses them for
    /// non-fatal warnings (e.g. the under-development feature notice), and the
    /// turn can still complete with exit code 0. Surfaced through the exit-code
    /// path or logged, never bailed on directly.
    diagnostics: Vec<String>,
    /// Whether any visible assistant text was produced. Used to detect a
    /// silent schema mismatch against a future/older codex build.
    saw_visible_text: bool,
    /// Whether any line parsed as JSON at all.
    saw_json: bool,
}

/// Incremental `codex exec --json` JSONL parser.
///
/// Split from the IO loop so the event mapping is unit-testable without
/// spawning a subprocess. Owns the running assistant text because `<file>`
/// block detection scans across item boundaries.
#[derive(Default)]
struct CodexJsonParser {
    /// Concatenated visible assistant text, in emission order.
    full_text: String,
    file_scan_pos: usize,
    /// Item ids already announced via `ToolCallStart`, so an item that only
    /// reports `item.completed` still produces a start/end pair.
    started_tools: HashSet<String>,
    outcome: CodexStdoutOutcome,
}

impl CodexJsonParser {
    /// Map one stdout line to zero or more stream events.
    fn push_line(&mut self, line: &str) -> Vec<UnifiedStreamEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            // Not JSON. Either a codex build that ignored `--json`, or stray
            // output interleaved on stdout. Treat it as visible text rather
            // than dropping it — degrading to the pre-`--json` behavior beats
            // showing the user nothing.
            let mut out = Vec::new();
            self.emit_text(line, &mut out);
            return out;
        };

        self.outcome.saw_json = true;
        let mut out = Vec::new();
        match value.get("type").and_then(Value::as_str).unwrap_or_default() {
            "item.started" => self.push_item(&value, ItemPhase::Started, &mut out),
            "item.completed" => self.push_item(&value, ItemPhase::Completed, &mut out),
            "turn.completed" => {
                self.outcome.usage = Some(parse_usage(value.get("usage")));
            }
            "turn.failed" => {
                self.outcome.turn_failed = Some(
                    error_message(value.get("error"))
                        .unwrap_or_else(|| "codex turn failed".to_string()),
                );
            }
            // Lifecycle events with nothing to show. `thread.started` carries
            // a `thread_id`; `codex exec` is stateless per turn so gaviero has
            // nowhere to persist it (the app-server session owns continuity).
            "thread.started" | "turn.started" | "item.updated" => {}
            other => {
                tracing::debug!(
                    target: "backend.codex",
                    event = other,
                    "unhandled codex event type"
                );
            }
        }
        out
    }

    fn push_item(&mut self, value: &Value, phase: ItemPhase, out: &mut Vec<UnifiedStreamEvent>) {
        let Some(item) = value.get("item") else {
            return;
        };
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match item_type {
            // Assistant prose. A turn can contain several (a preamble before a
            // command, then the answer); only the completed form carries text.
            "agent_message" => {
                if phase == ItemPhase::Completed
                    && let Some(text) = item_text(item)
                {
                    self.emit_text(&text, out);
                }
            }
            "reasoning" => {
                if phase == ItemPhase::Completed
                    && let Some(text) = item_text(item)
                {
                    out.push(UnifiedStreamEvent::ThinkingDelta(text));
                }
            }
            // Non-fatal by default — collected, not raised. See
            // `CodexStdoutOutcome::diagnostics`.
            "error" => {
                if let Some(msg) = error_message(Some(item)) {
                    self.outcome.diagnostics.push(msg);
                }
            }
            // Shell commands map onto the Bash tool so `format_tool_summary`
            // renders the argv the same way it does for every other provider.
            "command_execution" => {
                let command = item
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.emit_tool(
                    &id,
                    "Bash",
                    serde_json::json!({ "command": command }),
                    phase,
                    out,
                );
            }
            // Anything else that isn't prose is activity worth showing. Naming
            // the tool after the item type means a future codex tool surfaces
            // as "Using <type>..." instead of vanishing.
            other => {
                let name = mcp_tool_name(item).unwrap_or_else(|| other.to_string());
                self.emit_tool(&id, &name, item.clone(), phase, out);
            }
        }
    }

    /// Emit `ToolCallStart` / `ToolCallEnd`, synthesizing the start when an
    /// item reports only its completion.
    fn emit_tool(
        &mut self,
        id: &str,
        name: &str,
        args: Value,
        phase: ItemPhase,
        out: &mut Vec<UnifiedStreamEvent>,
    ) {
        if !self.started_tools.contains(id) {
            self.started_tools.insert(id.to_string());
            out.push(UnifiedStreamEvent::ToolCallStart {
                id: id.to_string(),
                name: name.to_string(),
                args,
            });
        }
        if phase == ItemPhase::Completed {
            out.push(UnifiedStreamEvent::ToolCallEnd { id: id.to_string() });
        }
    }

    /// Append visible text and drain any `<file>` blocks it completed.
    ///
    /// Blocks are scanned against the accumulated text, not the individual
    /// item, so `file_scan_pos` stays meaningful across messages.
    fn emit_text(&mut self, text: &str, out: &mut Vec<UnifiedStreamEvent>) {
        if text.is_empty() {
            return;
        }
        // Separate consecutive messages so they don't run together in chat and
        // so a `<file>` block can't be glued onto the previous message's tail.
        let chunk = if self.full_text.is_empty() || self.full_text.ends_with('\n') {
            text.to_string()
        } else {
            format!("\n\n{text}")
        };
        self.full_text.push_str(&chunk);
        self.outcome.saw_visible_text = true;
        out.push(UnifiedStreamEvent::TextDelta(chunk));

        while let Some((path, content, end)) =
            find_next_file_block(&self.full_text, self.file_scan_pos)
        {
            self.file_scan_pos = end;
            out.push(UnifiedStreamEvent::FileBlock { path, content });
        }
    }
}

/// Text payload of an item, tolerating the `text` / `summary` spellings codex
/// uses for prose and reasoning respectively.
fn item_text(item: &Value) -> Option<String> {
    for key in ["text", "summary", "message"] {
        match item.get(key) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            // Reasoning summaries can arrive as a list of paragraphs.
            Some(Value::Array(parts)) => {
                let joined = parts
                    .iter()
                    .filter_map(|p| {
                        p.as_str()
                            .map(str::to_string)
                            .or_else(|| p.get("text").and_then(Value::as_str).map(str::to_string))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !joined.is_empty() {
                    return Some(joined);
                }
            }
            _ => {}
        }
    }
    None
}

/// Pull a human-readable message out of an error-shaped value.
fn error_message(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    for key in ["message", "error", "text"] {
        if let Some(s) = value.get(key).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

/// `server.tool` label for an MCP tool call, when the item carries one.
fn mcp_tool_name(item: &Value) -> Option<String> {
    let tool = item.get("tool").and_then(Value::as_str)?;
    match item.get("server").and_then(Value::as_str) {
        Some(server) => Some(format!("{server}.{tool}")),
        None => Some(tool.to_string()),
    }
}

/// `turn.completed.usage` → [`TokenUsage`]. `input_tokens` is the total codex
/// reports (cached input included); `duration_ms` is filled in by the caller.
fn parse_usage(usage: Option<&Value>) -> TokenUsage {
    let get = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    TokenUsage {
        input_tokens: get("input_tokens"),
        output_tokens: get("output_tokens"),
        cost_usd: None,
        duration_ms: None,
    }
}

/// Read `codex exec --json` stdout line by line and forward mapped events.
async fn drive_codex_stdout(
    stdout: tokio::process::ChildStdout,
    tx: tokio::sync::mpsc::Sender<Result<UnifiedStreamEvent>>,
) -> Result<CodexStdoutOutcome> {
    let mut lines = BufReader::new(stdout).lines();
    let mut parser = CodexJsonParser::default();

    loop {
        // Racing `closed()` against the read means a dropped stream (dispatch
        // budget, cancel) ends this task immediately, so `kill_on_drop` reaps
        // the codex child. Waiting for the next line would never return when
        // the subprocess has gone silent — the case the budget exists for.
        let line = tokio::select! {
            biased;
            _ = tx.closed() => return Ok(parser.outcome),
            l = lines.next_line() => l.context("reading codex stdout")?,
        };
        let Some(line) = line else { break };
        for event in parser.push_line(&line) {
            if tx.send(Ok(event)).await.is_err() {
                return Ok(parser.outcome); // receiver dropped
            }
        }
    }

    Ok(parser.outcome)
}

/// Map the DSL's provider-neutral `effort` vocabulary into Codex's
/// `model_reasoning_effort` config value.
///
/// Gaviero accepts `off`, `auto`, `minimal`, `low`, `medium`, `high`,
/// `xhigh`, `max`, `ultra`. `None` / `off` / `auto` omit the flag so Codex
/// uses its model default.
///
/// Supported ceilings follow Codex `models.json` (2026-08):
/// * `gpt-5.6-sol` / `gpt-5.6-terra` / bare `gpt-5.6` → up to `ultra`
/// * `gpt-5.6-luna` → up to `max` (no `ultra`)
/// * older models (`gpt-5.5`, `gpt-5.4`, `gpt-5.2`, …) → up to `xhigh`
///
/// Requests above the active model's ceiling are clamped down (not dropped).
fn map_effort_to_codex(effort: Option<&str>, model: &str) -> Option<&'static str> {
    let requested = match effort?.trim().to_ascii_lowercase().as_str() {
        "off" | "auto" | "" => return None,
        "minimal" => "minimal",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" => "xhigh",
        "max" => "max",
        "ultra" => "ultra",
        other => {
            tracing::warn!(
                target: "backend.codex",
                effort = other,
                "unknown effort value; not forwarding to codex (supported: minimal|low|medium|high|xhigh|max|ultra|off|auto)"
            );
            return None;
        }
    };
    Some(clamp_codex_effort(requested, model))
}

/// Highest `model_reasoning_effort` the given Codex model advertises.
fn codex_effort_ceiling(model: &str) -> &'static str {
    let m = model
        .trim()
        .strip_prefix("codex:")
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase();
    if m == "gpt-5.6" || m.starts_with("gpt-5.6-sol") || m.starts_with("gpt-5.6-terra") {
        "ultra"
    } else if m.starts_with("gpt-5.6-luna") {
        "max"
    } else if m.starts_with("gpt-5.6") {
        // Unknown 5.6 variant — allow the common Sol/Terra ceiling.
        "ultra"
    } else {
        // gpt-5.5 / gpt-5.4 / gpt-5.2 / legacy: Codex lists through xhigh.
        "xhigh"
    }
}

fn codex_effort_rank(effort: &str) -> u8 {
    match effort {
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        "ultra" => 7,
        _ => 0,
    }
}

fn clamp_codex_effort(requested: &'static str, model: &str) -> &'static str {
    let ceiling = codex_effort_ceiling(model);
    if codex_effort_rank(requested) > codex_effort_rank(ceiling) {
        ceiling
    } else {
        requested
    }
}

/// Combine the in-band failure signals with stderr for the error message.
///
/// `turn.failed` and `error` items are the only place codex explains a
/// model-side failure; stderr covers process-level ones (missing auth, bad
/// flags). A failing turn can have either, so report both.
fn merge_diagnostics(outcome: &CodexStdoutOutcome, stderr_text: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(failure) = outcome.turn_failed.as_deref() {
        parts.push(failure);
    }
    parts.extend(outcome.diagnostics.iter().map(String::as_str));
    let trimmed_stderr = stderr_text.trim();
    if !trimmed_stderr.is_empty() {
        parts.push(trimmed_stderr);
    }
    parts.join("\n")
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
        assert_eq!(map_effort_to_codex(Some("low"), "gpt-5.5"), Some("low"));
        assert_eq!(map_effort_to_codex(Some("medium"), "gpt-5.5"), Some("medium"));
        assert_eq!(map_effort_to_codex(Some("high"), "gpt-5.5"), Some("high"));
        assert_eq!(map_effort_to_codex(Some("minimal"), "gpt-5.5"), Some("minimal"));
        assert_eq!(map_effort_to_codex(Some("xhigh"), "gpt-5.5"), Some("xhigh"));
    }

    #[test]
    fn test_map_effort_to_codex_gpt56_passes_max_and_ultra() {
        assert_eq!(map_effort_to_codex(Some("xhigh"), "gpt-5.6-sol"), Some("xhigh"));
        assert_eq!(map_effort_to_codex(Some("max"), "gpt-5.6-sol"), Some("max"));
        assert_eq!(map_effort_to_codex(Some("ultra"), "gpt-5.6-sol"), Some("ultra"));
        assert_eq!(map_effort_to_codex(Some("ultra"), "gpt-5.6-terra"), Some("ultra"));
        assert_eq!(map_effort_to_codex(Some("ultra"), "gpt-5.6"), Some("ultra"));
        assert_eq!(map_effort_to_codex(Some("max"), "codex:gpt-5.6-luna"), Some("max"));
    }

    #[test]
    fn test_map_effort_to_codex_clamps_to_model_ceiling() {
        // Older models top out at xhigh.
        assert_eq!(map_effort_to_codex(Some("max"), "gpt-5.5"), Some("xhigh"));
        assert_eq!(map_effort_to_codex(Some("ultra"), "gpt-5.5"), Some("xhigh"));
        // Luna advertises through max, not ultra.
        assert_eq!(map_effort_to_codex(Some("ultra"), "gpt-5.6-luna"), Some("max"));
    }

    #[test]
    fn test_map_effort_to_codex_off_and_auto_omit() {
        assert_eq!(map_effort_to_codex(Some("off"), "gpt-5.6-sol"), None);
        assert_eq!(map_effort_to_codex(Some("auto"), "gpt-5.6-sol"), None);
        assert_eq!(map_effort_to_codex(None, "gpt-5.6-sol"), None);
    }

    #[test]
    fn test_map_effort_to_codex_case_insensitive() {
        assert_eq!(map_effort_to_codex(Some("HIGH"), "gpt-5.6-sol"), Some("high"));
        assert_eq!(map_effort_to_codex(Some("Medium"), "gpt-5.6-sol"), Some("medium"));
        assert_eq!(map_effort_to_codex(Some("ULTRA"), "gpt-5.6-sol"), Some("ultra"));
    }

    #[test]
    fn test_map_effort_to_codex_unknown_omitted() {
        assert_eq!(map_effort_to_codex(Some("turbo"), "gpt-5.6-sol"), None);
    }

    #[test]
    fn test_codex_exec_args_forwards_gpt56_ultra_effort() {
        let args = codex_exec_args(
            "gpt-5.6-sol",
            Some("ultra"),
            &[],
            &[],
            std::path::Path::new(""),
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--config", "model_reasoning_effort=ultra"]),
            "expected model_reasoning_effort=ultra, got {args:?}"
        );
    }

    /// Drive the parser over a transcript and collect everything it emitted.
    fn parse_lines(lines: &[&str]) -> (Vec<UnifiedStreamEvent>, CodexStdoutOutcome) {
        let mut parser = CodexJsonParser::default();
        let mut events = Vec::new();
        for line in lines {
            events.extend(parser.push_line(line));
        }
        (events, parser.outcome)
    }

    fn text_of(events: &[UnifiedStreamEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                UnifiedStreamEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_codex_exec_args_request_json_event_stream() {
        // The observability contract depends on this flag: without it stdout
        // carries only the final message and all progress goes to stderr.
        let args = codex_exec_args("gpt-5.5", None, &[], &[], std::path::Path::new(""));
        assert!(args.iter().any(|a| a == "--json"), "missing --json in {args:?}");
        // Must be an argument of the `exec` subcommand, not the top-level one.
        let exec_pos = args.iter().position(|a| a == "exec").expect("exec subcommand");
        let json_pos = args.iter().position(|a| a == "--json").expect("--json");
        assert!(json_pos > exec_pos, "--json must follow `exec` in {args:?}");
    }

    #[test]
    fn parses_live_transcript_into_text_tool_and_usage_events() {
        // Verbatim stdout from `codex exec --json` (codex-cli 0.146.0), with
        // the long Windows command elided. Pins the real schema so a codex
        // upgrade that changes it fails here rather than silently in chat.
        let (events, outcome) = parse_lines(&[
            r#"{"type":"thread.started","thread_id":"019fc78f-c957-7d41-83a6-ffbcb903ec65"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"I'll count the Rust source files."}}"#,
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"pwsh -Command ls"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","exit_code":0}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"There are 4 files."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":26571,"cached_input_tokens":11008,"output_tokens":121}}"#,
        ]);

        // Both assistant messages surface, separated so they don't run together.
        assert_eq!(
            text_of(&events),
            "I'll count the Rust source files.\n\nThere are 4 files."
        );

        // The shell command becomes a Bash tool call — this is what drives the
        // chat panel's "Using Bash..." indicator, which the pre-`--json`
        // backend could never emit.
        assert!(
            events.iter().any(|e| matches!(
                e,
                UnifiedStreamEvent::ToolCallStart { name, args, .. }
                    if name == "Bash"
                        && args.get("command").and_then(|c| c.as_str())
                            == Some("pwsh -Command ls")
            )),
            "expected a Bash ToolCallStart, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UnifiedStreamEvent::ToolCallEnd { id } if id == "item_1"))
        );

        // Real token counts — the old backend hardcoded zeros.
        let usage = outcome.usage.expect("turn.completed carries usage");
        assert_eq!(usage.input_tokens, 26571);
        assert_eq!(usage.output_tokens, 121);
        assert!(outcome.saw_visible_text);
        assert!(outcome.turn_failed.is_none());
    }

    #[test]
    fn file_blocks_are_extracted_from_decoded_message_text() {
        // The block lives inside a JSON-escaped string, so scanning the raw
        // JSONL line would never match. Regression guard for the Write Gate.
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_0",
                "type": "agent_message",
                "text": "Here you go:\n<file path=\"src/lib.rs\">fn main() {}\n</file>\nDone."
            }
        })
        .to_string();
        let (events, _) = parse_lines(&[&line]);

        let blocks: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                UnifiedStreamEvent::FileBlock { path, content } => Some((path, content)),
                _ => None,
            })
            .collect();
        assert_eq!(blocks.len(), 1, "expected one file block in {events:?}");
        assert_eq!(blocks[0].0, &std::path::PathBuf::from("src/lib.rs"));
        // `find_next_file_block` drops the newline before the closing tag.
        assert_eq!(blocks[0].1, "fn main() {}");
    }

    #[test]
    fn file_block_split_across_two_messages_is_still_detected() {
        // Scanning is against the accumulated text, not the individual item,
        // so a block opened in one message and closed in the next resolves.
        let (events, _) = parse_lines(&[
            r#"{"type":"item.completed","item":{"id":"a","type":"agent_message","text":"<file path=\"a.txt\">first"}}"#,
            r#"{"type":"item.completed","item":{"id":"b","type":"agent_message","text":"second</file>"}}"#,
        ]);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UnifiedStreamEvent::FileBlock { path, .. }
                    if path == &std::path::PathBuf::from("a.txt"))),
            "expected a file block spanning both messages, got {events:?}"
        );
    }

    #[test]
    fn reasoning_items_map_to_thinking_deltas() {
        let (events, _) = parse_lines(&[
            r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"weighing options"}}"#,
        ]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                UnifiedStreamEvent::ThinkingDelta(t) if t == "weighing options"
            )),
            "reasoning must not surface as visible text: {events:?}"
        );
        assert!(text_of(&events).is_empty());
    }

    #[test]
    fn reasoning_summary_array_is_joined() {
        let (events, _) = parse_lines(&[
            r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","summary":["first","second"]}}"#,
        ]);
        assert!(events.iter().any(|e| matches!(
            e,
            UnifiedStreamEvent::ThinkingDelta(t) if t == "first\nsecond"
        )));
    }

    #[test]
    fn error_items_are_collected_not_raised() {
        // codex emits `error` items for warnings (e.g. the under-development
        // feature notice) and still exits 0. Raising UnifiedStreamEvent::Error
        // would make `complete_to_write_gate` bail on a successful turn.
        let (events, outcome) = parse_lines(&[
            r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Under-development features enabled: x."}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"done"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":2}}"#,
        ]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, UnifiedStreamEvent::Error(_))),
            "a non-fatal error item must not become an Error event: {events:?}"
        );
        assert_eq!(outcome.diagnostics.len(), 1);
        assert!(outcome.turn_failed.is_none());
        assert_eq!(text_of(&events), "done");
    }

    #[test]
    fn turn_failed_is_recorded_as_authoritative_failure() {
        let (_, outcome) = parse_lines(&[
            r#"{"type":"turn.failed","error":{"message":"context window exceeded"}}"#,
        ]);
        assert_eq!(
            outcome.turn_failed.as_deref(),
            Some("context window exceeded")
        );
    }

    #[test]
    fn merge_diagnostics_reports_turn_failure_items_and_stderr() {
        let outcome = CodexStdoutOutcome {
            turn_failed: Some("turn blew up".into()),
            diagnostics: vec!["bad tool".into()],
            ..Default::default()
        };
        let merged = merge_diagnostics(&outcome, "  auth failure\n");
        assert!(merged.contains("turn blew up"));
        assert!(merged.contains("bad tool"));
        assert!(merged.contains("auth failure"));
    }

    #[test]
    fn non_json_stdout_falls_back_to_visible_text() {
        // A codex build that ignores `--json` must still show *something*
        // rather than silently producing an empty turn.
        let (events, outcome) = parse_lines(&["I saw 36 entries."]);
        assert_eq!(text_of(&events), "I saw 36 entries.");
        assert!(outcome.saw_visible_text);
        assert!(!outcome.saw_json);
    }

    #[test]
    fn unknown_item_types_surface_as_tool_activity() {
        // A future codex tool should appear as activity, not vanish. The item
        // reports only `item.completed`, so the start is synthesized.
        let (events, _) = parse_lines(&[
            r#"{"type":"item.completed","item":{"id":"w1","type":"web_search","query":"rust"}}"#,
        ]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                UnifiedStreamEvent::ToolCallStart { name, .. } if name == "web_search"
            )),
            "expected synthesized ToolCallStart, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UnifiedStreamEvent::ToolCallEnd { id } if id == "w1"))
        );
    }

    #[test]
    fn mcp_tool_calls_are_named_server_dot_tool() {
        let (events, _) = parse_lines(&[
            r#"{"type":"item.started","item":{"id":"m1","type":"mcp_tool_call","server":"gaviero","tool":"memory_search"}}"#,
        ]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                UnifiedStreamEvent::ToolCallStart { name, .. } if name == "gaviero.memory_search"
            )),
            "got {events:?}"
        );
    }

    #[test]
    fn started_then_completed_emits_a_single_tool_start() {
        let (events, _) = parse_lines(&[
            r#"{"type":"item.started","item":{"id":"c1","type":"command_execution","command":"ls"}}"#,
            r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","exit_code":0}}"#,
        ]);
        let starts = events
            .iter()
            .filter(|e| matches!(e, UnifiedStreamEvent::ToolCallStart { .. }))
            .count();
        assert_eq!(starts, 1, "duplicate tool start in {events:?}");
    }

    #[test]
    fn lifecycle_and_malformed_events_are_ignored_without_output() {
        let (events, outcome) = parse_lines(&[
            r#"{"type":"thread.started","thread_id":"t1"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.updated","item":{"id":"x","type":"agent_message","text":"partial"}}"#,
            r#"{"type":"future.event","payload":{}}"#,
            r#"{"type":"item.completed"}"#,
            "",
        ]);
        assert!(events.is_empty(), "unexpected events: {events:?}");
        assert!(outcome.saw_json);
        assert!(!outcome.saw_visible_text);
    }

    #[test]
    fn missing_usage_yields_zeroed_counts() {
        let (_, outcome) = parse_lines(&[r#"{"type":"turn.completed"}"#]);
        let usage = outcome.usage.expect("usage present even when absent in JSON");
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
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
        assert!(!would_use_stdin(crate::util::spawn::argv_threshold() - 1));
    }

    #[test]
    fn large_prompt_passes_via_stdin() {
        assert!(would_use_stdin(crate::util::spawn::argv_threshold()));
        assert!(would_use_stdin(100_000));
        assert!(would_use_stdin(1_000_000));
    }
}
