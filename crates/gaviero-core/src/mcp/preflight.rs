//! MCP startup validation before swarm agents spawn.
//!
//! Structural checks (URL shape, shim on PATH) run by default. Optional
//! HTTP reachability probes are off unless `GAVIERO_MCP_PROBE_URLS=1`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::swarm::backend::shared::{is_codex_model, is_cursor_model};
use crate::swarm::plan::CompiledPlan;

use super::config_synth::{host_from_mcp_url, synth_has_remote_url_servers, McpConfigSynth};
use super::{ExtraMcpTransport, TrustConsent};

/// Whether any unit in the plan (including loop judges) resolves to a Codex backend.
pub fn plan_uses_codex(plan: &CompiledPlan, fallback_model: &str) -> bool {
    let mut units = plan
        .work_units_unordered()
        .into_iter()
        .chain(plan.loop_judge_units.iter());
    units.any(|u| unit_uses_codex(u, fallback_model))
}

fn unit_uses_codex(
    unit: &crate::swarm::models::WorkUnit,
    fallback_model: &str,
) -> bool {
    unit.model
        .as_deref()
        .map(is_codex_model)
        .unwrap_or_else(|| is_codex_model(fallback_model))
}

/// Fail when extra MCP servers are configured but Codex agents would not receive them.
pub fn validate_codex_trust_for_extras(
    synth: &McpConfigSynth,
    plan: &CompiledPlan,
    fallback_model: &str,
) -> Result<()> {
    if synth.extra_servers.is_empty() {
        return Ok(());
    }
    if !plan_uses_codex(plan, fallback_model) {
        return Ok(());
    }
    if matches!(synth.codex_trust, TrustConsent::Granted) {
        return Ok(());
    }
    let trust_state = match synth.codex_trust {
        TrustConsent::Unknown => "unknown",
        TrustConsent::Denied => "denied",
        TrustConsent::Granted => unreachable!(),
    };
    let names: Vec<_> = synth.extra_servers.iter().map(|s| s.name.as_str()).collect();
    bail!(
        "extra MCP server(s) [{names}] are configured but Codex trust is `{trust_state}` — \
         Codex agents in this plan will not load them (no `.codex/config.toml` is written).\n\
         Pass `--mcp-codex-trust granted` or set `mcp.gavieroServer.codexTrust = \"granted\"` \
         in workspace settings.",
        names = names.join(", "),
    );
}

/// Options for [`preflight_mcp`].
#[derive(Debug, Clone, Copy)]
pub struct PreflightOpts {
    /// When true, attempt a short HTTP GET against each remote extra URL.
    pub probe_urls: bool,
}

impl Default for PreflightOpts {
    fn default() -> Self {
        Self {
            probe_urls: std::env::var("GAVIERO_MCP_PROBE_URLS")
                .ok()
                .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes")),
        }
    }
}

/// Validate MCP wiring before config synthesis / agent spawn.
pub fn preflight_mcp(synth: &McpConfigSynth, opts: PreflightOpts) -> Result<()> {
    if !synth.enabled {
        return Ok(());
    }

    if synth.gaviero_enabled {
        ensure_shim_resolvable(&synth.shim_binary)?;
    }

    for extra in &synth.extra_servers {
        match &extra.transport {
            ExtraMcpTransport::Url { url } => validate_remote_url(&extra.name, url)?,
            ExtraMcpTransport::Stdio { command, args } => {
                if command.trim().is_empty() {
                    bail!("MCP server {:?}: stdio command must be non-empty", extra.name);
                }
                ensure_stdio_command_resolvable(command, args)?;
            }
        }
    }

    if opts.probe_urls {
        for extra in &synth.extra_servers {
            if let ExtraMcpTransport::Url { url } = &extra.transport {
                probe_remote_url(&extra.name, url)?;
            }
        }
    }

    Ok(())
}

fn validate_remote_url(name: &str, url: &str) -> Result<()> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!(
            "MCP server {name:?}: url must start with http:// or https:// (got {url:?})",
        );
    }
    if host_from_mcp_url(url).is_none() {
        bail!("MCP server {name:?}: url has no parseable host ({url:?})");
    }
    Ok(())
}

/// Whether `shim_binary` resolves to an executable (absolute path or on PATH).
pub fn shim_binary_resolvable(shim_binary: &str) -> bool {
    ensure_shim_resolvable(shim_binary).is_ok()
}

fn ensure_shim_resolvable(shim_binary: &str) -> Result<()> {
    let path = Path::new(shim_binary);
    if path.is_absolute() || shim_binary.contains('/') || shim_binary.contains('\\') {
        if path.is_file() {
            return Ok(());
        }
        bail!(
            "gaviero MCP shim not found at {} (set mcp.gavieroServer.shimBinary or install gaviero-mcp-shim)",
            path.display()
        );
    }
    let ok = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {shim_binary} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        bail!(
            "gaviero MCP shim {shim_binary:?} is not on PATH — \
             run `cargo install --path crates/gaviero-mcp-shim` or set an absolute shimBinary"
        );
    }
}

fn ensure_stdio_command_resolvable(command: &str, args: &[String]) -> Result<()> {
    let path = Path::new(command);
    if path.is_absolute() || command.contains('/') || command.contains('\\') {
        if path.is_file() {
            return Ok(());
        }
        bail!("stdio MCP command not found: {}", path.display());
    }
    let probe = if args.is_empty() {
        format!("command -v {command} >/dev/null 2>&1")
    } else {
        format!("command -v {command} >/dev/null 2>&1")
    };
    let ok = std::process::Command::new("sh")
        .args(["-c", &probe])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        bail!("stdio MCP command {command:?} is not on PATH");
    }
}

fn probe_remote_url(name: &str, url: &str) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("building HTTP client for MCP preflight")?;
    let resp = client
        .get(url.trim())
        .send()
        .with_context(|| format!("MCP server {name:?}: HTTP probe to {url}"))?;
    if resp.status().is_success() || resp.status().is_redirection() {
        return Ok(());
    }
    // Some MCP gateways only accept POST; treat 405 as reachable.
    if resp.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
        return Ok(());
    }
    bail!(
        "MCP server {name:?}: HTTP probe to {url} returned {}",
        resp.status()
    );
}

/// After [`super::synthesize_for_worktree`], ensure `.cursor/mcp.json` is lean
/// enough for remote streamable-HTTP MCP (no poison stdio servers).
pub fn validate_synthesized_cursor_remote_mcp(synth: &McpConfigSynth) -> Result<()> {
    if !synth_has_remote_url_servers(synth) {
        return Ok(());
    }
    let mcp_path = synth.worktree.join(".cursor/mcp.json");
    let body = std::fs::read_to_string(&mcp_path)
        .with_context(|| format!("reading {}", mcp_path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).context("parsing .cursor/mcp.json after synthesis")?;
    let servers = v
        .get("mcpServers")
        .and_then(|s| s.as_object())
        .ok_or_else(|| anyhow::anyhow!(".cursor/mcp.json missing mcpServers object"))?;

    for stale in ["context7", "gaviero"] {
        if servers.contains_key(stale) {
            bail!(
                ".cursor/mcp.json still lists {stale:?} alongside remote MCP URL server(s). \
                 A failing stdio server prevents Cursor from registering streamable HTTP MCP \
                 (ListMcpResources empty / network rejected). Delete {path} and re-run, or \
                 remove {stale} manually.",
                path = mcp_path.display(),
            );
        }
    }

    let has_url = servers.values().any(|entry| {
        entry
            .get("url")
            .and_then(|u| u.as_str())
            .is_some_and(|u| !u.trim().is_empty())
    });
    if !has_url {
        bail!(
            ".cursor/mcp.json has no remote url server after synthesis (expected at least one \
             from --mcp-url or SEMANTIC_SCHOLAR_MCP_URL)",
        );
    }

    let cli_path = synth.worktree.join(".cursor/cli.json");
    if !cli_path.is_file() {
        bail!(
            "missing {} — gaviero should write MCP/WebFetch permissions when remote URLs are \
             configured",
            cli_path.display()
        );
    }
    let cli: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&cli_path)
            .with_context(|| format!("reading {}", cli_path.display()))?,
    )
    .context("parsing .cursor/cli.json")?;
    let allow = cli
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !allow.iter().any(|s| s == "Mcp(*:*)") {
        bail!(
            ".cursor/cli.json permissions.allow is missing Mcp(*:*) (got {allow:?}); \
             Cursor headless runs reject MCP tool calls without colon-form allow patterns",
        );
    }
    Ok(())
}

/// Cursor agents in the plan (for diagnostics only).
pub fn plan_uses_cursor(plan: &CompiledPlan, fallback_model: &str) -> bool {
    let mut units = plan
        .work_units_unordered()
        .into_iter()
        .chain(plan.loop_judge_units.iter());
    units.any(|u| {
        u.model
            .as_deref()
            .map(is_cursor_model)
            .unwrap_or_else(|| is_cursor_model(fallback_model))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{ExtraMcpServer, McpConfigSynth};

    fn synth_with_extra_url(url: &str) -> McpConfigSynth {
        McpConfigSynth {
            extra_servers: vec![ExtraMcpServer {
                name: "probe".into(),
                transport: ExtraMcpTransport::Url {
                    url: url.to_string(),
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn validate_remote_url_rejects_non_http() {
        assert!(validate_remote_url("x", "ftp://example.com").is_err());
    }

    #[test]
    fn validate_remote_url_accepts_https() {
        validate_remote_url("x", "https://example.com/mcp").unwrap();
    }

    #[test]
    fn preflight_rejects_bad_url() {
        let synth = synth_with_extra_url("not-a-url");
        assert!(preflight_mcp(&synth, PreflightOpts { probe_urls: false }).is_err());
    }

    #[test]
    fn codex_trust_fails_when_extras_and_codex_in_plan() {
        use std::collections::HashMap;

        use crate::swarm::models::WorkUnit;
        use crate::swarm::plan::CompiledPlan;
        use crate::types::{FileScope, ModelTier, PrivacyLevel};

        let plan = CompiledPlan::from_work_units(
            vec![WorkUnit {
                id: "a".into(),
                description: "probe".into(),
                scope: FileScope {
                    owned_paths: vec![".".into()],
                    read_only_paths: vec![],
                    interface_contracts: HashMap::new(),
                },
                depends_on: vec![],
                backend: Default::default(),
                model: Some("codex:gpt-5.5".into()),
                effort: None,
                extra: vec![],
                tier: ModelTier::Cheap,
                privacy: PrivacyLevel::Public,
                coordinator_instructions: String::new(),
                estimated_tokens: 0,
                max_retries: 1,
                escalation_tier: None,
                read_namespaces: None,
                write_namespace: None,
                memory_importance: None,
                staleness_sources: vec![],
                memory_read_query: None,
                memory_read_limit: None,
                memory_write_content: None,
                impact_scope: false,
                context_callers_of: vec![],
                context_tests_for: vec![],
                context_depth: 2,
                extra_allowed_tools: vec![],
            }],
            None,
        );
        let mut synth = McpConfigSynth::default();
        synth.extra_servers = synth_with_extra_url("https://example.com/mcp").extra_servers;
        synth.codex_trust = TrustConsent::Unknown;
        assert!(validate_codex_trust_for_extras(&synth, &plan, "claude:sonnet").is_err());
    }

    #[test]
    fn validate_synthesized_cursor_remote_mcp_passes_after_synth() {
        use tempfile::tempdir;

        use super::super::synthesize_for_worktree;

        let dir = tempdir().unwrap();
        let mut synth = synth_with_extra_url("https://scholar.example/mcp/");
        synth.worktree = dir.path().to_path_buf();
        synth.extra_servers[0].name = "semantic-scholar".into();
        synthesize_for_worktree(&synth).unwrap();
        validate_synthesized_cursor_remote_mcp(&synth).unwrap();
    }

    #[test]
    fn validate_synthesized_cursor_remote_mcp_rejects_poisoned_context7() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cursor_dir = dir.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        std::fs::write(
            cursor_dir.join("mcp.json"),
            r#"{"mcpServers":{"context7":{"command":"npx","args":["-y","@upstash/context7-mcp"]},"semantic-scholar":{"url":"https://scholar.example/mcp/","headers":{}}}}"#,
        )
        .unwrap();
        let mut synth = synth_with_extra_url("https://scholar.example/mcp/");
        synth.worktree = dir.path().to_path_buf();
        assert!(validate_synthesized_cursor_remote_mcp(&synth).is_err());
    }
}
