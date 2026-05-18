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
//!
//! Both configs are per-worktree, not per-user, so swarm worktrees
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
    /// MCP entirely for one-shot `gaviero-cli --no-memory` runs.
    pub enabled: bool,
    /// Context7 docs-lookup MCP server defaults.
    pub context7: Context7Config,
}

impl Default for McpConfigSynth {
    fn default() -> Self {
        Self {
            worktree: PathBuf::new(),
            socket_path: PathBuf::new(),
            shim_binary: "gaviero-mcp-shim".to_string(),
            codex_trust: TrustConsent::Unknown,
            enabled: true,
            context7: Context7Config::default(),
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
    let mut servers = serde_json::Map::new();
    servers.insert("gaviero".to_string(), gaviero_server_entry(synth));
    if synth.context7.enabled {
        servers.insert("context7".to_string(), context7_server_entry(&synth.context7));
    }
    let body = serde_json::json!({ "mcpServers": serde_json::Value::Object(servers) });
    Ok(serde_json::to_string_pretty(&body).context("serialising .mcp.json")?)
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

/// Build the `.codex/config.toml` body for Codex.
///
/// Schema: `[mcp_servers.gaviero]` — one table per server. Codex reads
/// the first `.codex/config.toml` it finds walking up from the
/// worktree. Requires the worktree to be a trusted Codex project; we
/// write a `[projects.<worktree>]` trust stanza alongside so a one-
/// time consent grant propagates in-band.
pub fn codex_mcp_config_toml(synth: &McpConfigSynth) -> Result<String> {
    // Manually construct the TOML — toml's serializer doesn't like
    // the dotted-header shape Codex expects.
    let socket = synth.socket_path.to_string_lossy();
    let worktree = synth.worktree.to_string_lossy();
    let trust_value = match synth.codex_trust {
        TrustConsent::Granted => "trusted",
        _ => "untrusted",
    };
    let mut body = format!(
        "# Generated by gaviero (Tier A / A5). Do not edit — regenerated per swarm worktree.\n\n\
         [mcp_servers.gaviero]\n\
         command = {command:?}\n\
         args = [\"--socket\", {socket:?}]\n",
        command = synth.shim_binary,
        socket = socket,
    );
    if synth.context7.enabled {
        body.push_str(&format!(
            "\n[mcp_servers.context7]\n\
             command = {command:?}\n\
             args = {args}\n",
            command = synth.context7.command,
            args = toml_string_array(&synth.context7.args),
        ));
    }
    body.push_str(&format!(
        "\n[projects.{worktree:?}]\n\
         trust_level = {trust_value:?}\n",
        worktree = worktree,
        trust_value = trust_value,
    ));
    Ok(body)
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
    let claude_body = claude_mcp_config_json_merged(synth, &claude_path)?;
    write_if_changed(&claude_path, &claude_body)?;
    written.push(claude_path);

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

fn claude_mcp_config_json_merged(synth: &McpConfigSynth, path: &Path) -> Result<String> {
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
    servers_map.insert("gaviero".to_string(), gaviero_server_entry(synth));
    if synth.context7.enabled {
        servers_map.insert("context7".to_string(), context7_server_entry(&synth.context7));
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serialising merged .mcp.json")
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
            context7: Context7Config::default(),
        }
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
    fn synth_writes_claude_always_codex_only_on_granted() {
        let dir = tempdir().unwrap();
        let mut synth = fixture(dir.path().to_path_buf());

        // Trust unknown → only Claude config written.
        synth.codex_trust = TrustConsent::Unknown;
        let files = synthesize_for_worktree(&synth).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with(".mcp.json"));
        assert!(!dir.path().join(".codex/config.toml").exists());

        // Trust granted → both files.
        synth.codex_trust = TrustConsent::Granted;
        let files = synthesize_for_worktree(&synth).unwrap();
        assert_eq!(files.len(), 2);
        assert!(dir.path().join(".codex/config.toml").exists());
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
}
