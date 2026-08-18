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
use tokio::process::Child;

use super::protocol::{StreamEvent, parse_stream_line};
use crate::observer::{PromptEvent, PromptObserver};

/// If the enriched prompt + system prompt combined exceed
/// [`crate::util::spawn::argv_threshold`], pass the prompt to Claude through
/// a workspace-local tempfile via `@`-reference instead of argv. Linux
/// `ARG_MAX` is ~128 KB; Windows caps the whole command line at ~32,767
/// UTF-16 chars, hence the much lower Windows threshold. The tempfile path
/// itself has no practical size ceiling.
fn argv_threshold() -> usize {
    crate::util::spawn::argv_threshold()
}

/// Subdirectory under the workspace root where oversized prompt tempfiles live.
/// `--add-dir <cwd>` already lets Claude read files under the workspace; the
/// `.gaviero/tmp` subpath keeps these transient files out of the way of code.
const TEMP_SUBDIR: &str = ".gaviero/tmp";

/// Options for the Claude agent subprocess.
#[derive(Clone)]
pub struct AgentOptions {
    /// Effort level for the CLI (off, low, medium, high, xhigh, max, auto).
    /// "off" and "auto" mean don't pass --effort (use CLI / model default).
    /// `xhigh` applies on Opus 4.7; lower models fall back to `high`.
    /// `max` is session-only (deepest reasoning, no token cap).
    pub effort: String,
    /// Max output tokens (0 = use default). Reserved for future API-based backends.
    pub max_tokens: u32,
    /// When true, pass `--dangerously-skip-permissions` so the subprocess never
    /// pauses for permission prompts. Intended for single-prompt "yes to all" mode.
    pub auto_approve: bool,
    /// Tool surface offered to the subprocess via `--tools`. `None`
    /// keeps the legacy hardcoded list (`Read,Glob,Grep,Write,Edit,
    /// MultiEdit`). Hosts that read `agent.availableTools` from
    /// workspace settings populate this with the resolved value.
    pub available_tools: Option<Vec<String>>,
    /// Subset of `available_tools` auto-approved via `--allowedTools`.
    /// `None` keeps the legacy default (`Read,Glob,Grep`, or the full
    /// available set when `auto_approve` is true).
    pub approved_tools: Option<Vec<String>>,
    /// When `Some`, resume the Claude session with the given id (Claude's
    /// `--resume <id>` flag) so model context (prior messages, read file
    /// cache) carries across turns. When `None`, a fresh one-shot session
    /// is spawned.
    ///
    /// **Deprecated (M6).** `ClaudeSession` drives resume via
    /// `ContinuityHandle::ClaudeSessionId` stored in the `SessionLedger`.
    /// This field remains for `LegacyAgentSession` (Ollama/Codex) until M10
    /// cleanup; new per-provider sessions must not read it. Removal: M10.
    #[deprecated(
        since = "0.1.0",
        note = "M6: use ContinuityHandle::ClaudeSessionId via SessionLedger; removal in M10"
    )]
    pub resume_session_id: Option<String>,
    /// Test-only: when set, fires once per `AcpSession::spawn` with the
    /// exact prompt + system-prompt bytes that the runtime would
    /// otherwise hand to argv or spill to `.gaviero/tmp/prompt-*.md`.
    /// Production callers leave this `None` and pay nothing.
    pub prompt_observer: Option<Arc<dyn PromptObserver>>,
    /// Stable per-turn id threaded into [`PromptEvent::turn_id`]. The
    /// orchestrator owns generation; `None` propagates as an empty
    /// string, in which case the `RecordingPromptObserver` fallback
    /// (T1.2) substitutes a "current turn id" mutex.
    pub turn_id: Option<String>,
    /// When true, sets `CLAUDE_QUIET=1` on the subprocess so global
    /// Claude Code Stop/Notification hooks skip machine turns (memory
    /// extractor, swarm agents, etc.). Interactive chat leaves this false.
    pub suppress_hooks: bool,
}

impl std::fmt::Debug for AgentOptions {
    // M6: `resume_session_id` deprecated; keep visible in Debug for
    // diagnostics until M10 removal.
    #[allow(deprecated)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOptions")
            .field("effort", &self.effort)
            .field("max_tokens", &self.max_tokens)
            .field("auto_approve", &self.auto_approve)
            .field("available_tools", &self.available_tools)
            .field("approved_tools", &self.approved_tools)
            .field("resume_session_id", &self.resume_session_id)
            .field(
                "prompt_observer",
                &self.prompt_observer.as_ref().map(|_| "<set>"),
            )
            .field("turn_id", &self.turn_id)
            .field("suppress_hooks", &self.suppress_hooks)
            .finish()
    }
}

impl Default for AgentOptions {
    // M6: `resume_session_id` deprecated; allow here because `Default` must
    // initialize every field. Stays until M10 removes the field.
    #[allow(deprecated)]
    fn default() -> Self {
        Self {
            effort: "off".to_string(),
            max_tokens: 16384,
            auto_approve: false,
            available_tools: None,
            approved_tools: None,
            resume_session_id: None,
            prompt_observer: None,
            turn_id: None,
            suppress_hooks: false,
        }
    }
}

/// Hardcoded fallback tool surface for callers that haven't been
/// migrated to populate `AgentOptions::available_tools` from workspace
/// settings. Matches the pre-config behaviour: read-only browse + Write
/// Gate-routed edits, no shell.
pub const DEFAULT_AVAILABLE_TOOLS: &[&str] =
    &["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"];

/// Hardcoded fallback approved-tool subset (see `DEFAULT_AVAILABLE_TOOLS`).
pub const DEFAULT_APPROVED_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// Tool name Claude uses for clarifying multiple-choice questions.
/// Always injected into `--tools` for interactive chat so the model can
/// surface questions through the control protocol instead of narrating them.
pub const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";

/// Ensure `AskUserQuestion` is present in the available-tools list.
pub fn ensure_ask_user_question(tools: &mut Vec<String>) {
    if !tools.iter().any(|t| t == ASK_USER_QUESTION_TOOL) {
        tools.push(ASK_USER_QUESTION_TOOL.to_string());
    }
}

/// True for `agent.availableTools` entries naming an MCP server or tool
/// (`mcp__gaviero`, `mcp__context7__query_docs`) rather than a built-in.
///
/// Such names are valid in `--allowedTools` (they are permission rules) but
/// never in `--tools`, which selects from the built-in set only.
pub fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with("mcp__")
}

/// Build the `--tools` value for a Claude spawn from `agent.availableTools`.
///
/// Drops `mcp__…` entries — see [`is_mcp_tool_name`]; passing one costs the
/// session every MCP server — and injects `AskUserQuestion` for interactive
/// turns so clarifying questions ride the control channel.
pub fn build_available_tools(available: &[&str], interactive: bool) -> Vec<String> {
    let mut tools: Vec<String> = available
        .iter()
        .map(|s| (*s).to_string())
        .filter(|t| !is_mcp_tool_name(t))
        .collect();
    if interactive {
        ensure_ask_user_question(&mut tools);
    }
    tools
}

impl AgentOptions {
    /// Resolve the effective `(available, approved)` tool lists for the
    /// subprocess spawn. If `available_tools` is unset, falls back to
    /// [`DEFAULT_AVAILABLE_TOOLS`]. If `approved_tools` is unset, the
    /// approved list defaults to the full available set when
    /// `auto_approve` is true, otherwise to [`DEFAULT_APPROVED_TOOLS`]
    /// filtered to the available set.
    pub fn resolved_tools(&self) -> (Vec<String>, Vec<String>) {
        let available: Vec<String> = match self.available_tools.as_ref() {
            Some(list) => list.clone(),
            None => DEFAULT_AVAILABLE_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let approved: Vec<String> = match self.approved_tools.as_ref() {
            Some(list) => list
                .iter()
                .filter(|name| available.iter().any(|a| a == *name))
                .cloned()
                .collect(),
            None if self.auto_approve => available.clone(),
            None => DEFAULT_APPROVED_TOOLS
                .iter()
                .map(|s| s.to_string())
                .filter(|name| available.contains(name))
                .collect(),
        };
        (available, approved)
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
pub fn would_use_tempfile(prompt_len: usize, system_prompt_len: usize) -> bool {
    prompt_len + system_prompt_len >= argv_threshold()
}

/// Write `prompt` to a workspace-local tempfile and return (NamedTempFile,
/// short argv to use instead of the full prompt). The argv tells Claude to
/// read the file via its `@`-syntax and follow its instructions.
pub fn spill_prompt_to_tempfile(
    cwd: &Path,
    prompt: &str,
) -> Result<(tempfile::NamedTempFile, String)> {
    let dir: PathBuf = cwd.join(TEMP_SUBDIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating tempdir {}", dir.display()))?;

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
    ///
    /// `additional_roots` adds extra writable folders alongside `cwd` via
    /// repeated `--add-dir` flags. Used in workspace-mode multi-folder
    /// setups so Claude can read/write sibling folders, not just the
    /// primary cwd. Empty for single-folder workspaces and per-agent
    /// swarm worktrees.
    // M6: reads `options.resume_session_id` (deprecated); allow stays until M10.
    #[allow(deprecated)]
    pub fn spawn(
        model: &str,
        cwd: &Path,
        prompt: &str,
        system_prompt: &str,
        available_tools: &[&str],
        approved_tools: &[&str],
        options: &AgentOptions,
        file_attachments: &[&Path],
        additional_roots: &[&Path],
    ) -> Result<Self> {
        // Decide argv vs tempfile for the prompt. Small prompts take the
        // zero-overhead argv path; anything that might approach ARG_MAX is
        // spilled to a workspace-local `.gaviero/tmp/prompt-*.md` file and
        // referenced via `@path` so the argv stays tiny.
        let use_tempfile = would_use_tempfile(prompt.len(), system_prompt.len());

        // T1.1: fire the test-only PromptObserver once with the exact
        // bytes that would land in argv or `.gaviero/tmp/prompt-*.md`.
        // Symmetric across both branches; production leaves this `None`.
        if let Some(obs) = options.prompt_observer.as_ref() {
            obs.on_prompt(PromptEvent {
                turn_id: options.turn_id.clone().unwrap_or_default(),
                resume_session_id: options.resume_session_id.clone(),
                prompt: prompt.to_string(),
                system_prompt: system_prompt.to_string(),
                used_tempfile: use_tempfile,
                argv_threshold: argv_threshold(),
                captured_at: std::time::Instant::now(),
            });
        }

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

        let mut cmd = crate::util::spawn::agent_command("claude");
        // Interactive turns need the Agent SDK control channel
        // (`--permission-prompt-tool stdio` + stream-json stdin) so
        // unapproved tools / AskUserQuestion emit `control_request` events
        // the host can answer. Auto-approve keeps the classic `--print`
        // one-shot shape with `--dangerously-skip-permissions`.
        let interactive_permissions = !options.auto_approve;
        cmd.arg("--print")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages");
        if interactive_permissions {
            cmd.arg("--input-format").arg("stream-json");
            cmd.arg("--permission-prompt-tool").arg("stdio");
            cmd.arg("--permission-mode").arg("default");
            // Project/local Claude settings stay loaded on purpose. Gaviero
            // translates `mcp.permissions` *and* `agent.permissions.bash`
            // into `.claude/settings.json` (see
            // `mcp::config_synth::synthesize_for_worktree`), and that file
            // also carries the operator's own deny rules. Excluding it with
            // `--setting-sources ""` dropped every allow *and* deny rule, so
            // routine tools prompted on every turn while the safety denies
            // silently stopped applying. Claude treats `permissions.allow` as
            // "skip the prompt", never as a widening of the tool surface, so
            // it cannot override the hard limits `--tools` / `--mcp-config`
            // impose below.
        }
        cmd.arg("--model")
            // Resolve alias→concrete id (e.g. `sonnet` → Sonnet 5) so the
            // `sonnet` alias pins to a specific model rather than the CLI's
            // drifting "latest sonnet". Every Claude spawn funnels through here.
            .arg(crate::swarm::backend::shared::resolve_claude_cli_model(
                model,
            ));

        // Session reuse: when a prior session_id is known (captured from the
        // first turn's SystemInit event), resume it so Claude's model keeps
        // conversation context, read-file cache, and thinking state.
        //
        // M2 (borrowed scope from M6): turn 1 must NOT pass
        // `--no-session-persistence`. Doing so makes Claude discard the
        // conversation immediately; turn 2's `--resume <id>` then errors with
        // "No conversation found" — see [Finding G in baselines/m0.md]. The
        // borrowed-scope rationale is documented on the M2 PR. M6 owns the
        // rest of the Claude normalization (full session lifecycle, restart,
        // etc.); this is the minimum to unblock M2 acceptance.
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
        if let Some(id) = options.resume_session_id.as_deref()
            && !id.is_empty()
        {
            cmd.arg("--resume").arg(id);
        }

        if !options.effort.is_empty() && options.effort != "off" && options.effort != "auto" {
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

        // Interactive chat always exposes AskUserQuestion so clarifying
        // questions ride the same can_use_tool control channel as tool
        // approvals. Auto-approve / swarm paths leave the caller's list alone.
        //
        // `mcp__…` entries are stripped first: `--tools` selects from Claude's
        // *built-in* set only, and an unknown name there is not merely
        // ignored. Verified against Claude Code 2.1.220 — passing `--tools`
        // restricts the whole session and drops every MCP server with it
        // (35 tools + context7 reachable without the flag; 4 tools and no
        // context7 with it). MCP admission is governed by `.mcp.json` /
        // `--mcp-config` below, so listing servers here only costs the agent
        // its MCP tools.
        let available_owned = build_available_tools(available_tools, interactive_permissions);
        if !available_owned.is_empty() {
            cmd.arg("--tools").arg(available_owned.join(","));
        }

        // Load the per-worktree MCP config synthesized for this agent
        // (`<cwd>/.mcp.json`: the gaviero shim + context7 + any operator
        // `--mcp-url` / `--mcp-stdio` servers). In headless `--print` mode
        // Claude does NOT auto-load a project `.mcp.json` — that discovery
        // path requires interactive trust approval — so without this flag
        // swarm / worktree agents see ZERO MCP tools and report "no MCP
        // client capability". `--tools` only governs the built-in set, so it
        // does not bring MCP tools in; only `--mcp-config` does. MCP tools
        // are auto-approved by `--dangerously-skip-permissions` above when
        // `auto_approve` is set (otherwise they surface as permission
        // prompts, matching built-in tool behaviour). Non-strict on purpose:
        // the user's own global MCP servers still load alongside. Mirrors how
        // the Codex backend replays its synthesized `.codex/config.toml`.
        let mcp_config_path = cwd.join(".mcp.json");
        if mcp_config_path.is_file() {
            cmd.arg("--mcp-config").arg(&mcp_config_path);
        }

        // NOTE: Claude CLI's `--file` flag is for downloading remote file
        // resources (format `file_id:relative_path`), not for attaching
        // local images or documents. Passing local paths there fails with
        // "Session token required for file downloads". Local attachments
        // are mentioned inside the prompt body by `agent_session::claude`
        // (so the model invokes its `Read` tool) and their parent
        // directories are widened into `additional_roots` so the tool is
        // allowed to access them. `file_attachments` here is kept for log
        // parity but does not produce argv.
        let _ = file_attachments;

        cmd.arg("--add-dir").arg(cwd);

        // Workspace-mode multi-folder: each sibling folder is added as a
        // writable root. `--add-dir` is repeatable on the Claude CLI; the
        // first one (cwd above) anchors path resolution, additional ones
        // extend the writable scope so cross-folder edits don't fail.
        for extra in additional_roots {
            if extra.as_os_str().is_empty() || *extra == cwd {
                continue;
            }
            cmd.arg("--add-dir").arg(extra);
        }

        // Interactive turns: prompt goes on stdin as a stream-json user
        // message so the control channel stays bidirectional. Auto-approve
        // keeps the classic positional argv prompt (stdin still piped for
        // any late control traffic, but unused).
        if !interactive_permissions {
            cmd.arg("--").arg(&argv_prompt);
        }

        cmd.current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            // Without this, dropping `Child` (e.g. when the host task is
            // aborted on Ctrl+C) leaves an orphaned `claude` subprocess that
            // keeps issuing tool calls and editing files. Cancellation must be
            // transactional: kill the child, then revert tool snapshots.
            .kill_on_drop(true);

        if options.suppress_hooks {
            cmd.env("CLAUDE_QUIET", "1");
        }

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
                     This shouldn't happen — user prompts spill to a tempfile above {} B.\n\
                     The system prompt or flag arguments must be pathologically large; report this as a bug.",
                    argv_threshold()
                )
            } else {
                anyhow::anyhow!("spawning claude subprocess: {e}")
            }
        })?;

        // Keep stdin open for control_response / permission replies.
        // Interactive turns also enqueue the user prompt as the first
        // stream-json line. Dropping the sender closes stdin.
        let stdin_tx = if let Some(mut stdin) = child.stdin.take() {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            if interactive_permissions {
                let user_line = serde_json::json!({
                    "type": "user",
                    "message": { "role": "user", "content": argv_prompt },
                    "parent_tool_use_id": serde_json::Value::Null,
                })
                .to_string()
                    + "\n";
                let _ = tx.send(user_line);
            }
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

    /// Send a permission / AskUserQuestion decision back to the Claude
    /// subprocess via the control protocol.
    ///
    /// Prefer this over [`Self::respond_permission`] when the caller has the
    /// original tool `input` (needed so allow responses echo `updatedInput`).
    pub fn respond_permission_decision(
        &self,
        decision: &crate::observer::PermissionDecision,
        request_id: &str,
        original_input: &serde_json::Value,
    ) {
        let Some(ref tx) = self.stdin_tx else { return };
        let response_body = match decision {
            crate::observer::PermissionDecision::Allow { updated_input } => {
                let input = updated_input
                    .clone()
                    .unwrap_or_else(|| original_input.clone());
                serde_json::json!({
                    "behavior": "allow",
                    "updatedInput": input,
                })
            }
            crate::observer::PermissionDecision::Deny { message } => {
                serde_json::json!({
                    "behavior": "deny",
                    "message": message.as_deref().unwrap_or("Denied by user"),
                })
            }
        };
        let msg = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response_body,
            }
        })
        .to_string()
            + "\n";
        let _ = tx.send(msg);
    }

    /// Send a permission response back to the Claude subprocess via stdin.
    ///
    /// Convenience wrapper around [`Self::respond_permission_decision`] for
    /// callers that only have a boolean (legacy tests / swarm).
    pub fn respond_permission(&self, allow: bool, request_id: &str) {
        let decision = if allow {
            crate::observer::PermissionDecision::allow()
        } else {
            crate::observer::PermissionDecision::deny()
        };
        self.respond_permission_decision(
            &decision,
            request_id,
            &serde_json::Value::Object(Default::default()),
        );
    }

    /// Wait for the subprocess to exit and return its status.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        self.child
            .wait()
            .await
            .context("waiting for claude subprocess")
    }

    /// Return captured stderr lines (useful for diagnosing exit-without-output).
    pub async fn stderr_output(&self) -> String {
        let lines = self.stderr_buf.lock().await;
        lines.join("\n")
    }
}

/// Check if the `claude` CLI binary is available on PATH.
pub fn is_claude_available() -> bool {
    crate::util::spawn::agent_command_std("claude")
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
        // Right at the boundary — just below the threshold.
        assert!(!would_use_tempfile(argv_threshold() - 1, 0));
    }

    #[test]
    fn large_prompt_spills_to_tempfile() {
        assert!(would_use_tempfile(argv_threshold(), 0));
        assert!(would_use_tempfile(100_000, 0));
        // Combined prompt + system prompt crossing threshold.
        assert!(would_use_tempfile(argv_threshold() - 100, 200));
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

    #[test]
    fn resolved_tools_falls_back_to_legacy_defaults() {
        let opts = AgentOptions::default();
        let (available, approved) = opts.resolved_tools();
        assert_eq!(available, DEFAULT_AVAILABLE_TOOLS);
        assert_eq!(approved, DEFAULT_APPROVED_TOOLS);
    }

    #[test]
    fn resolved_tools_auto_approve_promotes_full_set_when_unset() {
        let opts = AgentOptions {
            auto_approve: true,
            ..AgentOptions::default()
        };
        let (available, approved) = opts.resolved_tools();
        assert_eq!(
            available, approved,
            "auto_approve approves everything available"
        );
    }

    #[test]
    fn build_available_tools_strips_mcp_names() {
        // `mcp__…` in `--tools` costs the session every MCP server
        // (Claude Code 2.1.220), so they must never reach the flag.
        let available = [
            "Read",
            "Glob",
            "Grep",
            "Write",
            "Edit",
            "MultiEdit",
            "Bash",
            "mcp__gaviero",
            "mcp__context7__query_docs",
        ];
        let tools = build_available_tools(&available, true);
        assert!(
            !tools.iter().any(|t| t.starts_with("mcp__")),
            "MCP names must not reach --tools: {tools:?}"
        );
        assert!(tools.contains(&"Bash".to_string()));
        assert!(tools.contains(&ASK_USER_QUESTION_TOOL.to_string()));
        assert_eq!(tools.len(), 8, "7 built-ins + AskUserQuestion");
    }

    #[test]
    fn build_available_tools_omits_ask_user_question_when_not_interactive() {
        let tools = build_available_tools(&["Read", "mcp__gaviero"], false);
        assert_eq!(tools, vec!["Read".to_string()]);
    }

    #[test]
    fn ensure_ask_user_question_injects_once() {
        let mut tools = vec!["Read".into(), "Bash".into()];
        ensure_ask_user_question(&mut tools);
        ensure_ask_user_question(&mut tools);
        assert_eq!(
            tools
                .iter()
                .filter(|t| *t == ASK_USER_QUESTION_TOOL)
                .count(),
            1
        );
        assert!(tools.ends_with(&[ASK_USER_QUESTION_TOOL.to_string()]));
    }

    #[test]
    fn control_response_allow_includes_updated_input() {
        let decision = crate::observer::PermissionDecision::Allow {
            updated_input: Some(serde_json::json!({"command": "echo hi"})),
        };
        let body = match &decision {
            crate::observer::PermissionDecision::Allow { updated_input } => {
                serde_json::json!({
                    "behavior": "allow",
                    "updatedInput": updated_input.clone().unwrap(),
                })
            }
            _ => unreachable!(),
        };
        let msg = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "req-1",
                "response": body,
            }
        });
        assert_eq!(msg["type"], "control_response");
        assert_eq!(msg["response"]["request_id"], "req-1");
        assert_eq!(msg["response"]["response"]["behavior"], "allow");
        assert_eq!(
            msg["response"]["response"]["updatedInput"]["command"],
            "echo hi"
        );
    }

    #[test]
    fn agent_options_size_is_bounded() {
        // Sanity: AgentOptions is cloned per turn. The two new fields
        // (Option<Arc<dyn _>>, Option<String>) must not balloon it past
        // a sensible budget. 256 B leaves slack for future extension.
        assert!(
            std::mem::size_of::<AgentOptions>() <= 256,
            "AgentOptions = {} B (budget 256 B)",
            std::mem::size_of::<AgentOptions>()
        );
    }

    #[derive(Default)]
    struct CapturingObserver {
        events: std::sync::Mutex<Vec<PromptEvent>>,
    }

    impl PromptObserver for CapturingObserver {
        fn on_prompt(&self, ev: PromptEvent) {
            self.events.lock().unwrap().push(ev);
        }
    }

    #[test]
    fn agent_options_default_observer_is_none() {
        let opts = AgentOptions::default();
        assert!(opts.prompt_observer.is_none());
        assert!(opts.turn_id.is_none());
    }

    #[test]
    fn agent_options_debug_does_not_leak_observer_pointer() {
        let obs: Arc<dyn PromptObserver> = Arc::new(CapturingObserver::default());
        let opts = AgentOptions {
            prompt_observer: Some(obs),
            turn_id: Some("turn-7".into()),
            ..AgentOptions::default()
        };
        let s = format!("{:?}", opts);
        assert!(s.contains("prompt_observer: Some(\"<set>\")"));
        assert!(s.contains("turn_id: Some(\"turn-7\")"));
    }

    #[test]
    fn resolved_tools_honours_explicit_available_list_with_bash() {
        let opts = AgentOptions {
            available_tools: Some(vec!["Read".into(), "Bash".into()]),
            approved_tools: Some(vec!["Read".into(), "Bash".into(), "Edit".into()]),
            ..AgentOptions::default()
        };
        let (available, approved) = opts.resolved_tools();
        assert_eq!(available, vec!["Read".to_string(), "Bash".to_string()]);
        // "Edit" silently dropped — not in the available set.
        assert_eq!(approved, vec!["Read".to_string(), "Bash".to_string()]);
    }
}

/// Query `claude --help` for the model options documented by the CLI.
///
/// Parses the `--model` flag description and extracts all single-quoted model
/// ids (aliases and full names), then prefixes them with `claude:` so callers
/// receive the canonical `provider:model` form. Returns an empty Vec if the
/// CLI is unavailable or the help text format changes — the picker still
/// offers [`crate::swarm::backend::shared::CLAUDE_MODEL_ALIASES`] in that case.
pub fn discover_model_options() -> Vec<String> {
    let output = crate::util::spawn::agent_command_std("claude")
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    let text = String::from_utf8_lossy(&output.stdout);
    parse_claude_help_models(&text)
}

/// Parse the `--model` option block out of `claude --help` into `claude:<id>`
/// specs. Extracted from [`discover_model_options`] so it can be unit-tested
/// without a `claude` binary on PATH.
///
/// The quoted aliases/full-names live in the option's description, which wraps
/// across several indented continuation lines, so we accumulate the `--model`
/// line plus its continuations (up to the next flag or a blank line) before
/// extracting ids.
fn parse_claude_help_models(help_text: &str) -> Vec<String> {
    let mut block = String::new();
    let mut in_model = false;
    for line in help_text.lines() {
        let trimmed = line.trim();
        if in_model {
            // A new option (`-x` / `--flag`) or a blank line ends the block.
            if trimmed.is_empty() || trimmed.starts_with('-') {
                break;
            }
            block.push(' ');
            block.push_str(trimmed);
        } else if trimmed.starts_with("--model") {
            in_model = true;
            block.push_str(trimmed);
        }
    }

    if block.is_empty() {
        return Vec::new();
    }

    extract_quoted_model_ids(&block)
        .into_iter()
        .map(|id| format!("claude:{id}"))
        .collect()
}

/// Extract single-quoted, model-id-shaped tokens from help prose.
///
/// Robust against stray apostrophes (e.g. "a model's full name"): a `'…'` pair
/// whose contents don't look like a model id is not consumed — scanning
/// resumes one byte past the opening quote, so a genuine id later on the line
/// (e.g. `'claude-fable-5'`) is still recovered instead of being swallowed by
/// the mispaired apostrophe.
fn extract_quoted_model_ids(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\'' {
            i += 1;
            continue;
        }
        if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'\'') {
            let content = &text[i + 1..i + 1 + rel];
            if is_model_id_shaped(content) {
                out.push(content.to_string());
                i = i + 1 + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Model ids are ASCII alphanumerics plus `_ . -` (matches the Cursor parser's
/// defensive shape check). Rejects empty strings and any token with spaces or
/// punctuation, which is what keeps apostrophe-induced garbage out.
fn is_model_id_shaped(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Query the Cursor CLI (`agent --list-models`) for the model ids the
/// current account can route to, prefixed with `cursor:` so the TUI's
/// model picker receives canonical `provider:model` strings.
///
/// On failure — binary missing, not logged in, free-tier "named models
/// unavailable" banner — falls back to `["cursor:composer-2.5"]` (the
/// default for paid plans). `cursor:composer-2.5` is pinned at the top
/// on success too so it's the obvious selection; `cursor:auto`, when
/// present in the CLI's list, follows immediately after for free-tier
/// users.
pub fn discover_cursor_model_options() -> Vec<String> {
    let output = crate::util::spawn::agent_command_std("agent")
        .arg("--list-models")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    // Default fallback model — mirrors `swarm::backend::cursor::DEFAULT_CURSOR_MODEL`
    // so a future change to the constant doesn't silently re-introduce
    // `auto` as the picker's default.
    let default_pin = "cursor:composer-2.5".to_string();
    let auto_alt = "cursor:auto".to_string();
    let fallback = || vec![default_pin.clone()];
    let Ok(output) = output else {
        return fallback();
    };
    if !output.status.success() {
        return fallback();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let parsed = parse_cursor_list_models(&text);
    if parsed.is_empty() {
        return fallback();
    }

    pin_cursor_picker_order(parsed, &default_pin, &auto_alt)
}

/// Pin order: default first, then `auto` (free-tier safe option), then
/// the remainder in `--list-models` order. Dedup preserves the order
/// established above. Extracted so tests can pin the rule without
/// shelling out to the `agent` CLI.
fn pin_cursor_picker_order(parsed: Vec<String>, default_pin: &str, auto_alt: &str) -> Vec<String> {
    let mut deduped: Vec<String> = Vec::with_capacity(parsed.len() + 2);
    deduped.push(default_pin.to_string());
    if parsed.iter().any(|m| m == auto_alt) {
        deduped.push(auto_alt.to_string());
    }
    for m in parsed {
        if !deduped.contains(&m) {
            deduped.push(m);
        }
    }
    deduped
}

/// Parse the body of `agent --list-models` / `agent models` into
/// `cursor:<id>` strings. Extracted so the unit tests can exercise the
/// parsing without depending on a Cursor CLI being installed in CI.
fn parse_cursor_list_models(text: &str) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Available models") {
            continue;
        }
        if !trimmed.contains(" - ") {
            continue;
        }
        let id = trimmed.split(" - ").next().unwrap_or("").trim();
        if id.is_empty() {
            continue;
        }
        // Defensive validation: model ids are ASCII alnum + `_.-`. Skip
        // anything else so a future banner row can't poison the list.
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            continue;
        }
        models.push(format!("cursor:{}", id));
    }
    models
}

#[cfg(test)]
mod discover_cursor_tests {
    use super::{parse_cursor_list_models, pin_cursor_picker_order};

    #[test]
    fn parses_real_world_list_models_output() {
        let sample = "Available models\n\nauto - Auto\ncomposer-2-fast - Composer 2 Fast (default)\ngpt-5.2 - GPT-5.2\nclaude-4.6-opus-high-thinking - Opus 4.6 1M Thinking\n";
        let out = parse_cursor_list_models(sample);
        assert!(out.contains(&"cursor:auto".to_string()));
        assert!(out.contains(&"cursor:composer-2-fast".to_string()));
        assert!(out.contains(&"cursor:gpt-5.2".to_string()));
        assert!(out.contains(&"cursor:claude-4.6-opus-high-thinking".to_string()));
    }

    #[test]
    fn skips_informational_lines_without_dash_separator() {
        let sample = "Available models\n\nFree plans only have Auto.\nauto - Auto\n";
        let out = parse_cursor_list_models(sample);
        assert_eq!(out, vec!["cursor:auto".to_string()]);
    }

    #[test]
    fn skips_ids_with_invalid_characters() {
        let sample = "auto - Auto\nbad model id - Should Be Skipped\nworking-id - Kept\n";
        let out = parse_cursor_list_models(sample);
        assert!(out.contains(&"cursor:auto".to_string()));
        assert!(out.contains(&"cursor:working-id".to_string()));
        assert!(!out.iter().any(|m| m.contains("bad")));
    }

    #[test]
    fn pin_order_puts_default_first_then_auto_then_rest() {
        // Real CLI ordering shows `auto` first and `composer-2.5`
        // somewhere later. The picker order must surface the default
        // (`composer-2.5`) first so it's the obvious selection, then
        // keep `auto` available right after as the free-tier fallback.
        let parsed = vec![
            "cursor:auto".to_string(),
            "cursor:composer-2-fast".to_string(),
            "cursor:gpt-5.2".to_string(),
            "cursor:composer-2.5".to_string(),
            "cursor:claude-4.6-opus-high-thinking".to_string(),
        ];
        let out = pin_cursor_picker_order(parsed, "cursor:composer-2.5", "cursor:auto");
        assert_eq!(out[0], "cursor:composer-2.5");
        assert_eq!(out[1], "cursor:auto");
        // Remaining models follow in their input order, minus the two
        // pinned slots which were de-duplicated.
        assert_eq!(out[2], "cursor:composer-2-fast");
        assert_eq!(out[3], "cursor:gpt-5.2");
        assert_eq!(out[4], "cursor:claude-4.6-opus-high-thinking");
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn pin_order_skips_auto_when_not_in_parsed_list() {
        // Paid-tier accounts that don't have `auto` listed (or a future
        // CLI release that drops the alias) must still get the default
        // at the top — no spurious `auto` entry inserted.
        let parsed = vec![
            "cursor:composer-2.5".to_string(),
            "cursor:gpt-5.2".to_string(),
        ];
        let out = pin_cursor_picker_order(parsed, "cursor:composer-2.5", "cursor:auto");
        assert_eq!(out[0], "cursor:composer-2.5");
        assert_eq!(out[1], "cursor:gpt-5.2");
        assert_eq!(out.len(), 2);
        assert!(!out.iter().any(|m| m == "cursor:auto"));
    }

    #[test]
    fn pin_order_inserts_default_even_when_absent_from_parsed_list() {
        // If the CLI's list doesn't include the default model, we still
        // surface it so the user can attempt it — the runtime will
        // route the request and surface any "model unavailable" error.
        let parsed = vec!["cursor:auto".to_string(), "cursor:gpt-5.2".to_string()];
        let out = pin_cursor_picker_order(parsed, "cursor:composer-2.5", "cursor:auto");
        assert_eq!(out[0], "cursor:composer-2.5");
        assert_eq!(out[1], "cursor:auto");
        assert_eq!(out[2], "cursor:gpt-5.2");
    }
}

#[cfg(test)]
mod discover_claude_tests {
    use super::{extract_quoted_model_ids, parse_claude_help_models};

    /// Real `claude --help` shape: the quoted aliases/full-name wrap onto
    /// indented continuation lines below the `--model` line, and the prose
    /// contains a stray apostrophe in "model's". The old parser only scanned
    /// the `--model` line itself and returned nothing — the regression this
    /// fixes (typing `claude:` surfaced no models).
    const WRAPPED_HELP: &str = "\
  --mcp-debug                           [DEPRECATED] Enable MCP debug mode
  --model <model>                       Model for the current session. Provide
                                        an alias for the latest model (e.g.
                                        'fable', 'opus', or 'sonnet') or a
                                        model's full name (e.g.
                                        'claude-fable-5').
  -n, --name <name>                     Set a display name for this session
";

    #[test]
    fn parses_aliases_across_wrapped_continuation_lines() {
        let out = parse_claude_help_models(WRAPPED_HELP);
        assert!(out.contains(&"claude:fable".to_string()), "got {out:?}");
        assert!(out.contains(&"claude:opus".to_string()), "got {out:?}");
        assert!(out.contains(&"claude:sonnet".to_string()), "got {out:?}");
    }

    #[test]
    fn recovers_full_model_name_after_stray_apostrophe() {
        // The apostrophe in "model's" must not swallow the later
        // 'claude-fable-5' id, and must not leak a garbage entry.
        let out = parse_claude_help_models(WRAPPED_HELP);
        assert!(
            out.contains(&"claude:claude-fable-5".to_string()),
            "got {out:?}"
        );
        assert!(
            out.iter().all(|m| !m.contains(' ')),
            "no model id should contain spaces: {out:?}"
        );
    }

    #[test]
    fn returns_empty_when_no_model_flag_present() {
        let out = parse_claude_help_models("  --verbose   Be loud\n  -h, --help   Help\n");
        assert!(out.is_empty(), "got {out:?}");
    }

    #[test]
    fn parses_inline_aliases_on_a_single_wide_line() {
        // Wide terminals keep everything on the `--model` line; that case
        // must still parse.
        let inline = "  --model <model>   alias 'opus' or 'sonnet' or full 'claude-fable-5'\n";
        let out = parse_claude_help_models(inline);
        assert_eq!(
            out,
            vec![
                "claude:opus".to_string(),
                "claude:sonnet".to_string(),
                "claude:claude-fable-5".to_string(),
            ]
        );
    }

    #[test]
    fn extract_skips_non_id_shaped_quoted_text() {
        let ids = extract_quoted_model_ids("'good-id' and 'not an id' and 'also.good'");
        assert_eq!(ids, vec!["good-id".to_string(), "also.good".to_string()]);
    }
}
