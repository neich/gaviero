//! Per-backend MCP config synthesis (Tier A / A5).
//!
//! At subprocess spawn time, Gaviero writes a backend-appropriate MCP
//! config pointing at the `gaviero-mcp-shim` binary + the workspace's
//! socket path. The shim forwards stdio↔socket so the subprocess
//! agent sees a standard stdio MCP server.
//!
//! Config placement (plan §A5):
//!
//! * **Claude Code** — `<worktree>/.mcp.json`; Claude picks it up
//!   automatically when the CLI is invoked with `--mcp-config` or with
//!   the default project-level discovery.
//! * **Codex** — `<worktree>/.codex/config.toml`; Codex reads it when
//!   the worktree is a trusted project. First-time use requires user
//!   consent (shown in the TUI via [`TrustConsent`]).
//! * **Cursor** — `<worktree>/.cursor/mcp.json`; same JSON schema as
//!   Claude's `.mcp.json` (`{"mcpServers":{...}}`). Cursor's CLI picks
//!   it up automatically when the worktree is the cwd; no trust gate.
//!
//! All configs are per-worktree, not per-user, so swarm worktrees
//! get isolated MCP wiring that cleans up with the worktree itself.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Trusted-projects consent state for Codex (plan §A5: "one-time user
/// consent dialog"). TUI-layer opens a prompt; the `Workspace`
/// persists the answer in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustConsent {
    /// Consent not yet requested.
    Unknown,
    /// User accepted: Codex may receive `.codex/config.toml`.
    Granted,
    /// User declined: Codex runs without MCP, falls back to prompt-
    /// time injection only.
    Denied,
}

impl Default for TrustConsent {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Context7 MCP server defaults (Upstash hosted docs lookup).
///
/// Gaviero injects this server entry alongside the `gaviero` shim
/// entry so swarm subprocess agents (Claude Code, Codex) can call
/// `resolve-library-id` / `get-library-docs` against current docs
/// instead of relying on stale training data. Disabled per workspace
/// via `mcp.context7.enabled = false` (e.g. offline / privacy work).
#[derive(Debug, Clone)]
pub struct Context7Config {
    /// When `false`, no context7 entry is written into either the
    /// Claude `.mcp.json` or the Codex `config.toml`. Existing entries
    /// the user may have authored manually are preserved by the merge
    /// step.
    pub enabled: bool,
    /// Runtime that hosts the context7 MCP server. Default `"npx"`;
    /// users can swap to `"bunx"`, `"pnpm"`, or an absolute path.
    pub command: String,
    /// Argv passed to `command`. Default `["-y", "@upstash/context7-mcp"]`.
    pub args: Vec<String>,
}

impl Default for Context7Config {
    fn default() -> Self {
        Self {
            enabled: true,
            command: "npx".to_string(),
            args: vec!["-y".into(), "@upstash/context7-mcp".into()],
        }
    }
}

/// Transport for an operator-defined MCP server (workspace or CLI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraMcpTransport {
    /// Remote MCP (SSE / streamable HTTP) — Claude/Cursor `url` field.
    Url { url: String },
    /// Local subprocess — `command` + `args`.
    Stdio { command: String, args: Vec<String> },
}

/// Named MCP server merged into every synthesized worktree config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraMcpServer {
    pub name: String,
    pub transport: ExtraMcpTransport,
}

/// Configuration for a per-worktree MCP config synth.
#[derive(Debug, Clone)]
pub struct McpConfigSynth {
    /// Absolute path to the worktree where the subprocess will run.
    pub worktree: PathBuf,
    /// Absolute path to the Unix domain socket the MCP server is
    /// listening on. The shim proxies stdio to this socket.
    pub socket_path: PathBuf,
    /// Path (or bare name) written verbatim as the config's `command`.
    /// The default `"gaviero-mcp-shim"` expects the binary to be on
    /// `PATH` — `cargo install --path crates/gaviero-mcp-shim` or a
    /// distribution package must place it there. Subprocess agents
    /// (Claude Code, Codex) spawn this as their MCP server; if PATH
    /// resolution fails they log `command not found` and fall back to
    /// prompt-time injection only. Callers who install the shim to a
    /// non-standard location should override with an absolute path.
    pub shim_binary: String,
    /// Codex trust state. When `Denied`, synthesis skips Codex config.
    pub codex_trust: TrustConsent,
    /// When `false`, no config files are written — caller can suppress
    /// MCP entirely for one-shot `gaviero-cli --no-mcp` runs.
    pub enabled: bool,
    /// When `false`, omit the `gaviero` shim entry (no in-process socket).
    pub gaviero_enabled: bool,
    /// Context7 docs-lookup MCP server defaults.
    pub context7: Context7Config,
    /// Extra servers from `mcp.extraServers` settings and/or CLI flags.
    pub extra_servers: Vec<ExtraMcpServer>,
}

impl Default for McpConfigSynth {
    fn default() -> Self {
        Self {
            worktree: PathBuf::new(),
            socket_path: PathBuf::new(),
            shim_binary: "gaviero-mcp-shim".to_string(),
            codex_trust: TrustConsent::Unknown,
            enabled: true,
            gaviero_enabled: true,
            context7: Context7Config::default(),
            extra_servers: Vec::new(),
        }
    }
}

/// Build the `.mcp.json` body for Claude Code.
///
/// Schema: `mcpServers` is the top-level key; each entry is a
/// `{ command, args, env? }` record. Claude Code reads this from the
/// worktree root (auto-discovery) or via the `--mcp-config <path>`
/// flag.
pub fn claude_mcp_config_json(synth: &McpConfigSynth) -> Result<String> {
    let servers = managed_mcp_json_servers(synth)?;
    let body = serde_json::json!({ "mcpServers": serde_json::Value::Object(servers) });
    Ok(serde_json::to_string_pretty(&body).context("serialising .mcp.json")?)
}

fn managed_mcp_json_servers(synth: &McpConfigSynth) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut servers = serde_json::Map::new();
    if synth.gaviero_enabled {
        servers.insert("gaviero".to_string(), gaviero_server_entry(synth));
    }
    if synth.context7.enabled {
        servers.insert("context7".to_string(), context7_server_entry(&synth.context7));
    }
    for extra in &synth.extra_servers {
        servers.insert(extra.name.clone(), extra_server_json_entry(extra));
    }
    Ok(servers)
}

/// Hostname from an `http(s)://` MCP URL (for Cursor network allowlists).
pub fn host_from_mcp_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Whether `path` (a `.cursor/mcp.json` or `.mcp.json`) declares any remote `url` server.
pub fn mcp_json_has_remote_urls(path: &Path) -> bool {
    !remote_mcp_servers_from_mcp_json_path(path).is_empty()
}

/// Whether the worktree declares remote HTTP/SSE MCP servers for Cursor.
///
/// Probes both `.cursor/mcp.json` (Cursor-native) and `.mcp.json` (Claude /
/// user-authored extras) so `--sandbox disabled` is applied even when only
/// the latter file carries URL entries before synthesis.
pub fn worktree_has_remote_mcp_urls(worktree: &Path) -> bool {
    mcp_json_has_remote_urls(&worktree.join(".cursor/mcp.json"))
        || mcp_json_has_remote_urls(&worktree.join(".mcp.json"))
}

/// True when the synth struct declares at least one remote URL extra server.
pub fn synth_has_remote_url_servers(synth: &McpConfigSynth) -> bool {
    synth
        .extra_servers
        .iter()
        .any(|s| matches!(s.transport, ExtraMcpTransport::Url { .. }))
}

/// `(server_name, url_host)` pairs from a `{"mcpServers":{...}}` file.
pub fn remote_mcp_servers_from_mcp_json_path(path: &Path) -> Vec<(String, String)> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    remote_mcp_servers_from_mcp_json_body(&body)
}

fn remote_mcp_servers_from_mcp_json_body(body: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, entry) in servers {
        let Some(url) = entry.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let Some(host) = host_from_mcp_url(url) else {
            continue;
        };
        out.push((name.clone(), host));
    }
    out
}

fn extra_server_json_entry(extra: &ExtraMcpServer) -> serde_json::Value {
    match &extra.transport {
        ExtraMcpTransport::Url { url } => serde_json::json!({ "url": url }),
        ExtraMcpTransport::Stdio { command, args } => serde_json::json!({
            "command": command,
            "args": args,
        }),
    }
}

/// Cursor streamable-HTTP MCP entries need an explicit `headers` object
/// (empty when no auth headers are configured).
fn extra_server_json_entry_for_cursor(extra: &ExtraMcpServer) -> serde_json::Value {
    match &extra.transport {
        ExtraMcpTransport::Url { url } => serde_json::json!({ "url": url, "headers": {} }),
        ExtraMcpTransport::Stdio { command, args } => serde_json::json!({
            "command": command,
            "args": args,
        }),
    }
}

/// Managed server entries for `.cursor/mcp.json`.
///
/// Cursor's headless MCP registry is fragile when stdio servers fail at
/// startup — a broken `npx` (context7) or missing shim can prevent URL
/// servers from registering in `ListMcpResources`. Keep Cursor lean:
/// remote extras + gaviero (only when the shim resolves), no context7.
fn managed_cursor_mcp_json_servers(synth: &McpConfigSynth) -> Result<serde_json::Map<String, serde_json::Value>> {
    use super::preflight::shim_binary_resolvable;

    let has_remote_extra = synth_has_remote_url_servers(synth);
    let mut servers = serde_json::Map::new();
    // When a remote URL extra (e.g. semantic-scholar) is configured, keep
    // Cursor's registry lean — the stdio gaviero shim competes for startup
    // with streamable HTTP and often leaves ListMcpResources empty.
    if synth.gaviero_enabled
        && !has_remote_extra
        && shim_binary_resolvable(&synth.shim_binary)
    {
        servers.insert("gaviero".to_string(), gaviero_server_entry(synth));
    }
    for extra in &synth.extra_servers {
        servers.insert(
            extra.name.clone(),
            extra_server_json_entry_for_cursor(extra),
        );
    }
    Ok(servers)
}

/// Build the `.cursor/mcp.json` body for the Cursor CLI.
///
/// Same top-level `{"mcpServers":{...}}` schema as Claude's `.mcp.json`,
/// but omits context7 and skips the gaviero shim when it is not resolvable
/// so a poisoned stdio server cannot block remote URL registration.
pub fn cursor_mcp_config_json(synth: &McpConfigSynth) -> Result<String> {
    let servers = managed_cursor_mcp_json_servers(synth)?;
    let body = serde_json::json!({ "mcpServers": serde_json::Value::Object(servers) });
    Ok(serde_json::to_string_pretty(&body).context("serialising .cursor/mcp.json")?)
}

fn gaviero_server_entry(synth: &McpConfigSynth) -> serde_json::Value {
    serde_json::json!({
        "command": synth.shim_binary,
        "args": ["--socket", synth.socket_path.to_string_lossy()],
    })
}

fn context7_server_entry(ctx7: &Context7Config) -> serde_json::Value {
    serde_json::json!({
        "command": ctx7.command,
        "args": ctx7.args,
    })
}

/// Match a comma-form Cursor MCP permission entry like `Mcp(name, *)`
/// or `Mcp(*, *)` — the broken shape gaviero used to emit before
/// 2026-06-03. Used by [`synthesize_cursor_cli_for_remote_mcp`] to
/// scrub poisoned entries from an existing cli.json before adding the
/// correct colon-form ones.
fn is_legacy_comma_mcp_pattern(s: &str) -> bool {
    let trimmed = s.trim();
    let Some(inner) = trimmed
        .strip_prefix("Mcp(")
        .and_then(|s| s.strip_suffix(')'))
    else {
        return false;
    };
    inner.contains(',') && !inner.contains(':')
}

/// Whether the synthesized `<worktree>/.codex/config.toml` declares **any**
/// MCP server at all (stdio or remote).
///
/// Used to decide when to invoke `codex exec` with
/// `--dangerously-bypass-approvals-and-sandbox`. Probed against
/// `codex-cli 0.131.0`: with **any** other `approval_policy` value
/// (`never`, `on-request`, `on-failure`, `untrusted`), MCP tool calls
/// surface as `user cancelled MCP tool call` because `codex exec` is
/// non-interactive and has no way to satisfy the elicitation request.
/// The bypass flag is the documented escape hatch for externally
/// sandboxed environments — for gaviero swarm agents that's the
/// per-agent git worktree plus the Write Gate at merge time.
pub fn codex_synth_has_any_mcp(workspace_root: &Path) -> bool {
    let codex_config = workspace_root.join(".codex/config.toml");
    let Ok(body) = std::fs::read_to_string(&codex_config) else {
        return false;
    };
    let Ok(value) = body.parse::<toml::Value>() else {
        return false;
    };
    value
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .map(|t| !t.is_empty())
        .unwrap_or(false)
}

/// Whether the synthesized `<worktree>/.codex/config.toml` declares any
/// HTTP MCP server (`url = "..."` entry under `[mcp_servers.NAME]`).
///
/// Used to decide when to grant network access to Codex's sandbox:
/// `--sandbox read-only` blocks outbound HTTP to MCP endpoints, so when
/// any remote URL is present the caller must upgrade the sandbox mode
/// and set `sandbox_workspace_write.network_access=true`. Empty when
/// the file is missing, malformed, or contains only stdio servers.
pub fn codex_synth_has_remote_mcp(workspace_root: &Path) -> bool {
    let codex_config = workspace_root.join(".codex/config.toml");
    let Ok(body) = std::fs::read_to_string(&codex_config) else {
        return false;
    };
    let Ok(value) = body.parse::<toml::Value>() else {
        return false;
    };
    let Some(servers) = value.get("mcp_servers").and_then(toml::Value::as_table) else {
        return false;
    };
    servers.values().any(|entry| {
        entry
            .as_table()
            .and_then(|t| t.get("url"))
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    })
}

/// Read every `[mcp_servers.NAME]` table from the synthesized
/// `<worktree>/.codex/config.toml` and emit `key=value` strings suitable
/// for `codex exec --config <pair>` overrides.
///
/// Codex CLI only loads its config from `$CODEX_HOME/config.toml`
/// (default `~/.codex/config.toml`); it never walks up from the
/// worktree, so the synthesized per-worktree TOML is otherwise ignored.
/// Replaying each MCP server table as a `--config` override is the only
/// way to make the synthesized servers visible to a `codex exec`
/// invocation without persisting them globally.
///
/// Returns an empty vec when the file is missing or fails to parse.
pub fn codex_mcp_overrides_from_config_file(path: &Path) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let value = match body.parse::<toml::Value>() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "mcp_config",
                path = %path.display(),
                error = %e,
                "synthesized codex MCP config failed to parse; skipping --config overrides"
            );
            return Vec::new();
        }
    };
    let Some(servers) = value.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, entry) in servers {
        let Some(table) = entry.as_table() else {
            continue;
        };
        for (key, val) in table {
            let Some(inline) = toml_value_inline(val) else {
                continue;
            };
            out.push(format!("mcp_servers.{name}.{key}={inline}"));
        }
    }
    out
}

fn toml_value_inline(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(s) => Some(toml_quote_string(s)),
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        toml::Value::Boolean(b) => Some(b.to_string()),
        toml::Value::Datetime(d) => Some(d.to_string()),
        toml::Value::Array(arr) => {
            let parts: Option<Vec<String>> = arr.iter().map(toml_value_inline).collect();
            parts.map(|p| format!("[{}]", p.join(", ")))
        }
        toml::Value::Table(table) => {
            let parts: Option<Vec<String>> = table
                .iter()
                .map(|(k, v)| toml_value_inline(v).map(|s| format!("{} = {}", toml_quote_key(k), s)))
                .collect();
            parts.map(|p| format!("{{ {} }}", p.join(", ")))
        }
    }
}

fn toml_quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_quote_key(k: &str) -> String {
    let bare_ok = !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare_ok {
        k.to_string()
    } else {
        toml_quote_string(k)
    }
}

/// Build the `.codex/config.toml` body for Codex.
///
/// Schema: `[mcp_servers.gaviero]` — one table per server. Codex's CLI
/// only loads `$CODEX_HOME/config.toml`, so this per-worktree file is
/// also replayed as `--config` overrides via
/// [`codex_mcp_overrides_from_config_file`] when `codex exec` is
/// spawned. The synthesized file remains useful as an audit artefact
/// and as the input the host re-reads at spawn time. A
/// `[projects.<worktree>]` trust stanza is included so the grant is
/// still discoverable if Codex ever gains worktree-local config
/// discovery.
pub fn codex_mcp_config_toml(synth: &McpConfigSynth) -> Result<String> {
    // Manually construct the TOML — toml's serializer doesn't like
    // the dotted-header shape Codex expects.
    let socket = synth.socket_path.to_string_lossy();
    let worktree = synth.worktree.to_string_lossy();
    let trust_value = match synth.codex_trust {
        TrustConsent::Granted => "trusted",
        _ => "untrusted",
    };
    let mut body = String::from(
        "# Generated by gaviero (Tier A / A5). Do not edit — regenerated per swarm worktree.\n\n",
    );
    if synth.gaviero_enabled {
        body.push_str(&format!(
            "[mcp_servers.gaviero]\n\
             command = {command:?}\n\
             args = [\"--socket\", {socket:?}]\n\
             startup_timeout_sec = {start}\n\
             tool_timeout_sec = {tool}\n",
            command = synth.shim_binary,
            socket = socket,
            start = CODEX_MCP_STARTUP_TIMEOUT_SECS,
            tool = CODEX_MCP_TOOL_TIMEOUT_SECS,
        ));
    }
    if synth.context7.enabled {
        body.push_str(&format!(
            "\n[mcp_servers.context7]\n\
             command = {command:?}\n\
             args = {args}\n\
             startup_timeout_sec = {start}\n\
             tool_timeout_sec = {tool}\n",
            command = synth.context7.command,
            args = toml_string_array(&synth.context7.args),
            start = CODEX_MCP_STARTUP_TIMEOUT_SECS,
            tool = CODEX_MCP_TOOL_TIMEOUT_SECS,
        ));
    }
    for extra in &synth.extra_servers {
        body.push_str(&extra_server_codex_toml(extra));
    }
    body.push_str(&format!(
        "\n[projects.{worktree:?}]\n\
         trust_level = {trust_value:?}\n",
        worktree = worktree,
        trust_value = trust_value,
    ));
    Ok(body)
}

/// Default startup + tool timeouts emitted into the synthesized codex MCP
/// config. Codex's own defaults are tight enough that a slow remote MCP
/// (Wuilder proxies, OAuth-fronted endpoints) intermittently surfaces as
/// "MCP calls were cancelled by the tool layer". 60 seconds matches the
/// ballpark we see on cold remote servers and is forward-compatible with
/// codex's `startup_timeout_sec` / `tool_timeout_sec` config keys.
const CODEX_MCP_STARTUP_TIMEOUT_SECS: u32 = 60;
const CODEX_MCP_TOOL_TIMEOUT_SECS: u32 = 60;

fn extra_server_codex_toml(extra: &ExtraMcpServer) -> String {
    let header = format!("\n[mcp_servers.{}]\n", extra.name);
    let body = match &extra.transport {
        ExtraMcpTransport::Url { url } => format!("{header}url = {url:?}\n"),
        ExtraMcpTransport::Stdio { command, args } => format!(
            "{header}command = {command:?}\nargs = {}\n",
            toml_string_array(args),
        ),
    };
    // `required = true` makes codex block conversation startup until this
    // server is `ready`. Without it, codex starts the session in parallel
    // with MCP server startup, and a slow remote endpoint (or one that
    // hasn't finished its handshake) lands in the `cancelled` state by the
    // time the model tries to call a tool — which the agent surfaces as
    // "MCP calls were cancelled by the tool layer". For user-explicit
    // extras the failure mode we want is a clean startup error from
    // `codex exec`, not silent agent confusion. The built-in `gaviero` and
    // `context7` servers stay non-required so a missing shim / npx outage
    // doesn't take down the whole run.
    format!(
        "{body}startup_timeout_sec = {start}\n\
         tool_timeout_sec = {tool}\n\
         required = true\n",
        start = CODEX_MCP_STARTUP_TIMEOUT_SECS,
        tool = CODEX_MCP_TOOL_TIMEOUT_SECS,
    )
}

fn toml_string_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{:?}", s));
    }
    out.push(']');
    out
}

/// Write the appropriate config files into `worktree`. Returns the
/// paths written. Creates parent directories as needed.
///
/// Skips Claude config if `enabled = false`. Skips Codex config if
/// `enabled = false` OR `codex_trust != Granted`.
pub fn synthesize_for_worktree(synth: &McpConfigSynth) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    if !synth.enabled {
        return Ok(written);
    }

    // Claude: <worktree>/.mcp.json
    let claude_path = synth.worktree.join(".mcp.json");
    let claude_body = mcp_json_servers_merged(synth, &claude_path)?;
    write_if_changed(&claude_path, &claude_body)?;
    written.push(claude_path);

    // Cursor: <worktree>/.cursor/mcp.json — same schema as Claude's
    // `.mcp.json`, so the merge helper handles preserving any user
    // entries the same way.
    let cursor_dir = synth.worktree.join(".cursor");
    std::fs::create_dir_all(&cursor_dir)
        .with_context(|| format!("creating {}", cursor_dir.display()))?;
    let cursor_path = cursor_dir.join("mcp.json");
    let cursor_body = cursor_mcp_json_servers_merged(synth, &cursor_path)?;
    write_if_changed(&cursor_path, &cursor_body)?;
    if let Some(cli_path) = synthesize_cursor_cli_for_remote_mcp(&cursor_dir, &cursor_path)? {
        written.push(cli_path);
    }
    written.push(cursor_path);

    // Codex: <worktree>/.codex/config.toml — only when trust is
    // explicitly granted. Plan defaults to asking on first swarm.
    if matches!(synth.codex_trust, TrustConsent::Granted) {
        let codex_dir = synth.worktree.join(".codex");
        std::fs::create_dir_all(&codex_dir)
            .with_context(|| format!("creating {}", codex_dir.display()))?;
        let codex_path = codex_dir.join("config.toml");
        let codex_body = codex_mcp_config_toml(synth)?;
        write_if_changed(&codex_path, &codex_body)?;
        written.push(codex_path);
    }

    Ok(written)
}

/// Write `<worktree>/.cursor/cli.json` when remote MCP URLs are present, so
/// headless `agent -p` allow-lists the MCP server entries and any `WebFetch`
/// hosts they cover.
///
/// **Project-level schema is permissions-only.** Per cursor.com/docs/cli/
/// reference/configuration: "Project-specific configurations can be placed
/// in `<project>/.cursor/cli.json`, but only permission settings can be
/// configured at this level; all other settings must be global." The
/// `version` / `editor` / `approvalMode` / `sandbox` /
/// `webFetchDomainAllowlist` keys belong in the *global* file
/// (`~/.cursor/cli-config.json`); writing them at project level fails the
/// schema validator with `"unrecognized_keys"`, which kills every `agent`
/// invocation in the worktree. CLI-wide policies (sandbox off, MCP
/// auto-approve) ride argv flags from
/// [`crate::swarm::backend::cursor::cursor_argv`] instead. This function
/// emits **only** the `permissions` object and preserves any user-authored
/// entries already in the file.
fn synthesize_cursor_cli_for_remote_mcp(
    cursor_dir: &Path,
    mcp_json_path: &Path,
) -> Result<Option<PathBuf>> {
    let remote = remote_mcp_servers_from_mcp_json_path(mcp_json_path);
    if remote.is_empty() {
        return Ok(None);
    }

    let cli_path = cursor_dir.join("cli.json");
    let mut root = std::fs::read_to_string(&cli_path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    // Strip every key that's only valid in the global cli-config.json (or
    // an obsolete one from a prior gaviero version). Leaving them in place
    // would re-trip Cursor's `unrecognized_keys` validator and kill the
    // turn before our `permissions` additions even matter. `version` is
    // included here because, despite what the cursor.com docs imply, the
    // schema validator rejects it at the project level (`v2025.11.x+`).
    for k in [
        "version",
        "editor",
        "approvalMode",
        "sandbox",
        "webFetchDomainAllowlist",
    ] {
        root.remove(k);
    }

    // Build the host allowlist for WebFetch permissions: both the apex host
    // (`api.example.com`) and the wildcard subdomain form (`*.api.example.com`)
    // so a request that follows a redirect to a subdomain still resolves.
    let mut hosts: Vec<String> = remote
        .iter()
        .map(|(_, h)| h.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    hosts.sort_unstable();
    let mut webfetch_patterns: Vec<String> = hosts
        .iter()
        .flat_map(|h| [format!("WebFetch({h})"), format!("WebFetch(*.{h})")])
        .collect();
    webfetch_patterns.sort();
    webfetch_patterns.dedup();

    // Cursor's permission syntax for MCP uses a colon, not a comma:
    // `Mcp(serverName:toolName)` — `Mcp(server:*)` for all tools from a
    // server, `Mcp(*:*)` for everything. Verified live against cursor
    // `agent 2026.06.02-8c11d9f`: with `Mcp(server, *)` the tool call
    // returns `User rejected MCP: <name>` even with `--approve-mcps`;
    // with `Mcp(server:*)` it succeeds and the server receives the
    // request. Sourced from cursor.com/docs/cli/reference/permissions
    // (the docs example uses `Mcp(datadog:*)`).
    let permissions = root
        .entry("permissions".to_string())
        .or_insert_with(|| serde_json::json!({ "allow": [], "deny": [] }));
    if !permissions.is_object() {
        *permissions = serde_json::json!({ "allow": [], "deny": [] });
    }
    if let Some(obj) = permissions.as_object_mut() {
        let allow = obj
            .entry("allow".to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let Some(arr) = allow.as_array_mut() {
            let mut existing: std::collections::HashSet<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            // Strip any prior comma-format entries our earlier releases
            // wrote — they don't grant access and are pure noise.
            arr.retain(|v| match v.as_str() {
                Some(s) => !is_legacy_comma_mcp_pattern(s),
                None => true,
            });
            existing.retain(|s| !is_legacy_comma_mcp_pattern(s));
            let mut patterns = vec!["Mcp(*:*)".to_string()];
            patterns.extend(remote.iter().map(|(s, _)| format!("Mcp({s}:*)")));
            patterns.extend(webfetch_patterns);
            for pat in patterns {
                if existing.insert(pat.clone()) {
                    arr.push(serde_json::Value::String(pat));
                }
            }
        }
        obj.entry("deny".to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    }

    let body = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serialising .cursor/cli.json")?;
    write_if_changed(&cli_path, &body)?;
    Ok(Some(cli_path))
}

/// Merge lean Cursor-managed entries into an existing `.cursor/mcp.json`.
fn cursor_mcp_json_servers_merged(synth: &McpConfigSynth, path: &Path) -> Result<String> {
    let managed = managed_cursor_mcp_json_servers(synth)?;
    let mut root = std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !servers.is_object() {
        *servers = serde_json::Value::Object(serde_json::Map::new());
    }
    let servers_map = servers.as_object_mut().expect("mcpServers object");
    // Drop stale managed stdio servers from older gaviero runs (context7
    // poisons ListMcpResources; gaviero is omitted when URL extras exist).
    for stale in ["context7", "gaviero"] {
        if !managed.contains_key(stale) {
            servers_map.remove(stale);
        }
    }
    for (name, entry) in managed {
        servers_map.insert(name, entry);
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serialising merged cursor mcp servers JSON")
}

/// Merge our gaviero (and optionally context7) entries into an existing
/// `{"mcpServers":{...}}` document at `path`, preserving any
/// user-authored entries. Used for `.mcp.json` (Claude).
fn mcp_json_servers_merged(synth: &McpConfigSynth, path: &Path) -> Result<String> {
    merge_mcp_json_servers_at_path(path, managed_mcp_json_servers(synth)?)
}

fn merge_mcp_json_servers_at_path(
    path: &Path,
    managed: serde_json::Map<String, serde_json::Value>,
) -> Result<String> {
    let mut root = std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !servers.is_object() {
        *servers = serde_json::Value::Object(serde_json::Map::new());
    }
    let servers_map = servers.as_object_mut().expect("mcpServers object");
    for (name, entry) in managed {
        servers_map.insert(name, entry);
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serialising merged mcp servers JSON")
}

/// Write `body` to `path` only when the contents differ — avoids
/// churning mtimes on Codex's trust cache. Creates parent dirs.
fn write_if_changed(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == body {
            return Ok(());
        }
    }
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture(worktree: PathBuf) -> McpConfigSynth {
        McpConfigSynth {
            worktree: worktree.clone(),
            socket_path: worktree.join(".gaviero/mcp.sock"),
            shim_binary: "gaviero-mcp-shim".into(),
            codex_trust: TrustConsent::Granted,
            enabled: true,
            gaviero_enabled: true,
            context7: Context7Config::default(),
            extra_servers: Vec::new(),
        }
    }

    /// Fixture with a shim path that always resolves in unit tests.
    fn fixture_resolvable_shim(worktree: PathBuf) -> McpConfigSynth {
        let mut synth = fixture(worktree);
        synth.shim_binary = "/bin/sh".into();
        synth
    }

    #[test]
    fn claude_config_contains_server_block() {
        let synth = fixture(PathBuf::from("/tmp/wt"));
        let body = claude_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["mcpServers"]["gaviero"]["command"].is_string());
        assert_eq!(
            v["mcpServers"]["gaviero"]["args"][0].as_str().unwrap(),
            "--socket"
        );
    }

    #[test]
    fn codex_config_contains_trust_and_server_blocks() {
        let synth = fixture(PathBuf::from("/tmp/wt"));
        let body = codex_mcp_config_toml(&synth).unwrap();
        assert!(body.contains("[mcp_servers.gaviero]"));
        assert!(body.contains("command = \"gaviero-mcp-shim\""));
        assert!(body.contains("trust_level = \"trusted\""));
    }

    #[test]
    fn synth_writes_claude_and_cursor_always_codex_only_on_granted() {
        let dir = tempdir().unwrap();
        let mut synth = fixture(dir.path().to_path_buf());

        // Trust unknown → Claude + Cursor configs written, Codex skipped.
        synth.codex_trust = TrustConsent::Unknown;
        let files = synthesize_for_worktree(&synth).unwrap();
        assert_eq!(files.len(), 2);
        assert!(
            files.iter().any(|p| p.ends_with(".mcp.json")),
            "expected .mcp.json among {:?}",
            files
        );
        assert!(
            files.iter().any(|p| p.ends_with(".cursor/mcp.json")),
            "expected .cursor/mcp.json among {:?}",
            files
        );
        assert!(!dir.path().join(".codex/config.toml").exists());

        // Trust granted → all three configs.
        synth.codex_trust = TrustConsent::Granted;
        let files = synthesize_for_worktree(&synth).unwrap();
        assert_eq!(files.len(), 3);
        assert!(dir.path().join(".cursor/mcp.json").exists());
        assert!(dir.path().join(".codex/config.toml").exists());
    }

    #[test]
    fn cursor_config_contains_gaviero_when_shim_resolvable() {
        let synth = fixture_resolvable_shim(PathBuf::from("/tmp/wt"));
        let body = cursor_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["mcpServers"]["gaviero"]["command"].is_string());
        assert_eq!(
            v["mcpServers"]["gaviero"]["args"][0].as_str().unwrap(),
            "--socket"
        );
    }

    #[test]
    fn cursor_config_omits_context7_even_when_enabled() {
        let synth = fixture_resolvable_shim(PathBuf::from("/tmp/wt"));
        let claude = claude_mcp_config_json(&synth).unwrap();
        let cursor = cursor_mcp_config_json(&synth).unwrap();
        let claude_v: serde_json::Value = serde_json::from_str(&claude).unwrap();
        let cursor_v: serde_json::Value = serde_json::from_str(&cursor).unwrap();
        assert!(claude_v["mcpServers"]["context7"].is_object());
        assert!(cursor_v["mcpServers"].get("context7").is_none());
        assert_ne!(claude, cursor);
    }

    #[test]
    fn cursor_config_omits_gaviero_when_remote_extra_configured() {
        let mut synth = fixture_resolvable_shim(PathBuf::from("/tmp/wt"));
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://example.com/mcp/".into(),
            },
        });
        let body = cursor_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["mcpServers"]["semantic-scholar"].is_object());
        assert!(v["mcpServers"].get("gaviero").is_none());
    }

    #[test]
    fn cursor_config_url_extra_includes_empty_headers() {
        let mut synth = fixture_resolvable_shim(PathBuf::from("/tmp/wt"));
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://example.com/mcp/".into(),
            },
        });
        let body = cursor_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["mcpServers"]["semantic-scholar"]["url"].as_str().unwrap(),
            "https://example.com/mcp/"
        );
        assert!(v["mcpServers"]["semantic-scholar"]["headers"].is_object());
    }

    #[test]
    fn worktree_has_remote_mcp_urls_checks_both_mcp_json_files() {
        let dir = tempdir().unwrap();
        let claude_only = dir.path().join(".mcp.json");
        std::fs::write(
            &claude_only,
            r#"{"mcpServers":{"remote":{"url":"https://api.example.com/mcp/"}}}"#,
        )
        .unwrap();
        assert!(worktree_has_remote_mcp_urls(dir.path()));
    }

    #[test]
    fn merged_cursor_config_preserves_user_servers() {
        let dir = tempdir().unwrap();
        let cursor_path = dir.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(cursor_path.parent().unwrap()).unwrap();
        let existing = serde_json::json!({
            "mcpServers": {
                "user-cursor-server": { "command": "user-bin", "args": [] }
            }
        });
        std::fs::write(&cursor_path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        let synth = fixture_resolvable_shim(dir.path().to_path_buf());
        synthesize_for_worktree(&synth).unwrap();
        let body = std::fs::read_to_string(&cursor_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // User entry preserved; gaviero added; context7 stays out of Cursor config.
        assert!(v["mcpServers"]["user-cursor-server"].is_object());
        assert!(v["mcpServers"]["gaviero"].is_object());
        assert!(v["mcpServers"].get("context7").is_none());
        let claude_body = std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let claude_v: serde_json::Value = serde_json::from_str(&claude_body).unwrap();
        assert!(claude_v["mcpServers"]["context7"].is_object());
    }

    #[test]
    fn synth_disabled_writes_nothing() {
        let dir = tempdir().unwrap();
        let mut synth = fixture(dir.path().to_path_buf());
        synth.enabled = false;
        let files = synthesize_for_worktree(&synth).unwrap();
        assert!(files.is_empty());
        assert!(!dir.path().join(".mcp.json").exists());
    }

    #[test]
    fn claude_config_includes_context7_when_enabled() {
        let synth = fixture(PathBuf::from("/tmp/wt"));
        let body = claude_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["mcpServers"]["context7"]["command"].as_str().unwrap(),
            "npx"
        );
        assert_eq!(
            v["mcpServers"]["context7"]["args"][1].as_str().unwrap(),
            "@upstash/context7-mcp"
        );
    }

    #[test]
    fn claude_config_omits_context7_when_disabled() {
        let mut synth = fixture(PathBuf::from("/tmp/wt"));
        synth.context7.enabled = false;
        let body = claude_mcp_config_json(&synth).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["mcpServers"]["gaviero"].is_object());
        assert!(v["mcpServers"].get("context7").is_none());
    }

    #[test]
    fn codex_config_includes_context7_when_enabled() {
        let synth = fixture(PathBuf::from("/tmp/wt"));
        let body = codex_mcp_config_toml(&synth).unwrap();
        assert!(body.contains("[mcp_servers.context7]"));
        assert!(body.contains("command = \"npx\""));
        assert!(body.contains("\"@upstash/context7-mcp\""));
        assert!(body.contains("trust_level = \"trusted\""));
        let parsed: toml::Value = toml::from_str(&body).expect("codex config is valid TOML");
        let args = parsed["mcp_servers"]["context7"]["args"].as_array().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[1].as_str().unwrap(), "@upstash/context7-mcp");
    }

    #[test]
    fn codex_config_omits_context7_when_disabled() {
        let mut synth = fixture(PathBuf::from("/tmp/wt"));
        synth.context7.enabled = false;
        let body = codex_mcp_config_toml(&synth).unwrap();
        assert!(body.contains("[mcp_servers.gaviero]"));
        assert!(!body.contains("[mcp_servers.context7]"));
    }

    #[test]
    fn codex_config_emits_mcp_startup_and_tool_timeouts() {
        // Codex defaults are tight enough that slow remote MCP endpoints
        // (Wuilder proxies, OAuth-fronted servers) intermittently get their
        // tool calls cancelled by the tool layer. Pinning explicit
        // `startup_timeout_sec` / `tool_timeout_sec` values on every server
        // table gives the host one less surprising failure mode.
        let mut synth = fixture(PathBuf::from("/tmp/wt"));
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        let body = codex_mcp_config_toml(&synth).unwrap();
        let parsed: toml::Value = toml::from_str(&body).expect("codex config is valid TOML");
        for name in ["gaviero", "context7", "semantic-scholar"] {
            let server = &parsed["mcp_servers"][name];
            assert_eq!(
                server["startup_timeout_sec"].as_integer(),
                Some(CODEX_MCP_STARTUP_TIMEOUT_SECS as i64),
                "missing startup_timeout_sec for {name} in {body}",
            );
            assert_eq!(
                server["tool_timeout_sec"].as_integer(),
                Some(CODEX_MCP_TOOL_TIMEOUT_SECS as i64),
                "missing tool_timeout_sec for {name} in {body}",
            );
        }
    }

    #[test]
    fn codex_config_marks_user_extras_required_but_leaves_managed_optional() {
        // `required = true` makes codex block the session until the MCP
        // server is ready and surface a clean startup error if it never
        // reaches `ready`. Without it, codex starts the session in parallel
        // and the agent reports tool calls as "cancelled by the tool
        // layer" when it tries to use a server that's still `starting` /
        // `cancelled`. User-explicit extras flip this on so the failure is
        // loud; managed `gaviero` / `context7` stay opt-out so a missing
        // shim or `npx` outage doesn't crash the run.
        let mut synth = fixture(PathBuf::from("/tmp/wt"));
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        synth.extra_servers.push(ExtraMcpServer {
            name: "user-stdio".into(),
            transport: ExtraMcpTransport::Stdio {
                command: "my-tool".into(),
                args: vec!["--mode".into(), "prod".into()],
            },
        });
        let body = codex_mcp_config_toml(&synth).unwrap();
        let parsed: toml::Value = toml::from_str(&body).expect("codex config is valid TOML");

        assert_eq!(
            parsed["mcp_servers"]["semantic-scholar"]["required"].as_bool(),
            Some(true),
            "expected `required = true` on user-explicit URL server in {body}",
        );
        assert_eq!(
            parsed["mcp_servers"]["user-stdio"]["required"].as_bool(),
            Some(true),
            "expected `required = true` on user-explicit stdio server in {body}",
        );
        assert!(
            parsed["mcp_servers"]["gaviero"].get("required").is_none(),
            "managed gaviero server should not set `required`; got {body}",
        );
        assert!(
            parsed["mcp_servers"]["context7"].get("required").is_none(),
            "managed context7 server should not set `required`; got {body}",
        );
    }

    #[test]
    fn merged_config_includes_extra_url_server() {
        let dir = tempdir().unwrap();
        let mut synth = fixture(dir.path().to_path_buf());
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        synthesize_for_worktree(&synth).unwrap();
        let body = std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["mcpServers"]["semantic-scholar"]["url"].as_str().unwrap(),
            "https://scholar.example/mcp"
        );

        // Project-level .cursor/cli.json only accepts `permissions`. The
        // Cursor CLI's schema validator rejects every other root key —
        // including `version` and `editor`, despite what cursor.com/docs
        // implies. That validation runs on every `agent` invocation, so
        // any stray key kills the turn before our allowlist matters.
        let cli_path = dir.path().join(".cursor/cli.json");
        assert!(cli_path.exists(), "expected .cursor/cli.json for remote MCP");
        let cli: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cli_path).unwrap()).unwrap();
        let obj = cli.as_object().expect("cli.json is a JSON object");
        for forbidden in [
            "version",
            "editor",
            "approvalMode",
            "sandbox",
            "webFetchDomainAllowlist",
        ] {
            assert!(
                !obj.contains_key(forbidden),
                "cli.json must not contain `{forbidden}` (rejected by Cursor schema validator at the project level)",
            );
        }
        let allow = cli["permissions"]["allow"].as_array().unwrap();
        let allow_strs: Vec<&str> = allow.iter().filter_map(|v| v.as_str()).collect();
        // Cursor's MCP permission syntax is colon-separated (`Mcp(s:*)`),
        // NOT comma-separated. Verified live against agent
        // `2026.06.02-8c11d9f`: with the comma shape the tool call
        // returns `User rejected MCP: <name>` even under --approve-mcps.
        assert!(
            allow_strs.contains(&"Mcp(*:*)"),
            "missing wildcard Mcp(*:*); got {allow_strs:?}",
        );
        assert!(
            allow_strs.contains(&"Mcp(semantic-scholar:*)"),
            "missing per-server Mcp(name:*); got {allow_strs:?}",
        );
        assert!(allow_strs.contains(&"WebFetch(scholar.example)"));
        assert!(allow_strs.contains(&"WebFetch(*.scholar.example)"));
        // And — crucially — the old comma shape must NOT leak through.
        assert!(
            !allow_strs.iter().any(|s| s.contains(", ")),
            "legacy comma-form Mcp entry in {allow_strs:?}",
        );
    }

    #[test]
    fn cli_json_strips_legacy_invalid_root_keys() {
        // A prior gaviero version wrote `version` / `editor` / `approvalMode`
        // / `sandbox` / `webFetchDomainAllowlist` at the project level.
        // Cursor's schema validator rejects all of those with
        // `"unrecognized_keys"`, so re-synthesis must delete them rather
        // than leave them in place alongside our valid `permissions` block.
        let dir = tempdir().unwrap();
        let cursor_dir = dir.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        let cli_path = cursor_dir.join("cli.json");
        std::fs::write(
            &cli_path,
            r#"{
              "version": 1,
              "editor": { "vimMode": false },
              "approvalMode": "unrestricted",
              "sandbox": { "mode": "disabled" },
              "webFetchDomainAllowlist": ["old.example"],
              "permissions": {
                "allow": [
                  "Shell(ls)",
                  "Mcp(*, *)",
                  "Mcp(semantic-scholar, *)"
                ],
                "deny": []
              }
            }"#,
        )
        .unwrap();

        let mut synth = fixture(dir.path().to_path_buf());
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        synthesize_for_worktree(&synth).unwrap();

        let cli: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cli_path).unwrap()).unwrap();
        let obj = cli.as_object().unwrap();
        for forbidden in [
            "version",
            "editor",
            "approvalMode",
            "sandbox",
            "webFetchDomainAllowlist",
        ] {
            assert!(
                !obj.contains_key(forbidden),
                "legacy `{forbidden}` survived re-synthesis; would re-trip Cursor's project schema validator",
            );
        }
        // User-authored permissions survive, but legacy comma-form Mcp
        // entries (which Cursor's permission gate doesn't recognise) get
        // scrubbed so the new colon-form `Mcp(name:*)` actually applies.
        let allow_strs: Vec<&str> = cli["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(allow_strs.contains(&"Shell(ls)"));
        assert!(allow_strs.contains(&"Mcp(semantic-scholar:*)"));
        assert!(allow_strs.contains(&"Mcp(*:*)"));
        assert!(
            !allow_strs.iter().any(|s| s.contains(", ")),
            "legacy comma-form Mcp entry survived re-synthesis in {allow_strs:?}",
        );
    }

    #[test]
    fn is_legacy_comma_mcp_pattern_recognises_old_shape_only() {
        // Old (broken) shape we used to emit.
        assert!(is_legacy_comma_mcp_pattern("Mcp(*, *)"));
        assert!(is_legacy_comma_mcp_pattern("Mcp(semantic-scholar, *)"));
        // New correct shape — must NOT match.
        assert!(!is_legacy_comma_mcp_pattern("Mcp(*:*)"));
        assert!(!is_legacy_comma_mcp_pattern("Mcp(semantic-scholar:*)"));
        // Other permission entries — left alone.
        assert!(!is_legacy_comma_mcp_pattern("Shell(ls, -la)"));
        assert!(!is_legacy_comma_mcp_pattern("WebFetch(api.example.com)"));
        assert!(!is_legacy_comma_mcp_pattern(""));
    }

    #[test]
    fn host_from_mcp_url_parses_https() {
        assert_eq!(
            host_from_mcp_url("https://mcp.wuilder.com/semantic-scholar/token/"),
            Some("mcp.wuilder.com".to_string())
        );
    }

    fn codex_config_includes_extra_url_server() {
        let mut synth = fixture(PathBuf::from("/tmp/wt"));
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        let body = codex_mcp_config_toml(&synth).unwrap();
        assert!(body.contains("[mcp_servers.semantic-scholar]"));
        assert!(body.contains("url = \"https://scholar.example/mcp\""));
    }

    fn merged_claude_config_preserves_user_servers_and_adds_context7() {
        let dir = tempdir().unwrap();
        let claude_path = dir.path().join(".mcp.json");
        let existing = serde_json::json!({
            "mcpServers": {
                "user-server": { "command": "user-bin", "args": [] }
            }
        });
        std::fs::write(&claude_path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        let synth = fixture(dir.path().to_path_buf());
        synthesize_for_worktree(&synth).unwrap();
        let body = std::fs::read_to_string(&claude_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["mcpServers"]["user-server"].is_object());
        assert!(v["mcpServers"]["gaviero"].is_object());
        assert!(v["mcpServers"]["context7"].is_object());
    }

    #[test]
    fn codex_overrides_replay_every_server_table_from_synth_file() {
        let dir = tempdir().unwrap();
        let mut synth = fixture(dir.path().to_path_buf());
        synth.extra_servers.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://scholar.example/mcp".into(),
            },
        });
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let body = codex_mcp_config_toml(&synth).unwrap();
        std::fs::write(codex_dir.join("config.toml"), body).unwrap();
        let pairs = codex_mcp_overrides_from_config_file(&codex_dir.join("config.toml"));
        assert!(
            pairs
                .iter()
                .any(|p| p == r#"mcp_servers.gaviero.command="gaviero-mcp-shim""#),
            "missing gaviero.command in {pairs:?}",
        );
        assert!(
            pairs.iter().any(|p| p.starts_with("mcp_servers.gaviero.args=[")
                && p.contains("\"--socket\"")
                && p.contains(".gaviero/mcp.sock")),
            "missing or malformed gaviero.args in {pairs:?}",
        );
        assert!(
            pairs.iter().any(|p| p == r#"mcp_servers.context7.command="npx""#),
            "missing context7.command in {pairs:?}",
        );
        assert!(
            pairs
                .iter()
                .any(|p| p == r#"mcp_servers.semantic-scholar.url="https://scholar.example/mcp""#),
            "missing semantic-scholar.url in {pairs:?}",
        );
    }

    #[test]
    fn codex_overrides_return_empty_when_file_is_missing() {
        let dir = tempdir().unwrap();
        let pairs = codex_mcp_overrides_from_config_file(&dir.path().join(".codex/config.toml"));
        assert!(pairs.is_empty());
    }

    #[test]
    fn codex_overrides_return_empty_when_toml_is_malformed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is = not [ valid toml").unwrap();
        let pairs = codex_mcp_overrides_from_config_file(&path);
        assert!(pairs.is_empty());
    }

    #[test]
    fn codex_synth_has_any_mcp_detects_stdio_and_url_entries() {
        let dir = tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        // Stdio entry should count — the cancellation symptom hits stdio
        // servers too, not just HTTP ones.
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.gaviero]\ncommand = \"shim\"\nargs = []\n",
        )
        .unwrap();
        assert!(codex_synth_has_any_mcp(dir.path()));

        // Remote-only file: still detected.
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.semantic-scholar]\nurl = \"https://x/mcp/\"\n",
        )
        .unwrap();
        assert!(codex_synth_has_any_mcp(dir.path()));

        // Empty server table → no MCP.
        std::fs::write(codex_dir.join("config.toml"), "[mcp_servers]\n").unwrap();
        assert!(!codex_synth_has_any_mcp(dir.path()));

        // No file at all → no MCP.
        std::fs::remove_file(codex_dir.join("config.toml")).unwrap();
        assert!(!codex_synth_has_any_mcp(dir.path()));
    }

    #[test]
    fn codex_synth_has_remote_mcp_detects_http_servers() {
        let dir = tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.semantic-scholar]\nurl = \"https://scholar.example/mcp\"\n",
        )
        .unwrap();
        assert!(codex_synth_has_remote_mcp(dir.path()));
    }

    #[test]
    fn codex_synth_has_remote_mcp_returns_false_for_stdio_only() {
        let dir = tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.gaviero]\ncommand = \"gaviero-mcp-shim\"\nargs = [\"--socket\", \"/tmp/m\"]\n",
        )
        .unwrap();
        assert!(!codex_synth_has_remote_mcp(dir.path()));
    }

    #[test]
    fn codex_synth_has_remote_mcp_returns_false_when_file_missing() {
        let dir = tempdir().unwrap();
        assert!(!codex_synth_has_remote_mcp(dir.path()));
    }

    #[test]
    fn codex_synth_has_remote_mcp_returns_false_for_empty_url_string() {
        let dir = tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.weird]\nurl = \"   \"\n",
        )
        .unwrap();
        assert!(!codex_synth_has_remote_mcp(dir.path()));
    }

    #[test]
    fn codex_overrides_escape_special_characters_in_string_values() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[mcp_servers.weird]\ncommand = \"He said \\\"hi\\\"\\nbye\"\n",
        )
        .unwrap();
        let pairs = codex_mcp_overrides_from_config_file(&path);
        let cmd = pairs
            .iter()
            .find(|p| p.starts_with("mcp_servers.weird.command="))
            .expect("command override missing");
        // Inner quote and newline must be re-escaped so codex's TOML
        // parser sees them as literal content.
        assert!(cmd.contains("\\\""), "double quote not escaped in {cmd:?}");
        assert!(cmd.contains("\\n"), "newline not escaped in {cmd:?}");
    }
}
