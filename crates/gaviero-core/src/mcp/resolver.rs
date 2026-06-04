//! Resolve [`McpConfigSynth`] from workspace settings + CLI overrides.
//!
//! Workspace key `mcp.extraServers` holds a JSON array of extra servers
//! merged into every swarm worktree alongside `gaviero` and `context7`.
//! CLI flags `--mcp-url` / `--mcp-stdio` append or override by name.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::workspace::{Workspace, settings as S};
use crate::mcp::{Context7Config, ExtraMcpServer, ExtraMcpTransport, McpConfigSynth, TrustConsent};

/// CLI / caller overrides layered on workspace defaults.
#[derive(Debug, Clone, Default)]
pub struct McpConfigOverrides {
    /// `name=url` pairs from `--mcp-url`.
    pub extra_urls: Vec<(String, String)>,
    /// `name=command,arg1,arg2` from `--mcp-stdio`.
    pub extra_stdio: Vec<(String, String, Vec<String>)>,
    /// When set, beats `mcp.gavieroServer.codexTrust` from settings.
    pub codex_trust: Option<TrustConsent>,
    /// When `false`, skip all MCP config synthesis (`--no-mcp`).
    pub enabled: Option<bool>,
    /// When `false`, omit the in-process `gaviero` shim entry (no socket).
    pub gaviero_enabled: Option<bool>,
}

/// Parse `--mcp-url name=https://example/mcp`.
pub fn parse_mcp_url_flag(raw: &str) -> Result<(String, String)> {
    let (name, url) = raw
        .split_once('=')
        .with_context(|| format!("--mcp-url must be name=url, got {raw:?}"))?;
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() || url.is_empty() {
        bail!("--mcp-url name and url must be non-empty: {raw:?}");
    }
    Ok((name.to_string(), url.to_string()))
}

/// Parse `--mcp-stdio name=command,arg1,arg2`.
pub fn parse_mcp_stdio_flag(raw: &str) -> Result<(String, String, Vec<String>)> {
    let (name, rest) = raw
        .split_once('=')
        .with_context(|| format!("--mcp-stdio must be name=command[,args...], got {raw:?}"))?;
    let name = name.trim();
    if name.is_empty() {
        bail!("--mcp-stdio name must be non-empty: {raw:?}");
    }
    let parts: Vec<&str> = rest.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        bail!("--mcp-stdio command must be non-empty: {raw:?}");
    }
    let command = parts[0].to_string();
    let args = parts[1..].iter().map(|s| (*s).to_string()).collect();
    Ok((name.to_string(), command, args))
}

/// Parse `--mcp-codex-trust granted|denied|unknown`.
pub fn parse_mcp_codex_trust_flag(raw: &str) -> Result<TrustConsent> {
    match raw.trim().to_lowercase().as_str() {
        "granted" | "trusted" => Ok(TrustConsent::Granted),
        "denied" | "untrusted" => Ok(TrustConsent::Denied),
        "unknown" => Ok(TrustConsent::Unknown),
        other => bail!(
            "invalid --mcp-codex-trust {other:?} (expected granted, denied, or unknown)"
        ),
    }
}

/// Load `mcp.extraServers` from workspace settings.
pub fn extra_servers_from_workspace(workspace: &Workspace, root: Option<&Path>) -> Vec<ExtraMcpServer> {
    let val = workspace.resolve_setting(S::MCP_EXTRA_SERVERS, root);
    match parse_extra_servers_json(&val) {
        Ok(servers) => servers,
        Err(e) => {
            tracing::warn!(
                target: "mcp_config",
                error = %e,
                "ignoring invalid mcp.extraServers workspace setting"
            );
            Vec::new()
        }
    }
}

fn parse_extra_servers_json(val: &serde_json::Value) -> Result<Vec<ExtraMcpServer>> {
    let Some(arr) = val.as_array() else {
        if val.is_null() {
            return Ok(Vec::new());
        }
        bail!("mcp.extraServers must be a JSON array");
    };
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        out.push(parse_extra_server_object(item).with_context(|| {
            format!("mcp.extraServers[{i}]")
        })?);
    }
    Ok(out)
}

fn parse_extra_server_object(item: &serde_json::Value) -> Result<ExtraMcpServer> {
    let name = item
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing non-empty \"name\""))?
        .to_string();

    if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
        let url = url.trim();
        if url.is_empty() {
            bail!("\"url\" must be non-empty for server {name:?}");
        }
        return Ok(ExtraMcpServer {
            name,
            transport: ExtraMcpTransport::Url {
                url: url.to_string(),
            },
        });
    }

    let command = item
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("server {name:?} needs \"url\" or \"command\""))?
        .to_string();
    let args = item
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ExtraMcpServer {
        name,
        transport: ExtraMcpTransport::Stdio { command, args },
    })
}

/// Read remote MCP `url` entries from project-level `.cursor/mcp.json` or
/// `.mcp.json`. Skips servers Gaviero manages (`gaviero`, `context7`).
/// Used when operators configure Semantic Scholar in the IDE but omit
/// `--mcp-url` / `SEMANTIC_SCHOLAR_MCP_URL` on the CLI.
pub fn extra_urls_from_project_mcp_json(root: &Path) -> Vec<(String, String)> {
    const MANAGED: &[&str] = &["gaviero", "context7"];
    let mut out = Vec::new();
    for rel in [".cursor/mcp.json", ".mcp.json"] {
        let path = root.join(rel);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object()) else {
            continue;
        };
        for (name, entry) in servers {
            if MANAGED.contains(&name.as_str()) {
                continue;
            }
            let Some(url) = entry.get("url").and_then(|u| u.as_str()).map(str::trim) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            if out.iter().any(|(n, _)| n == name) {
                continue;
            }
            out.push((name.clone(), url.to_string()));
        }
    }
    out
}

fn merge_extra_servers(
    mut base: Vec<ExtraMcpServer>,
    overrides: &[ExtraMcpServer],
) -> Vec<ExtraMcpServer> {
    for ov in overrides {
        if let Some(pos) = base.iter().position(|s| s.name == ov.name) {
            base[pos] = ov.clone();
        } else {
            base.push(ov.clone());
        }
    }
    base
}

fn overrides_to_extra_servers(overrides: &McpConfigOverrides) -> Vec<ExtraMcpServer> {
    let mut out = Vec::new();
    for (name, url) in &overrides.extra_urls {
        out.push(ExtraMcpServer {
            name: name.clone(),
            transport: ExtraMcpTransport::Url { url: url.clone() },
        });
    }
    for (name, command, args) in &overrides.extra_stdio {
        out.push(ExtraMcpServer {
            name: name.clone(),
            transport: ExtraMcpTransport::Stdio {
                command: command.clone(),
                args: args.clone(),
            },
        });
    }
    out
}

pub fn resolve_context7_config(workspace: &Workspace, root: Option<&Path>) -> Context7Config {
    let defaults = Context7Config::default();
    let enabled = workspace
        .resolve_setting(S::MCP_CONTEXT7_ENABLED, root)
        .as_bool()
        .unwrap_or(defaults.enabled);
    let command = workspace
        .resolve_setting(S::MCP_CONTEXT7_COMMAND, root)
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or(defaults.command);
    let args = workspace
        .resolve_setting(S::MCP_CONTEXT7_ARGS, root)
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or(defaults.args);
    Context7Config {
        enabled,
        command,
        args,
    }
}

/// Build the synth struct used by [`super::synthesize_for_worktree`].
pub fn resolve_mcp_config_synth(
    workspace: &Workspace,
    root: &Path,
    socket_path: PathBuf,
    overrides: &McpConfigOverrides,
) -> McpConfigSynth {
    let enabled = overrides.enabled.unwrap_or_else(|| {
        workspace
            .resolve_setting(S::MCP_GAVIERO_ENABLED, Some(root))
            .as_bool()
            .unwrap_or(true)
    });
    let gaviero_enabled = overrides.gaviero_enabled.unwrap_or_else(|| {
        workspace
            .resolve_setting(S::MCP_GAVIERO_ENABLED, Some(root))
            .as_bool()
            .unwrap_or(true)
    });
    let shim_binary = workspace
        .resolve_setting(S::MCP_GAVIERO_SHIM_BINARY, Some(root))
        .as_str()
        .unwrap_or("gaviero-mcp-shim")
        .to_string();
    let codex_trust = overrides.codex_trust.unwrap_or_else(|| {
        match workspace
            .resolve_setting(S::MCP_GAVIERO_CODEX_TRUST, Some(root))
            .as_str()
            .unwrap_or("unknown")
        {
            "granted" | "trusted" => TrustConsent::Granted,
            "denied" | "untrusted" => TrustConsent::Denied,
            _ => TrustConsent::Unknown,
        }
    });

    let from_settings = extra_servers_from_workspace(workspace, Some(root));
    let from_cli = overrides_to_extra_servers(overrides);
    let extra_servers = merge_extra_servers(from_settings, &from_cli);

    McpConfigSynth {
        worktree: root.to_path_buf(),
        socket_path,
        shim_binary,
        codex_trust,
        enabled,
        gaviero_enabled,
        context7: resolve_context7_config(workspace, Some(root)),
        extra_servers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mcp_url_flag_splits_on_first_equals() {
        let (name, url) =
            parse_mcp_url_flag("semantic-scholar=https://example.com/mcp").unwrap();
        assert_eq!(name, "semantic-scholar");
        assert_eq!(url, "https://example.com/mcp");
    }

    #[test]
    fn parse_mcp_stdio_flag_splits_command_and_args() {
        let (name, cmd, args) = parse_mcp_stdio_flag("ctx=npx,-y,@upstash/context7-mcp").unwrap();
        assert_eq!(name, "ctx");
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["-y", "@upstash/context7-mcp"]);
    }

    #[test]
    fn parse_extra_servers_json_accepts_url_and_stdio() {
        let val = serde_json::json!([
            { "name": "semantic-scholar", "url": "https://scholar.example/mcp" },
            { "name": "local", "command": "my-bin", "args": ["--foo"] }
        ]);
        let servers = parse_extra_servers_json(&val).unwrap();
        assert_eq!(servers.len(), 2);
        assert!(matches!(
            &servers[0].transport,
            ExtraMcpTransport::Url { url } if url == "https://scholar.example/mcp"
        ));
        assert!(matches!(
            &servers[1].transport,
            ExtraMcpTransport::Stdio { command, args }
            if command == "my-bin" && args == &["--foo".to_string()]
        ));
    }

    #[test]
    fn extra_urls_from_project_mcp_json_reads_cursor_config() {
        let dir = tempfile::tempdir().unwrap();
        let cursor_dir = dir.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        std::fs::write(
            cursor_dir.join("mcp.json"),
            r#"{"mcpServers":{"semantic-scholar":{"url":"https://example/mcp/"},"gaviero":{"command":"shim"}}}"#,
        )
        .unwrap();
        let urls = extra_urls_from_project_mcp_json(dir.path());
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].0, "semantic-scholar");
        assert_eq!(urls[0].1, "https://example/mcp/");
    }

    #[test]
    fn cli_overrides_replace_workspace_server_by_name() {
        let ws = Workspace::single_folder(PathBuf::from("/tmp/ws"));
        let mut merged = extra_servers_from_workspace(&ws, None);
        merged.push(ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://from-settings".into(),
            },
        });
        let cli = vec![ExtraMcpServer {
            name: "semantic-scholar".into(),
            transport: ExtraMcpTransport::Url {
                url: "https://from-cli".into(),
            },
        }];
        let out = merge_extra_servers(merged, &cli);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0].transport,
            ExtraMcpTransport::Url { url } if url == "https://from-cli"
        ));
    }
}
