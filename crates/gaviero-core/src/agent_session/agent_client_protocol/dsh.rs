//! `dsh` ACP launcher: command, args, profile, env, and the model overlay.
//!
//! `@deepseek-ai/dsh-acp` is a library, not a CLI. The published launcher is
//! `dsh --profile acp` from `@deepseek-ai/dsh`. That profile takes no model
//! flag: the model lives in the profile's plugin config, which the launcher
//! lets a `--patch <file>` overlay replace (see [`write_model_overlay`]).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::agent_session::tool_agent::config::ApiClientConfig;
use crate::util::spawn::{agent_command, resolve_program};
use crate::workspace::{Workspace, settings as S};

pub const DEFAULT_DSH_COMMAND: &str = "dsh";
pub const DEFAULT_DSH_PROFILE: &str = "acp";
pub const DSH_COMMAND_OVERRIDE_KEY: &str = "dsh_command";

#[derive(Debug, Clone)]
pub struct DshLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub profile: String,
    /// The in-tree `fake_acp_agent` test double: no API key, no model
    /// overlay (its argv is the scenario name).
    pub test_double: bool,
}

impl DshLaunchSpec {
    /// Precedence: DSL `extra { dsh_command … }` > `GAVIERO_DSH_COMMAND` >
    /// `providers.dsh.*` through the settings cascade of `root`.
    pub fn from_workspace_root(root: &Path, extra: &[(String, String)]) -> Self {
        if let Some((_, cmd)) = extra
            .iter()
            .find(|(k, _)| k == DSH_COMMAND_OVERRIDE_KEY)
            && !cmd.trim().is_empty()
        {
            return Self {
                test_double: is_fake_agent(cmd),
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
                test_double: is_fake_agent(&cmd),
                command: cmd,
                args: Vec::new(),
                profile: String::new(),
            };
        }
        // Single-folder cascade: `<root>/.gaviero/settings.json`, then the
        // user file, then defaults. Workspace-file settings of a multi-root
        // workspace are not visible from a bare root (same limit as every
        // other per-root reader).
        let ws = Workspace::single_folder(root.to_path_buf());
        let command = ws
            .resolve_setting(S::PROVIDERS_DSH_COMMAND, Some(root))
            .as_str()
            .unwrap_or(DEFAULT_DSH_COMMAND)
            .to_string();
        let args = ws
            .resolve_setting(S::PROVIDERS_DSH_ARGS, Some(root))
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let profile = ws
            .resolve_setting(S::PROVIDERS_DSH_PROFILE, Some(root))
            .as_str()
            .unwrap_or(DEFAULT_DSH_PROFILE)
            .to_string();
        Self {
            test_double: is_fake_agent(&command),
            command,
            args,
            profile,
        }
    }

    pub fn command_resolvable(&self) -> bool {
        Path::new(&self.command).is_file() || resolve_program(&self.command).is_some()
    }

    /// Launcher flags first (`--profile`, `--patch`), then the configured
    /// args, so a user `providers.dsh.args` list reaches the app.
    pub fn build_command(
        &self,
        workspace_root: &Path,
        model: Option<&str>,
    ) -> Result<tokio::process::Command> {
        let mut cmd = agent_command(&self.command);
        if !self.profile.is_empty() && !args_already_set_profile(&self.args) {
            cmd.arg("--profile").arg(&self.profile);
        }
        if !self.test_double {
            if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
                let overlay = write_model_overlay(workspace_root, model)?;
                cmd.arg("--patch").arg(&overlay);
            }
            let cfg = ApiClientConfig::resolve_deepseek(workspace_root, None, None).context(
                "dsh: missing DeepSeek API key (set DEEPSEEK_API_KEY or \
                 .gaviero/secrets.toml [deepseek] api_key)",
            )?;
            cmd.env("DEEPSEEK_API_KEY", cfg.api_key.expose());
        }
        cmd.args(&self.args);
        cmd.current_dir(workspace_root);
        cmd.env("NO_COLOR", "1");
        Ok(cmd)
    }
}

fn args_already_set_profile(args: &[String]) -> bool {
    args.iter().any(|a| a == "--profile" || a.starts_with("--profile="))
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

/// `--patch` overlay that sets the profile's model.
///
/// Verified against `dsh 0.1.5-rc.1 --profile acp --dump-config`
/// (2026-09-14): the composed tree carries `config.model` on the `acp`
/// plugin (`@deepseek-ai/dsh-acp`) and on `agent-default-model`; a
/// patch-list entry with the same `id` merges into that entry and its
/// `model` shows up in the composed dump. Not yet confirmed live against
/// the DeepSeek API (no key on the build machine).
pub fn model_overlay_yaml(model: &str) -> String {
    let quoted = yaml_quote(model);
    format!(
        "# generated by gaviero: dsh model overlay for `dsh --patch`; safe to delete\n\
         - id: acp\n  config:\n    provider: deepseek-official\n    model: {quoted}\n\
         - id: agent-default-model\n  config:\n    provider: deepseek-official\n    model: {quoted}\n"
    )
}

fn yaml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Write the overlay under `<root>/.gaviero/tmp/` (rewritten only when the
/// content changed) and return its path.
pub fn write_model_overlay(root: &Path, model: &str) -> Result<PathBuf> {
    let dir = root.join(".gaviero").join("tmp");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let safe: String = model
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let path = dir.join(format!("dsh-model-{safe}.yml"));
    let body = model_overlay_yaml(model);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&path, &body).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(path)
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

/// MCP servers for `session/new`. Live `dsh --profile acp` (0.1.5-rc.1,
/// 2026-09-14) advertises `mcpCapabilities.http` only, so no stdio shim
/// entry is offered; without a live HTTP listener the list is empty and the
/// session runs without gaviero tools (the prompt then omits the retrieval
/// stanza).
pub fn mcp_servers_for_session(root: &Path) -> Vec<serde_json::Value> {
    match http_mcp_server(root) {
        Some(http) => vec![http],
        None => Vec::new(),
    }
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
        assert!(!spec.test_double);
        assert!(args_already_set_profile(&["--profile".into(), "acp".into()]));
        assert!(args_already_set_profile(&["--profile=acp".into()]));
        assert!(!args_already_set_profile(&[]));
    }

    #[test]
    fn extra_override_wins_and_marks_the_test_double() {
        let dir = tempdir().unwrap();
        let spec = DshLaunchSpec::from_workspace_root(
            dir.path(),
            &[("dsh_command".into(), "C:/tmp/fake-acp-agent.exe".into())],
        );
        assert!(spec.test_double);
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
        assert!(!spec.test_double);
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
        let err = spec
            .build_command(dir.path(), Some("deepseek-v4-flash"))
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("DEEPSEEK_API_KEY") || msg.contains("secrets.toml"),
            "{msg}"
        );
        // The overlay is written before the key check, so the launch shape
        // is inspectable even without a key.
        assert!(dir.path().join(".gaviero/tmp/dsh-model-deepseek-v4-flash.yml").is_file());
    }

    #[test]
    fn model_overlay_targets_acp_and_default_model_plugins() {
        let yaml = model_overlay_yaml("deepseek-v4-pro");
        assert!(yaml.contains("- id: acp\n"));
        assert!(yaml.contains("- id: agent-default-model\n"));
        assert_eq!(yaml.matches("model: \"deepseek-v4-pro\"").count(), 2);
        let dir = tempdir().unwrap();
        let p = write_model_overlay(dir.path(), "a/b").unwrap();
        assert!(p.ends_with("dsh-model-a_b.yml"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), model_overlay_yaml("a/b"));
    }

    #[test]
    fn mcp_servers_without_descriptor_is_empty() {
        let dir = tempdir().unwrap();
        assert!(mcp_servers_for_session(dir.path()).is_empty());
    }
}
