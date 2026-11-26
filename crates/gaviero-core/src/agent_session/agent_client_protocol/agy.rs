//! `agy` ACP launcher (Antigravity CLI): command, args, hook environment.
//!
//! `agy acp` serves the Agent Client Protocol on stdio. gaviero always adds
//! `--disable-telemetry` and hands the hook it registers in
//! `<folder>/.agents/hooks.json` (`gaviero-cli --hook agy:pre-tool-use`) two
//! variables through agy's environment, which agy passes on to hook commands:
//! the workspace root and the conversation's `/add-dir` read grants.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::util::spawn::{agent_command, resolve_program};

pub const DEFAULT_AGY_COMMAND: &str = "agy";
pub const AGY_COMMAND_SETTING: &str = "providers.agy.command";
pub const AGY_ARGS_SETTING: &str = "providers.agy.args";
/// Test override (`AcpClientSession::with_extra`), like dsh's `dsh_command`.
pub const AGY_COMMAND_OVERRIDE_KEY: &str = "agy_command";
pub const AGY_ARGS_OVERRIDE_KEY: &str = "agy_args";
pub const AGY_COMMAND_ENV: &str = "GAVIERO_AGY_COMMAND";
/// `/add-dir` read grants for the agy hook, joined with the platform path
/// separator (`std::env::join_paths`).
pub const HOOK_READ_FOLDERS_ENV: &str = "GAVIERO_HOOK_READ_FOLDERS";

const DISABLE_TELEMETRY: &str = "--disable-telemetry";

/// Which ACP agent an `AcpClientSession` drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpAgentKind {
    Dsh,
    Agy,
}

impl AcpAgentKind {
    /// `"agy"` → [`AcpAgentKind::Agy`]; anything else is dsh, the ACP agent
    /// this client was written for.
    pub fn from_provider(provider: &str) -> Self {
        if provider == "agy" { Self::Agy } else { Self::Dsh }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dsh => "dsh",
            Self::Agy => "agy",
        }
    }

    pub fn model_prefix(self) -> &'static str {
        match self {
            Self::Dsh => "dsh:",
            Self::Agy => "agy:",
        }
    }
}

/// The in-tree `fake-acp-agent` test double, by file stem (same rule as
/// dsh's launcher).
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

#[derive(Debug, Clone)]
pub struct AgyLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    /// The in-tree `fake-acp-agent` test double: no `acp` subcommand, no
    /// telemetry flag.
    pub fake: bool,
}

impl AgyLaunchSpec {
    /// Resolved like `DshLaunchSpec::from_workspace_root`: the `agy_command`
    /// extra, then `GAVIERO_AGY_COMMAND`, then `providers.agy.*` settings.
    pub fn from_workspace_root(root: &Path, extra: &[(String, String)]) -> Self {
        if let Some((_, cmd)) = extra.iter().find(|(k, _)| k == AGY_COMMAND_OVERRIDE_KEY)
            && !cmd.trim().is_empty()
        {
            return Self {
                fake: is_fake_agent(cmd),
                command: cmd.clone(),
                args: extra
                    .iter()
                    .find(|(k, _)| k == AGY_ARGS_OVERRIDE_KEY)
                    .map(|(_, v)| split_args(v))
                    .unwrap_or_default(),
            };
        }
        if let Ok(cmd) = std::env::var(AGY_COMMAND_ENV)
            && !cmd.trim().is_empty()
        {
            return Self {
                fake: is_fake_agent(&cmd),
                command: cmd,
                args: Vec::new(),
            };
        }
        let settings = crate::workspace::LayeredSettings::for_folder(root);
        let command = settings
            .get(AGY_COMMAND_SETTING)
            .and_then(|c| c.as_str())
            .filter(|c| !c.trim().is_empty())
            .unwrap_or(DEFAULT_AGY_COMMAND)
            .to_string();
        let args = settings
            .get(AGY_ARGS_SETTING)
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            fake: is_fake_agent(&command),
            command,
            args,
        }
    }

    pub fn command_resolvable(&self) -> bool {
        Path::new(&self.command).is_file() || resolve_program(&self.command).is_some()
    }

    /// `acp`, the configured args, then `--disable-telemetry` unless the args
    /// already carry it. The fake agent gets exactly its args.
    pub fn launch_args(&self) -> Vec<OsString> {
        if self.fake {
            return self.args.iter().map(Into::into).collect();
        }
        let mut out: Vec<OsString> = vec!["acp".into()];
        out.extend(self.args.iter().map(Into::into));
        if !self.args.iter().any(|a| a == DISABLE_TELEMETRY) {
            out.push(DISABLE_TELEMETRY.into());
        }
        out
    }

    /// The spawn command, through [`agent_command`] so the egress proxy
    /// environment applies. agy passes its environment on to hook commands,
    /// which is how the hook learns the workspace root and the `/add-dir`
    /// grants of this conversation.
    pub fn build_command(
        &self,
        workspace_root: &Path,
        session_read_folders: &[PathBuf],
    ) -> Result<tokio::process::Command> {
        let mut cmd = agent_command(&self.command);
        cmd.args(self.launch_args());
        cmd.current_dir(workspace_root);
        cmd.env("NO_COLOR", "1");
        cmd.env(crate::agent_guard::hook::HOOK_ROOT_ENV, workspace_root);
        if !session_read_folders.is_empty() {
            let joined = std::env::join_paths(session_read_folders)
                .context("agy: joining /add-dir folders for the hook environment")?;
            cmd.env(HOOK_READ_FOLDERS_ENV, joined);
        }
        Ok(cmd)
    }
}

/// `agy:<id>` specs agy offers over ACP, for the model picker.
///
/// agy only lists its ACP catalog inside a session (`models.availableModels`
/// on `session/new`), and it is account-specific: `agy models` has printed
/// ids that `session/set_model` then rejected over ACP. So this runs a short
/// handshake — `initialize`, `session/new` in the temp dir, `session/close` —
/// with no model call. Cached for the process; on any failure (agy missing,
/// not signed in, 20 s timeout) it offers
/// [`crate::swarm::backend::agy::AGY_DEFAULT_MODEL`].
pub fn discover_agy_model_options() -> Vec<String> {
    static CACHE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            query_agy_models().unwrap_or_else(|| {
                vec![format!(
                    "agy:{}",
                    crate::swarm::backend::agy::AGY_DEFAULT_MODEL
                )]
            })
        })
        .clone()
}

fn query_agy_models() -> Option<Vec<String>> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let tmp = std::env::temp_dir();
    let spec = AgyLaunchSpec::from_workspace_root(&tmp, &[]);
    if spec.fake {
        return None;
    }
    let mut child = crate::util::spawn::agent_command_std(&spec.command)
        .args(spec.launch_args())
        .current_dir(&tmp)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel::<serde_json::Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line)
                && tx.send(msg).is_err()
            {
                break;
            }
        }
    });
    let mut send = |msg: serde_json::Value| -> std::io::Result<()> {
        writeln!(stdin, "{msg}")?;
        stdin.flush()
    };
    // The response to request `id`, skipping notifications and other ids.
    let wait_for = |id: u64, within: Duration| -> Option<serde_json::Value> {
        let deadline = Instant::now() + within;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(msg)
                    if msg.get("method").is_none()
                        && msg.get("id").and_then(serde_json::Value::as_u64) == Some(id) =>
                {
                    return Some(msg);
                }
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    };

    let cwd = tmp.to_string_lossy().into_owned();
    let result = (|| {
        send(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                "clientInfo": { "name": "gaviero", "version": env!("CARGO_PKG_VERSION") }
            }
        }))
        .ok()?;
        // agy answers `session/new` with -32000 "Authentication required"
        // until its silent sign-in has run, which `initialize` waits for.
        wait_for(1, Duration::from_secs(20))?;
        send(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "session/new",
            "params": { "cwd": cwd, "mcpServers": [] }
        }))
        .ok()?;
        let reply = wait_for(2, Duration::from_secs(20))?;
        reply.get("result").cloned()
    })();
    if let Some(session_id) = result
        .as_ref()
        .and_then(|r| r.get("sessionId"))
        .and_then(serde_json::Value::as_str)
    {
        let closed = send(serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "session/close",
            "params": { "sessionId": session_id }
        }));
        if closed.is_ok() {
            let _ = wait_for(3, Duration::from_secs(2));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let models = parse_agy_available_models(result.as_ref()?);
    (!models.is_empty()).then_some(models)
}

/// `models.availableModels[].modelId` of a `session/new` result as
/// `agy:<id>` specs, keeping only ids of `[A-Za-z0-9_.-]`.
pub fn parse_agy_available_models(session_new: &serde_json::Value) -> Vec<String> {
    session_new
        .get("models")
        .and_then(|m| m.get("availableModels"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("modelId").and_then(serde_json::Value::as_str))
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        })
        .map(|id| format!("agy:{id}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use tempfile::tempdir;

    #[test]
    fn parses_available_models_into_agy_specs() {
        let session_new = serde_json::json!({
            "sessionId": "s",
            "models": {
                "availableModels": [
                    { "modelId": "gemini-4-neon-xhigh", "name": "Gemini 4 Neon (XHigh)" },
                    { "modelId": "gemini-3.8-flash-low", "name": "Gemini 3.8 Flash (Low)" }
                ],
                "currentModelId": "gemini-4-carbon-medium"
            }
        });
        assert_eq!(
            parse_agy_available_models(&session_new),
            ["agy:gemini-4-neon-xhigh", "agy:gemini-3.8-flash-low"]
        );
        assert!(parse_agy_available_models(&serde_json::json!({})).is_empty());
    }

    /// Live: the real `agy acp` handshake lists its catalog (run with
    /// `--ignored` on a machine where agy is installed and signed in).
    #[test]
    #[ignore = "needs agy on PATH, signed in"]
    fn live_discovery_lists_the_acp_catalog() {
        // The catalog is account-specific (which Claude ids agy offers over
        // ACP changed with the signed-in Google Cloud project), so only its
        // shape is asserted.
        let models = discover_agy_model_options();
        assert!(models.len() > 1, "expected the ACP catalog, got {models:?}");
        assert!(models.iter().all(|m| m.starts_with("agy:")), "{models:?}");
    }

    #[test]
    fn rejects_odd_ids() {
        let session_new = serde_json::json!({
            "models": { "availableModels": [
                { "modelId": "ok_id-1.2" },
                { "modelId": "" },
                { "modelId": "has space" },
                { "modelId": "agy:nested" },
                { "name": "no id" }
            ] }
        });
        assert_eq!(parse_agy_available_models(&session_new), ["agy:ok_id-1.2"]);
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn default_command_is_agy_acp_with_telemetry_off() {
        if std::env::var_os(AGY_COMMAND_ENV).is_some() {
            return;
        }
        let dir = tempdir().unwrap();
        let spec = AgyLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.command, "agy");
        assert!(!spec.fake);
        assert_eq!(strings(spec.launch_args()), ["acp", "--disable-telemetry"]);
    }

    #[test]
    fn settings_fill_command_and_args() {
        if std::env::var_os(AGY_COMMAND_ENV).is_some() {
            return;
        }
        let dir = tempdir().unwrap();
        let gav = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gav).unwrap();
        std::fs::write(
            gav.join("settings.json"),
            r#"{"providers":{"agy":{"command":"my-agy","args":["--sandbox"]}}}"#,
        )
        .unwrap();
        let spec = AgyLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.command, "my-agy");
        assert_eq!(spec.args, vec!["--sandbox".to_string()]);
        assert_eq!(
            strings(spec.launch_args()),
            ["acp", "--sandbox", "--disable-telemetry"]
        );
    }

    #[test]
    fn user_disable_telemetry_is_not_duplicated() {
        let spec = AgyLaunchSpec {
            command: "agy".into(),
            args: vec!["--disable-telemetry".into()],
            fake: false,
        };
        assert_eq!(strings(spec.launch_args()), ["acp", "--disable-telemetry"]);
    }

    #[test]
    fn fake_agent_gets_only_its_args() {
        let dir = tempdir().unwrap();
        let spec = AgyLaunchSpec::from_workspace_root(
            dir.path(),
            &[
                (AGY_COMMAND_OVERRIDE_KEY.into(), "/tmp/fake_acp_agent".into()),
                (AGY_ARGS_OVERRIDE_KEY.into(), "agy_inspect".into()),
            ],
        );
        assert!(spec.fake);
        assert_eq!(strings(spec.launch_args()), ["agy_inspect"]);
    }

    #[test]
    fn build_command_sets_hook_env() {
        let dir = tempdir().unwrap();
        let spec = AgyLaunchSpec {
            command: "agy".into(),
            args: vec![],
            fake: false,
        };
        let env_of = |cmd: &tokio::process::Command, key: &str| {
            cmd.as_std()
                .get_envs()
                .find(|(k, _)| *k == OsStr::new(key))
                .and_then(|(_, v)| v.map(|v| v.to_os_string()))
        };
        let plain = spec.build_command(dir.path(), &[]).unwrap();
        assert_eq!(
            env_of(&plain, crate::agent_guard::hook::HOOK_ROOT_ENV),
            Some(dir.path().as_os_str().to_os_string())
        );
        assert!(env_of(&plain, HOOK_READ_FOLDERS_ENV).is_none());

        let grants = vec![dir.path().join("a"), dir.path().join("b")];
        let granted = spec.build_command(dir.path(), &grants).unwrap();
        let joined = env_of(&granted, HOOK_READ_FOLDERS_ENV).expect("grants env");
        assert_eq!(std::env::split_paths(&joined).collect::<Vec<_>>(), grants);
    }

    #[test]
    fn agent_kind_labels_and_prefixes() {
        assert_eq!(AcpAgentKind::from_provider("agy"), AcpAgentKind::Agy);
        assert_eq!(AcpAgentKind::from_provider("dsh"), AcpAgentKind::Dsh);
        assert_eq!(AcpAgentKind::Agy.label(), "agy");
        assert_eq!(AcpAgentKind::Agy.model_prefix(), "agy:");
        assert_eq!(AcpAgentKind::Dsh.model_prefix(), "dsh:");
    }
}
