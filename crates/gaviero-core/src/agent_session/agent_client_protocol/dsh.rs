//! `dsh` ACP launcher: command, args, profile, env, version probe.
//!
//! `@deepseek-ai/dsh-acp` is a library, not a CLI. The published launcher is
//! `dsh --profile acp` from `@deepseek-ai/dsh`.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::Mutex as StdMutex;

use anyhow::{Context, Result, anyhow};

use crate::agent_session::tool_agent::config::ApiClientConfig;
use crate::mcp::resolver::resolve_shim_binary;
use crate::util::spawn::{agent_command, resolve_program};

pub const DEFAULT_DSH_COMMAND: &str = "dsh";
pub const DEFAULT_DSH_PROFILE: &str = "acp";
pub const DSH_COMMAND_SETTING: &str = "providers.dsh.command";
pub const DSH_ARGS_SETTING: &str = "providers.dsh.args";
pub const DSH_PROFILE_SETTING: &str = "providers.dsh.profile";
pub const DSH_COMMAND_OVERRIDE_KEY: &str = "dsh_command";

#[derive(Debug, Clone)]
pub struct DshLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub profile: String,
    pub skip_api_key: bool,
}

impl DshLaunchSpec {
    pub fn from_workspace_root(root: &Path, extra: &[(String, String)]) -> Self {
        if let Some((_, cmd)) = extra
            .iter()
            .find(|(k, _)| k == DSH_COMMAND_OVERRIDE_KEY)
            && !cmd.trim().is_empty()
        {
            return Self {
                skip_api_key: is_fake_agent(cmd),
                command: cmd.clone(),
                args: extra
                    .iter()
                    .find(|(k, _)| k == "dsh_args")
                    .map(|(_, v)| split_args(v))
                    .unwrap_or_default(),
                profile: extra
                    .iter()
                    .find(|(k, _)| k == "dsh_profile")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
            };
        }
        if let Ok(cmd) = std::env::var("GAVIERO_DSH_COMMAND")
            && !cmd.trim().is_empty()
        {
            return Self {
                skip_api_key: is_fake_agent(&cmd),
                command: cmd,
                args: Vec::new(),
                profile: String::new(),
            };
        }
        let settings = read_settings(root);
        let command = settings
            .get("providers")
            .and_then(|p| p.get("dsh"))
            .and_then(|d| d.get("command"))
            .and_then(|c| c.as_str())
            .unwrap_or(DEFAULT_DSH_COMMAND)
            .to_string();
        let args = settings
            .get("providers")
            .and_then(|p| p.get("dsh"))
            .and_then(|d| d.get("args"))
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let profile = settings
            .get("providers")
            .and_then(|p| p.get("dsh"))
            .and_then(|d| d.get("profile"))
            .and_then(|p| p.as_str())
            .unwrap_or(DEFAULT_DSH_PROFILE)
            .to_string();
        Self {
            skip_api_key: is_fake_agent(&command),
            command,
            args,
            profile,
        }
    }

    pub fn command_resolvable(&self) -> bool {
        Path::new(&self.command).is_file() || resolve_program(&self.command).is_some()
    }

    pub fn build_command(&self, workspace_root: &Path) -> Result<tokio::process::Command> {
        let mut cmd = agent_command(&self.command);
        cmd.args(&self.args);
        if !self.profile.is_empty() && !args_already_set_profile(&self.args) {
            cmd.arg("--profile").arg(&self.profile);
        }
        if !self.skip_api_key {
            let cfg = ApiClientConfig::resolve_deepseek(workspace_root, None, None).with_context(
                || {
                    format!(
                        "dsh: missing DeepSeek API key (set DEEPSEEK_API_KEY or \
                         .gaviero/secrets.toml [deepseek] api_key)"
                    )
                },
            )?;
            cmd.env("DEEPSEEK_API_KEY", cfg.api_key.expose());
        }
        cmd.current_dir(workspace_root);
        cmd.env("NO_COLOR", "1");
        Ok(cmd)
    }
}

fn args_already_set_profile(args: &[String]) -> bool {
    args.windows(2).any(|w| w[0] == "--profile")
        || args.iter().any(|a| a == "--profile" || a.starts_with("--profile="))
}

fn is_fake_agent(command: &str) -> bool {
    let name = Path::new(command)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    name.contains("fake-acp") || name.contains("fake_acp")
}

fn split_args(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn read_settings(root: &Path) -> serde_json::Value {
    let path = root.join(".gaviero").join("settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

static VERSION_CACHE: OnceLock<StdMutex<Option<String>>> = OnceLock::new();

/// Best-effort `dsh --version`, cached per process.
pub fn probe_version(command: &str) -> Option<String> {
    let cache = VERSION_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some(v) = guard.as_ref()
    {
        return Some(v.clone());
    }
    let output = std::process::Command::from(crate::util::spawn::agent_command_std(command))
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(text.clone());
    }
    Some(text)
}

/// Stdio MCP server entry for `session/new` when HTTP is off.
pub fn stdio_mcp_server(_root: &Path) -> serde_json::Value {
    let shim = resolve_shim_binary("gaviero-mcp-shim", crate::mcp::sibling_shim_path());
    serde_json::json!({
        "name": "gaviero",
        "command": shim,
        "args": ["--resolve"],
        "env": []
    })
}

/// HTTP MCP server entry from the endpoint descriptor, if live.
pub fn http_mcp_server(root: &Path) -> Option<serde_json::Value> {
    let desc = crate::mcp::read_descriptor(&crate::mcp::McpEndpointDescriptor::path(root)).ok()?;
    let url = desc.http_url?;
    let token_path = desc.http_token_path?;
    let token = std::fs::read_to_string(token_path).ok()?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "type": "http",
        "name": "gaviero",
        "url": url,
        "headers": [{ "name": "Authorization", "value": format!("Bearer {token}") }]
    }))
}

/// ACP HTTP server entry: `HttpHeader[]`, an **array** of `{name, value}`.
///
/// Deliberately not the object map that the Claude `.mcp.json` and Cursor
/// `.cursor/mcp.json` carry — those are vendor file formats, this is the ACP
/// wire shape. See [`http_mcp_server`] for the same reason.
fn http_entry(name: &str, url: &str, headers: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "type": "http",
        "name": name,
        "url": url,
        "headers": headers,
    })
}

/// Whether `servers` registers gaviero's **own** MCP server.
///
/// Callers need this separately from "the list is non-empty": context7 and
/// `extraServers` can be present while gaviero's own endpoint is down, and the
/// `exposedTools` gate describes gaviero's retrieval tools specifically.
pub fn registers_gaviero(servers: &[serde_json::Value]) -> bool {
    servers
        .iter()
        .any(|s| s.get("name").and_then(serde_json::Value::as_str) == Some("gaviero"))
}

/// MCP servers to hand dsh on `session/new`, in registration order.
///
/// **Declared transport limit (provider-parity decision 1).** Live
/// `dsh --profile acp` (0.1.5-rc.1, 2026-09-14) advertises
/// `mcpCapabilities.http` only. Every entry here is therefore HTTP; a stdio
/// entry is skipped rather than sent, so `session/new` is never rejected for
/// an unadvertised transport.
///
/// **Parity (decision E).** The same three source servers the file-based
/// providers register, each behind the *same* `mcp.permissions` registration
/// gate (`McpPermissions::server_allowed`) that `synthesize_for_worktree`
/// applies — which is the provider-independent hard gate, so "same MCP
/// servers, same permissions" holds for dsh:
///
/// 1. `gaviero` — only when a live HTTP endpoint exists ([`http_mcp_server`],
///    i.e. `.gaviero/mcp-url` plus token), else omitted.
/// 2. `context7` — only when `mcp.context7.enabled` **and** it resolves to a
///    URL. The stdio `npx` fallback cannot be hosted by dsh, so a stdio-mode
///    context7 is silently absent here; that is the one deliberate asymmetry.
/// 3. `mcp.extraServers` — only those declared with a `url` transport. A
///    `command` extra is stdio and likewise cannot be hosted.
///
/// Resolution is **root-scoped**: [`Workspace::single_folder`] reads
/// `<root>/.gaviero/settings.json`, mirroring the host's cascade for the same
/// root. Threading the host's resolved values instead would be preferable in
/// principle (see the `SessionConstruction::mcp_server` note), but the ACP
/// session is constructed below `create_session` where no `Workspace` is in
/// scope, and this keeps one call site rather than a new session field.
///
/// Separately: dsh's `session/new` carries **no tool list and no permission
/// policy** (`agent_client_protocol/mod.rs`), so `agent.availableTools` is
/// *structurally unenforced* for dsh. That is recorded as
/// `ToolEnforcement::Unenforced` in the capability table, not attempted here.
pub fn mcp_servers_for_session(root: &Path) -> Vec<serde_json::Value> {
    let mut servers = Vec::new();
    if let Some(http) = http_mcp_server(root) {
        servers.push(http);
    }

    let workspace = crate::workspace::Workspace::single_folder(root.to_path_buf());
    let permissions = crate::mcp::resolve_mcp_permissions(&workspace, Some(root));

    let ctx7 = crate::mcp::resolve_context7_config(&workspace, Some(root));
    if ctx7.enabled
        && let Some(url) = ctx7.remote_url()
        && permissions.server_allowed("context7")
    {
        servers.push(http_entry("context7", url, Vec::new()));
    }

    for extra in crate::mcp::extra_servers_from_workspace(&workspace, Some(root)) {
        if !permissions.server_allowed(&extra.name) {
            continue;
        }
        if let crate::mcp::ExtraMcpTransport::Url { url } = extra.transport {
            servers.push(http_entry(&extra.name, &url, Vec::new()));
        }
    }

    servers
}

pub fn missing_key_error() -> anyhow::Error {
    anyhow!(
        "dsh: no DeepSeek API key. Set DEEPSEEK_API_KEY or add `[deepseek] api_key` \
         to <workspace>/.gaviero/secrets.toml"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_command_is_dsh_profile_acp() {
        let dir = tempdir().unwrap();
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.command, "dsh");
        assert_eq!(spec.profile, "acp");
        assert!(spec.args.is_empty());
        assert!(args_already_set_profile(&["--profile".into(), "acp".into()]));
        assert!(!args_already_set_profile(&[]));
    }

    #[test]
    fn extra_override_wins_and_skips_key_for_fake() {
        let dir = tempdir().unwrap();
        let spec = DshLaunchSpec::from_workspace_root(
            dir.path(),
            &[("dsh_command".into(), "C:/tmp/fake-acp-agent.exe".into())],
        );
        assert!(spec.skip_api_key);
        assert!(spec.command.contains("fake-acp-agent"));
        assert!(spec.profile.is_empty());
    }

    #[test]
    fn settings_file_fills_command_args_profile() {
        let dir = tempdir().unwrap();
        let gav = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gav).unwrap();
        std::fs::write(
            gav.join("settings.json"),
            r#"{"providers":{"dsh":{"command":"my-dsh","args":["--x"],"profile":"p1"}}}"#,
        )
        .unwrap();
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.command, "my-dsh");
        assert_eq!(spec.args, vec!["--x".to_string()]);
        assert_eq!(spec.profile, "p1");
        assert!(!spec.skip_api_key);
    }

    #[test]
    fn missing_api_key_is_a_clear_error() {
        if std::env::var_os("DEEPSEEK_API_KEY").is_some()
            || std::env::var_os("GAVIERO_DSH_COMMAND").is_some()
        {
            return;
        }
        let dir = tempdir().unwrap();
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        let err = spec.build_command(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("DEEPSEEK_API_KEY") || msg.contains("secrets.toml"),
            "{msg}"
        );
    }

    #[test]
    fn mcp_servers_http_only_without_descriptor_is_empty() {
        let dir = tempdir().unwrap();
        assert!(mcp_servers_for_session(dir.path()).is_empty());
        let stdio = stdio_mcp_server(dir.path());
        assert_eq!(stdio["name"], "gaviero");
        assert!(stdio.get("command").and_then(|c| c.as_str()).is_some());
    }

    /// Write `<root>/.gaviero/settings.json` with `body`.
    fn write_settings(root: &Path, body: &str) {
        let gav = root.join(".gaviero");
        std::fs::create_dir_all(&gav).unwrap();
        std::fs::write(gav.join("settings.json"), body).unwrap();
    }

    fn names(servers: &[serde_json::Value]) -> Vec<String> {
        servers
            .iter()
            .filter_map(|s| s.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    }

    /// Decision E: with no gaviero endpoint but context7 on, dsh still gets
    /// context7 — and `registers_gaviero` must say *false*, so the
    /// `exposedTools` gate is not fooled by a non-empty list.
    #[test]
    fn context7_http_is_registered_without_a_gaviero_endpoint() {
        let dir = tempdir().unwrap();
        write_settings(dir.path(), r#"{"mcp":{"context7":{"enabled":true}}}"#);
        let servers = mcp_servers_for_session(dir.path());
        assert_eq!(names(&servers), vec!["context7"]);
        let ctx7 = &servers[0];
        assert_eq!(ctx7["type"], "http");
        let url = ctx7["url"].as_str().expect("context7 url");
        assert!(url.starts_with("https://"), "{url}");
        // ACP shape: an array, not the vendor object map.
        assert!(ctx7["headers"].is_array(), "{ctx7}");
        assert!(!registers_gaviero(&servers));
    }

    /// The stdio fallback cannot be hosted by dsh (HTTP-only capability), so
    /// it is skipped rather than sent as an unadvertised transport.
    #[test]
    fn context7_stdio_fallback_is_skipped() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{"context7":{"enabled":true,"url":""}}}"#,
        );
        assert!(mcp_servers_for_session(dir.path()).is_empty());
    }

    /// Disabled by default: no workspace opt-in, no entry.
    #[test]
    fn context7_is_opt_in() {
        let dir = tempdir().unwrap();
        write_settings(dir.path(), r#"{"mcp":{"context7":{"enabled":false}}}"#);
        assert!(mcp_servers_for_session(dir.path()).is_empty());
    }

    /// Decision E: URL extras register, stdio extras are skipped.
    #[test]
    fn extra_servers_register_url_and_skip_stdio() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{"extraServers":[
                {"name":"semantic-scholar","url":"https://example.test/mcp"},
                {"name":"local-thing","command":"some-server","args":["--x"]}
            ]}}"#,
        );
        let servers = mcp_servers_for_session(dir.path());
        assert_eq!(names(&servers), vec!["semantic-scholar"]);
        assert_eq!(servers[0]["type"], "http");
    }

    /// "Same permissions": `mcp.permissions` is the provider-independent
    /// registration gate, so a denied server never reaches `session/new`.
    #[test]
    fn mcp_permissions_deny_removes_the_server() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{
                "context7":{"enabled":true},
                "extraServers":[{"name":"semantic-scholar","url":"https://example.test/mcp"}],
                "permissions":{"deny":["context7:*","semantic-scholar:*"]}
            }}"#,
        );
        assert!(mcp_servers_for_session(dir.path()).is_empty());
    }

    /// An `allow` list that never names a server excludes it, matching
    /// `server_allowed` semantics on the file-based providers.
    #[test]
    fn mcp_permissions_allow_list_excludes_unnamed_servers() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{
                "context7":{"enabled":true},
                "extraServers":[{"name":"semantic-scholar","url":"https://example.test/mcp"}],
                "permissions":{"allow":["gaviero:*"]}
            }}"#,
        );
        assert!(mcp_servers_for_session(dir.path()).is_empty());
    }
}
