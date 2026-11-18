//! `dsh` ACP launcher: command, args, profile, env, version probe.
//!
//! `@deepseek-ai/dsh-acp` is a library, not a CLI. The published launcher is
//! `dsh --profile acp` from `@deepseek-ai/dsh`.

use std::path::Path;
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};

use crate::agent_session::tool_agent::config::ApiClientConfig;
use crate::context_planner::types::McpCapabilities;
use crate::mcp::resolver::resolve_shim_binary;
use crate::util::spawn::{agent_command, resolve_program};

pub const DEFAULT_DSH_COMMAND: &str = "dsh";
pub const DEFAULT_DSH_PROFILE: &str = "acp";
pub const DSH_COMMAND_SETTING: &str = "providers.dsh.command";
pub const DSH_ARGS_SETTING: &str = "providers.dsh.args";
pub const DSH_PROFILE_SETTING: &str = "providers.dsh.profile";
pub const DSH_CREDENTIAL_SOURCE_SETTING: &str = "providers.dsh.credentialSource";
pub const DSH_COMMAND_OVERRIDE_KEY: &str = "dsh_command";

/// dsh patch overlay that disables dsh's stored-credential store (the same
/// three rows dsh's own `headless` profile disables), so the
/// `DEEPSEEK_API_KEY` gaviero injects is the key dsh uses.
pub const GAVIERO_ACP_PATCH: &str = "\
# Written by gaviero; regenerated on every dsh: launch. Do not edit.
# Disables dsh's stored credentials so gaviero's DEEPSEEK_API_KEY wins
# (dsh >= 0.2.1-alpha.2 prefers ~/.dsh/.credentials.yaml over the env).
- id: credentials
  disabled: true
- id: authorization
  disabled: true
- id: deepseek-account
  disabled: true
";
pub const GAVIERO_ACP_PATCH_FILE: &str = "gaviero-acp.patch.yml";

/// dsh patch overlay that mounts gaviero's own skill folders into dsh's
/// `skill-filesystem` provider.
///
/// Two halves make a skill usable on a provider:
///
/// * **The body.** `$name` is gaviero's invocation syntax: the TUI resolves it
///   against [`crate::skills::SkillCatalog`] and the rendered `SKILL.md` rides
///   the ACP prompt as a `<skill name="…">` block
///   ([`super::build_prompt_blocks`]) — identical to Claude's enriched prompt.
/// * **The catalog.** dsh's `skill` tool and its `<available_skills>` reminder
///   are fed by dsh's own provider registry, which scans `<projectRoot>/.dsh/skills`,
///   `<projectRoot>/.agents/skills`, `~/.dsh/skills`, `~/.agents/skills` and the
///   bundled root — none of them gaviero's `.gaviero/skills`. Claude reads
///   `.claude/skills` natively, so a project that mirrors its skills there has
///   them in Claude's own catalog; dsh has no equivalent for `.gaviero/skills`.
///   Without this row the `$name` body still arrives, but the model's catalog
///   contradicts it and it can answer as if nothing was injected.
///
/// Applied as an **id-targeted config override** of the bundle's own
/// `skill-filesystem` row (not a second provider): dsh keeps its default
/// roots, watchers and `bundledSkillDir`, and the overlay imports no package,
/// so a dsh that does not ship the provider has nothing new to resolve.
/// `watch` stays at its default, so a skill added mid-session is catalogued on
/// the next lookup.
pub const DSH_SKILLS_PATCH_FILE: &str = "gaviero-skills.patch.yml";

/// Row id of the provider this overlay extends.
pub const DSH_SKILLS_ROW_ID: &str = "skill-filesystem";

/// Whose DeepSeek key `dsh:` uses (`providers.dsh.credentialSource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DshCredentialSource {
    /// gaviero's resolved key, enforced by [`GAVIERO_ACP_PATCH`].
    #[default]
    Gaviero,
    /// dsh's own stored credentials win when present; no patch.
    Dsh,
}

fn parse_credential_source(v: Option<&str>) -> DshCredentialSource {
    match v {
        Some("dsh") => DshCredentialSource::Dsh,
        None | Some("gaviero") => DshCredentialSource::Gaviero,
        Some(s) => {
            tracing::warn!(value = %s, "unknown providers.dsh.credentialSource; using \"gaviero\"");
            DshCredentialSource::Gaviero
        }
    }
}

#[derive(Debug, Clone)]
pub struct DshLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub profile: String,
    pub skip_api_key: bool,
    pub credential_source: DshCredentialSource,
}

impl DshLaunchSpec {
    pub fn from_workspace_root(root: &Path, extra: &[(String, String)]) -> Self {
        if let Some((_, cmd)) = extra.iter().find(|(k, _)| k == DSH_COMMAND_OVERRIDE_KEY)
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
                credential_source: DshCredentialSource::Gaviero,
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
                credential_source: DshCredentialSource::Gaviero,
            };
        }
        let settings = read_settings(root);
        let command = settings
            .get("providers.dsh.command")
            .and_then(|c| c.as_str())
            .unwrap_or(DEFAULT_DSH_COMMAND)
            .to_string();
        let args = settings
            .get("providers.dsh.args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let profile = settings
            .get("providers.dsh.profile")
            .and_then(|p| p.as_str())
            .unwrap_or(DEFAULT_DSH_PROFILE)
            .to_string();
        let credential_source = parse_credential_source(
            settings
                .get("providers.dsh.credentialSource")
                .and_then(|c| c.as_str()),
        );
        Self {
            skip_api_key: is_fake_agent(&command),
            command,
            args,
            profile,
            credential_source,
        }
    }

    pub fn command_resolvable(&self) -> bool {
        Path::new(&self.command).is_file() || resolve_program(&self.command).is_some()
    }

    /// Launcher arguments: the configured args, `--profile` unless the args
    /// already set one, then one `--patch <patch>` per overlay, in order.
    pub fn launch_args(&self, patches: &[&Path]) -> Vec<std::ffi::OsString> {
        let mut out: Vec<std::ffi::OsString> = self.args.iter().map(Into::into).collect();
        if !self.profile.is_empty() && !args_already_set_profile(&self.args) {
            out.push("--profile".into());
            out.push(self.profile.as_str().into());
        }
        for patch in patches {
            out.push("--patch".into());
            out.push(patch.as_os_str().to_owned());
        }
        out
    }

    /// `additional_roots` only decide whether a `GAVIERO.md` is in scope
    /// ([`crate::instructions`]); dsh's cwd stays `workspace_root`.
    pub fn build_command(
        &self,
        workspace_root: &Path,
        additional_roots: &[std::path::PathBuf],
    ) -> Result<tokio::process::Command> {
        let mut cmd = agent_command(&self.command);
        if !self.skip_api_key {
            // The inner error names every file that was searched (workspace,
            // then `~/.gaviero/secrets.toml`), so it is surfaced whole rather
            // than hidden behind a context line.
            let cfg = ApiClientConfig::resolve_deepseek(workspace_root, None, None)
                .map_err(|e| anyhow!("dsh: {e:#}"))?;
            cmd.env("DEEPSEEK_API_KEY", cfg.api_key.expose());
        }
        // Resolved after the key so a missing key is reported before
        // anything is written under `~/.gaviero`.
        let mut patches: Vec<std::path::PathBuf> = Vec::new();
        if !self.skip_api_key && self.credential_source == DshCredentialSource::Gaviero {
            patches.push(ensure_acp_patch_file(&acp_patch_dir().ok_or_else(|| {
                anyhow!(
                    "dsh: cannot locate ~/.gaviero for the credential patch; \
                     set providers.dsh.credentialSource to \"dsh\""
                )
            })?)?);
        }
        // GAVIERO.md in scope: switch off dsh's AGENTS.md / CLAUDE.md
        // discovery; the first prompt block carries GAVIERO.md instead.
        // Real dsh only — the fake agent takes no patch overlays.
        if !self.skip_api_key && crate::instructions::is_active(workspace_root, additional_roots) {
            match acp_patch_dir() {
                Some(dir) => patches.push(ensure_patch_file(
                    &dir,
                    crate::instructions::DSH_DISABLE_INSTRUCTIONS_PATCH_FILE,
                    crate::instructions::DSH_DISABLE_INSTRUCTIONS_PATCH,
                )?),
                None => tracing::warn!(
                    "dsh: cannot locate ~/.gaviero; AGENTS.md discovery stays on"
                ),
            }
        }
        // Gaviero's skills: dsh's own provider registry also scans gaviero's
        // skill folders, so the `skill` tool and `<available_skills>` catalog
        // match what `$name` resolves in chat. Skipped when no skill folder
        // exists — a `dsh:` launch pays nothing for a feature with no skills.
        // Real dsh only — the fake agent takes no patch overlays.
        if !self.skip_api_key {
            let skill_dirs = gaviero_skill_dirs(workspace_root, additional_roots);
            if !skill_dirs.is_empty() {
                match acp_patch_dir() {
                    Some(dir) => patches.push(ensure_patch_file(
                        &dir,
                        DSH_SKILLS_PATCH_FILE,
                        &skills_patch_body(&skill_dirs),
                    )?),
                    None => tracing::warn!(
                        "dsh: cannot locate ~/.gaviero; dsh's skill catalog stays gaviero-unaware"
                    ),
                }
            }
        }
        let patch_refs: Vec<&Path> = patches.iter().map(|p| p.as_path()).collect();
        cmd.args(self.launch_args(&patch_refs));
        cmd.current_dir(workspace_root);
        cmd.env("NO_COLOR", "1");
        Ok(cmd)
    }
}

/// `~/.gaviero/dsh/` — per-machine, so it ignores `GAVIERO_ROOT`.
fn acp_patch_dir() -> Option<std::path::PathBuf> {
    crate::workspace::home_gaviero_dir().map(|d| d.join("dsh"))
}

/// The gaviero skill folders in scope for a launch, most specific first.
///
/// Mirrors what `$name` resolves against in a chat turn
/// ([`crate::skills::SkillCatalog::scan`]): every workspace folder's
/// `.gaviero/skills`, the workspace layer's (beside the `.gaviero-workspace`
/// file), and the user-wide root. That user-wide root is
/// **`$GAVIERO_ROOT/.gaviero/skills`** — the folder gaviero's global skills
/// have always lived in; `~/.gaviero/skills` is only that root when
/// `GAVIERO_ROOT` is unset (see [`crate::workspace::user_gaviero_dir`]).
pub fn gaviero_skill_dirs(
    workspace_root: &Path,
    additional_roots: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    gaviero_skill_dirs_with_global(
        workspace_root,
        additional_roots,
        &crate::skills::SkillCatalog::global_skills_dir(),
    )
}

/// [`gaviero_skill_dirs`] with an explicit global folder, so tests do not
/// depend on `$GAVIERO_ROOT`.
///
/// The primary folder, the workspace layer and the global root come from
/// [`crate::skills::SkillCatalog::scan`] itself, so the list cannot drift from
/// the cascade. `skills.extraRoots` entries ride along for the same reason.
/// Sibling folders are added explicitly: a single-folder scan only sees the
/// primary, while `$name` resolves against every member of a multi-folder
/// workspace.
pub fn gaviero_skill_dirs_with_global(
    workspace_root: &Path,
    additional_roots: &[std::path::PathBuf],
    global_dir: &Path,
) -> Vec<std::path::PathBuf> {
    let workspace = crate::workspace::Workspace::single_folder(workspace_root.to_path_buf());
    let (catalog, _warnings) = crate::skills::SkillCatalog::scan(&workspace, global_dir);
    let mut dirs: Vec<std::path::PathBuf> = catalog
        .scanned_dirs()
        .iter()
        .filter(|dir| dir.exists)
        .map(|dir| dir.path.clone())
        .collect();
    // `native_roots` puts the global root last; keep every sibling folder
    // before it so dsh's catalog keeps the same reach, nearest scope first.
    let mut insert_at = dirs
        .iter()
        .position(|dir| dir.as_path() == global_dir)
        .unwrap_or(dirs.len());
    for root in additional_roots {
        let dir = root.join(".gaviero").join("skills");
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.insert(insert_at, dir);
            insert_at += 1;
        }
    }
    dirs
}

/// YAML body of [`DSH_SKILLS_PATCH_FILE`] for `dirs`.
///
/// Paths are written with forward slashes and YAML single quotes (literal, so
/// backslashes need no doubling); a `'` in a path is doubled, the one escape
/// YAML single-quoted scalars have.
pub fn skills_patch_body(dirs: &[std::path::PathBuf]) -> String {
    let mut body = String::from(
        "# Written by gaviero; regenerated on every dsh: launch. Do not edit.\n\
         # Extend dsh's own skill provider with gaviero's skill folders, so its\n\
         # `skill` tool and <available_skills> catalog list the skills that\n\
         # `$name` resolves in gaviero's chat. dsh's default roots, watchers and\n\
         # bundled skills are untouched: this only adds scan roots.\n",
    );
    body.push_str(&format!("- id: {DSH_SKILLS_ROW_ID}\n"));
    body.push_str("  config:\n");
    body.push_str("    customSkillDirs:\n");
    for dir in dirs {
        let slashy = dir
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''");
        body.push_str(&format!("      - '{slashy}'\n"));
    }
    body
}

/// Write [`GAVIERO_ACP_PATCH`] to `<dir>/gaviero-acp.patch.yml` (tmp + rename)
/// unless it is already current, and return its path.
pub fn ensure_acp_patch_file(dir: &Path) -> Result<std::path::PathBuf> {
    ensure_patch_file(dir, GAVIERO_ACP_PATCH_FILE, GAVIERO_ACP_PATCH)
}

/// Write `body` to `<dir>/<name>` (tmp + rename) unless it is already
/// current, and return its path.
pub fn ensure_patch_file(dir: &Path, name: &str, body: &str) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("dsh: creating {}", dir.display()))?;
    let path = dir.join(name);
    if std::fs::read_to_string(&path).is_ok_and(|s| s == body) {
        return Ok(path);
    }
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, body)
        .with_context(|| format!("dsh: writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("dsh: replacing {}", path.display()))?;
    Ok(path)
}

fn args_already_set_profile(args: &[String]) -> bool {
    args.windows(2).any(|w| w[0] == "--profile")
        || args
            .iter()
            .any(|a| a == "--profile" || a.starts_with("--profile="))
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

/// The settings governing `root`, layered user → workspace → repo.
fn read_settings(root: &Path) -> crate::workspace::LayeredSettings {
    crate::workspace::LayeredSettings::for_folder(root)
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
    let output = crate::util::spawn::agent_command_std(command)
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
///
/// `conv_id` is the chat conversation this session belongs to. When present it
/// rides as [`crate::mcp::ask::CONVERSATION_HEADER`] on every request dsh makes
/// to gaviero's server, which is what lets `ask_user_question` exist for a
/// `dsh:` session and nowhere else: ACP carries no elicitation, so the question
/// has to travel over MCP, and the server needs to know which conversation's
/// overlay to park it on.
///
/// Only the `gaviero` entry is tagged. context7 and `mcp.extraServers` are
/// other people's servers; the header means nothing to them and would leak a
/// local conversation id into a third party.
pub fn http_mcp_server(root: &Path, conv_id: Option<&str>) -> Option<serde_json::Value> {
    let desc = crate::mcp::read_descriptor(&crate::mcp::McpEndpointDescriptor::path(root)).ok()?;
    let url = desc.http_url?;
    let token_path = desc.http_token_path?;
    let token = std::fs::read_to_string(token_path).ok()?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let mut headers = vec![serde_json::json!({
        "name": "Authorization",
        "value": format!("Bearer {token}")
    })];
    if let Some(conv_id) = conv_id {
        headers.push(serde_json::json!({
            "name": crate::mcp::ask::CONVERSATION_HEADER,
            "value": conv_id,
        }));
    }
    Some(serde_json::json!({
        "type": "http",
        "name": "gaviero",
        "url": url,
        "headers": headers,
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
/// `conv_id` tags the `gaviero` entry only ([`http_mcp_server`]). It is what
/// makes `ask_user_question` available to a `dsh:` session and to nothing else:
/// the ACP wire has no elicitation, so a structured question has to travel over
/// MCP, and the header tells gaviero's server which conversation's overlay to
/// park it on.
///
/// Separately: dsh's `session/new` carries **no tool list and no permission
/// policy** (`agent_client_protocol/mod.rs`), so `agent.availableTools` is
/// *structurally unenforced* for dsh. That is recorded as
/// `ToolEnforcement::Unenforced` in the capability table, not attempted here.
pub fn mcp_servers_for_session(
    root: &Path,
    caps: McpCapabilities,
    conv_id: Option<&str>,
) -> Vec<serde_json::Value> {
    // An in-process provider links servers as a live value; it has no MCP
    // transport to hang a `mcpServers` entry on. The two capability axes say
    // *what* a provider may use, not *how* it receives it — so an in-process row
    // registers nothing even though it now allows context7 (Phase 2d reaches it
    // as a native tool instead).
    if !caps.uses_mcp_servers() {
        return Vec::new();
    }

    let mut servers = Vec::new();
    if let Some(http) = http_mcp_server(root, conv_id) {
        servers.push(http);
    }

    let workspace = crate::workspace::Workspace::single_folder(root.to_path_buf());
    let permissions = crate::mcp::resolve_mcp_permissions(&workspace, Some(root));

    // The capability table gates injection as well as the settings cascade:
    // `context7_allowed` / `extra_servers_allowed` are the per-provider "can",
    // `mcp.context7.enabled` / `mcp.extraServers` the per-workspace "wants".
    // Both must hold, and the table is the single source for the former
    // (`Provider::mcp_capabilities`).
    let ctx7 = crate::mcp::resolve_context7_config(&workspace, Some(root));
    if caps.context7
        && ctx7.enabled
        && let Some(url) = ctx7.remote_url()
        && permissions.server_allowed("context7")
    {
        servers.push(http_entry("context7", url, Vec::new()));
    }

    if caps.extra_servers {
        for extra in crate::mcp::extra_servers_from_workspace(&workspace, Some(root)) {
            if !permissions.server_allowed(&extra.name) {
                continue;
            }
            if let crate::mcp::ExtraMcpTransport::Url { url } = extra.transport {
                servers.push(http_entry(&extra.name, &url, Vec::new()));
            }
        }
    }

    servers
}

pub fn missing_key_error() -> anyhow::Error {
    anyhow!(
        "dsh: no DeepSeek API key. Set DEEPSEEK_API_KEY or add `[deepseek] api_key` \
         to <workspace>/.gaviero/secrets.toml or ~/.gaviero/secrets.toml"
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
        assert!(args_already_set_profile(&[
            "--profile".into(),
            "acp".into()
        ]));
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
    fn credential_source_defaults_to_gaviero() {
        let dir = tempdir().unwrap();
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.credential_source, DshCredentialSource::Gaviero);
    }

    #[test]
    fn credential_source_reads_dsh_setting() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"providers":{"dsh":{"credentialSource":"dsh"}}}"#,
        );
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        assert_eq!(spec.credential_source, DshCredentialSource::Dsh);
    }

    fn acp_spec() -> DshLaunchSpec {
        DshLaunchSpec {
            command: "dsh".into(),
            args: vec![],
            profile: "acp".into(),
            skip_api_key: false,
            credential_source: DshCredentialSource::Gaviero,
        }
    }

    #[test]
    fn launch_args_add_patch_after_profile() {
        let args = acp_spec().launch_args(&[Path::new("p.yml")]);
        assert_eq!(args, ["--profile", "acp", "--patch", "p.yml"]);
    }

    #[test]
    fn launch_args_repeat_patch_per_overlay() {
        let args = acp_spec().launch_args(&[Path::new("a.yml"), Path::new("b.yml")]);
        assert_eq!(
            args,
            ["--profile", "acp", "--patch", "a.yml", "--patch", "b.yml"]
        );
    }

    #[test]
    fn launch_args_without_patch_unchanged() {
        assert_eq!(acp_spec().launch_args(&[]), ["--profile", "acp"]);
    }

    #[test]
    fn acp_patch_file_is_written_once_and_disables_three_rows() {
        let dir = tempdir().unwrap();
        let first = ensure_acp_patch_file(dir.path()).unwrap();
        let second = ensure_acp_patch_file(dir.path()).unwrap();
        assert_eq!(first, second);
        let text = std::fs::read_to_string(&first).unwrap();
        assert_eq!(text, GAVIERO_ACP_PATCH);
        for row in ["- id: credentials", "- id: authorization", "- id: deepseek-account"] {
            assert!(text.contains(row), "{row}");
        }
        assert!(!dir.path().join(format!("{GAVIERO_ACP_PATCH_FILE}.tmp")).exists());
    }

    /// Write `<dir>/<name>/SKILL.md` with a minimal valid frontmatter.
    fn write_skill(dir: &Path, name: &str) {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test skill\n---\n\nBody.\n"),
        )
        .unwrap();
    }

    #[test]
    fn skill_dirs_are_the_scanned_roots_that_exist() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        // The user-wide root is `$GAVIERO_ROOT/.gaviero/skills`, not a copy
        // under the home directory.
        let global = dir
            .path()
            .join("gaviero-root")
            .join(".gaviero")
            .join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(&root.join(".gaviero").join("skills"), "lint");
        write_skill(&global, "migrate-component");

        let dirs = gaviero_skill_dirs_with_global(&root, &[], &global);
        let root_skills = root.join(".gaviero").join("skills");
        assert!(dirs.contains(&root_skills), "{dirs:?}");
        assert!(dirs.contains(&global), "{dirs:?}");
        // Most specific first, so dsh's rank tie-break prefers the folder and
        // the global root stays the fallback.
        let root_at = dirs.iter().position(|d| d == &root_skills).unwrap();
        let global_at = dirs.iter().position(|d| d == &global).unwrap();
        assert!(root_at < global_at, "{dirs:?}");
        assert_eq!(dirs.last(), Some(&global), "{dirs:?}");
        // A folder that does not exist is not mounted.
        assert!(dirs.iter().all(|d| d.exists()), "{dirs:?}");
    }

    /// A stale `~/.gaviero/skills` left behind by an older setup must never be
    /// mounted: the launch names `$GAVIERO_ROOT/.gaviero/skills` and nothing
    /// else for the user-wide scope.
    #[test]
    fn skill_dirs_never_add_a_stale_home_copy() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        let global = dir
            .path()
            .join("gaviero-root")
            .join(".gaviero")
            .join("skills");
        let home = dir.path().join("home").join(".gaviero").join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(&global, "deep-research");
        write_skill(&home, "deep-research");

        let dirs = gaviero_skill_dirs_with_global(&root, &[], &global);
        assert!(dirs.contains(&global), "{dirs:?}");
        assert!(!dirs.contains(&home), "{dirs:?}");
    }

    #[test]
    fn skill_dirs_include_sibling_workspace_folders() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        let sibling = dir.path().join("flutter");
        let global = dir
            .path()
            .join("gaviero-root")
            .join(".gaviero")
            .join("skills");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        write_skill(&sibling.join(".gaviero").join("skills"), "flutter-widget");

        let dirs = gaviero_skill_dirs_with_global(&root, &[sibling.clone()], &global);
        let sibling_skills = sibling.join(".gaviero").join("skills");
        assert!(dirs.contains(&sibling_skills), "{dirs:?}");

        // A sibling without a skills folder is not mounted, and the folder is
        // not listed twice when it is passed twice.
        let bare = dir.path().join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let dirs =
            gaviero_skill_dirs_with_global(&root, &[sibling.clone(), sibling, bare], &global);
        assert_eq!(
            dirs.iter().filter(|d| **d == sibling_skills).count(),
            1,
            "{dirs:?}"
        );
    }

    /// The global root the launcher mounts is `$GAVIERO_ROOT/.gaviero/skills`
    /// when the variable is set; the home directory is only the fallback.
    #[test]
    fn global_skill_root_follows_gaviero_root() {
        let dir = crate::skills::SkillCatalog::global_skills_dir();
        eprintln!("global skills root: {}", dir.display());
        let Some(root) = std::env::var(crate::workspace::GAVIERO_ROOT_ENV).ok() else {
            return;
        };
        if root.trim().is_empty() {
            return;
        }
        let root = std::path::absolute(std::path::PathBuf::from(root)).unwrap();
        assert_eq!(dir, root.join(".gaviero").join("skills"));
        if let Some(home) = dirs::home_dir() {
            let home_skills = home.join(".gaviero").join("skills");
            if home_skills != dir {
                assert_ne!(dir, home_skills);
            }
        }
    }

    #[test]
    fn skill_dirs_are_empty_when_nothing_is_installed() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let global = dir.path().join("absent").join("skills");
        assert!(gaviero_skill_dirs_with_global(&root, &[], &global).is_empty());
    }

    #[test]
    fn skills_patch_extends_the_bundle_provider_with_each_dir_quoted() {
        let dirs = vec![
            std::path::PathBuf::from("C:/repo/.gaviero/skills"),
            // Backslashes become forward slashes; a quote is YAML-doubled.
            std::path::PathBuf::from(r"C:\it's\.gaviero\skills"),
        ];
        let body = skills_patch_body(&dirs);
        assert!(body.contains("- id: skill-filesystem\n"), "{body}");
        assert!(body.contains("  config:\n"), "{body}");
        assert!(body.contains("    customSkillDirs:\n"), "{body}");
        assert!(
            body.contains("      - 'C:/repo/.gaviero/skills'\n"),
            "{body}"
        );
        assert!(
            body.contains("      - 'C:/it''s/.gaviero/skills'\n"),
            "{body}"
        );
        // An override of the existing row, not a second provider: no insert,
        // no second registration under another provider name.
        assert!(!body.contains("insert:"), "{body}");
        assert!(!body.contains("providerName:"), "{body}");
        assert_eq!(body.matches("- id: ").count(), 1, "{body}");
    }

    #[test]
    fn skills_patch_file_is_written_once_and_reused() {
        let dir = tempdir().unwrap();
        let body = skills_patch_body(&[std::path::PathBuf::from("C:/repo/.gaviero/skills")]);
        let first = ensure_patch_file(dir.path(), DSH_SKILLS_PATCH_FILE, &body).unwrap();
        let second = ensure_patch_file(dir.path(), DSH_SKILLS_PATCH_FILE, &body).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), body);
    }

    #[test]
    fn missing_api_key_is_a_clear_error() {
        if std::env::var_os("DEEPSEEK_API_KEY").is_some()
            || std::env::var_os("GAVIERO_DSH_COMMAND").is_some()
        {
            return;
        }
        let dir = tempdir().unwrap();
        // A machine with `~/.gaviero/secrets.toml` resolves a key anyway.
        if ApiClientConfig::resolve_deepseek(dir.path(), None, None).is_ok() {
            return;
        }
        let spec = DshLaunchSpec::from_workspace_root(dir.path(), &[]);
        let err = spec.build_command(dir.path(), &[]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("DEEPSEEK_API_KEY") || msg.contains("secrets.toml"),
            "{msg}"
        );
    }

    #[test]
    fn mcp_servers_http_only_without_descriptor_is_empty() {
        let dir = tempdir().unwrap();
        assert!(mcp_servers_for_session(dir.path(), dsh_caps(), None).is_empty());
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

    /// A live-looking HTTP endpoint descriptor plus its token file, so
    /// [`http_mcp_server`] returns an entry.
    fn write_http_descriptor(root: &Path) {
        let gav = root.join(".gaviero");
        std::fs::create_dir_all(&gav).unwrap();
        let token_path = gav.join("mcp-http-token");
        std::fs::write(&token_path, "test-token\n").unwrap();
        let desc = serde_json::json!({
            "v": 1,
            "workspace_id": "test",
            "http_url": "http://127.0.0.1:9/mcp",
            "http_token_path": token_path,
            "pid": std::process::id(),
            "started_at": chrono::Utc::now(),
        });
        std::fs::write(
            crate::mcp::McpEndpointDescriptor::path(root),
            desc.to_string(),
        )
        .unwrap();
    }

    /// A `dsh:` session's gaviero entry names the conversation; nothing else
    /// does. That header is the only thing that makes `ask_user_question`
    /// exist for this session, so the two must travel together.
    #[test]
    fn the_gaviero_http_entry_carries_the_conversation_header() {
        let dir = tempdir().unwrap();
        write_http_descriptor(dir.path());
        write_settings(dir.path(), r#"{"mcp":{"context7":{"enabled":true}}}"#);

        let tagged = mcp_servers_for_session(dir.path(), dsh_caps(), Some("conv-1"));
        let gaviero = tagged
            .iter()
            .find(|s| s["name"] == "gaviero")
            .expect("live endpoint must register gaviero");
        let headers = gaviero["headers"].as_array().expect("headers array");
        assert!(
            headers
                .iter()
                .any(|h| h["name"] == "Authorization" && h["value"]
                    .as_str()
                    .is_some_and(|v| v.starts_with("Bearer "))),
            "the bearer header must stay: {gaviero}"
        );
        let conversation = headers
            .iter()
            .find(|h| h["name"] == crate::mcp::ask::CONVERSATION_HEADER)
            .unwrap_or_else(|| panic!("no conversation header in {gaviero}"));
        assert_eq!(conversation["value"], "conv-1");

        // context7 is another party's server: the conversation id stays home.
        let ctx7 = tagged
            .iter()
            .find(|s| s["name"] == "context7")
            .expect("context7 enabled in settings");
        assert!(
            !ctx7["headers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["name"] == crate::mcp::ask::CONVERSATION_HEADER),
            "a local conversation id must not leak to context7: {ctx7}"
        );

        // A session with no conversation sends no such header.
        let untagged = mcp_servers_for_session(dir.path(), dsh_caps(), None);
        let gaviero = untagged
            .iter()
            .find(|s| s["name"] == "gaviero")
            .expect("live endpoint must register gaviero");
        assert!(
            !gaviero["headers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["name"] == crate::mcp::ask::CONVERSATION_HEADER),
            "no conversation, no header: {gaviero}"
        );
    }

    fn names(servers: &[serde_json::Value]) -> Vec<String> {
        servers
            .iter()
            .filter_map(|s| s.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    }

    /// dsh's declared MCP gates, **read from the capability table** so these
    /// tests fail when the row changes instead of silently pinning a copy.
    fn dsh_caps() -> crate::context_planner::types::McpCapabilities {
        crate::context_planner::types::Provider::Dsh.mcp_capabilities()
    }

    /// §D wiring: injection is gated by the capability table *as well as* the
    /// settings cascade. The same workspace opt-in must register nothing under
    /// a row that denies both axes — that is what makes the table a decision
    /// rather than a comment.
    #[test]
    fn capability_table_gates_context7_and_extras() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{
                "context7":{"enabled":true},
                "extraServers":[{"name":"consensus","url":"https://example.test/mcp"}]
            }}"#,
        );

        // dsh's row allows both, so the settings opt-in is sufficient.
        let allowed = crate::context_planner::types::Provider::Dsh.mcp_capabilities();
        assert!(allowed.context7 && allowed.extra_servers);
        assert_eq!(
            names(&mcp_servers_for_session(dir.path(), allowed, None)),
            vec!["context7", "consensus"]
        );

        // The in-process row allows context7 as a *native* tool (Phase 2d) and
        // foreign servers through its own MCP client — but it has no MCP
        // transport to *receive* a server on, so the same settings still
        // register no MCP server entry. Gated on transport, not on the two
        // axes: both are allowed here, and that is exactly the case the
        // transport check exists for.
        let in_process = crate::context_planner::types::Provider::Deepseek.mcp_capabilities();
        assert!(in_process.extra_servers);
        assert!(!in_process.uses_mcp_servers());
        assert!(mcp_servers_for_session(dir.path(), in_process, None).is_empty());
    }

    /// Decision E: with no gaviero endpoint but context7 on, dsh still gets
    /// context7 — and `registers_gaviero` must say *false*, so the
    /// `exposedTools` gate is not fooled by a non-empty list.
    #[test]
    fn context7_http_is_registered_without_a_gaviero_endpoint() {
        let dir = tempdir().unwrap();
        write_settings(dir.path(), r#"{"mcp":{"context7":{"enabled":true}}}"#);
        let servers = mcp_servers_for_session(dir.path(), dsh_caps(), None);
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
        assert!(mcp_servers_for_session(dir.path(), dsh_caps(), None).is_empty());
    }

    /// Disabled by default: no workspace opt-in, no entry.
    #[test]
    fn context7_is_opt_in() {
        let dir = tempdir().unwrap();
        write_settings(dir.path(), r#"{"mcp":{"context7":{"enabled":false}}}"#);
        assert!(mcp_servers_for_session(dir.path(), dsh_caps(), None).is_empty());
    }

    /// Decision E: URL extras register, stdio extras are skipped.
    #[test]
    fn extra_servers_register_url_and_skip_stdio() {
        let dir = tempdir().unwrap();
        write_settings(
            dir.path(),
            r#"{"mcp":{"extraServers":[
                {"name":"consensus","url":"https://example.test/mcp"},
                {"name":"local-thing","command":"some-server","args":["--x"]}
            ]}}"#,
        );
        let servers = mcp_servers_for_session(dir.path(), dsh_caps(), None);
        assert_eq!(names(&servers), vec!["consensus"]);
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
                "extraServers":[{"name":"consensus","url":"https://example.test/mcp"}],
                "permissions":{"deny":["context7:*","consensus:*"]}
            }}"#,
        );
        assert!(mcp_servers_for_session(dir.path(), dsh_caps(), None).is_empty());
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
                "extraServers":[{"name":"consensus","url":"https://example.test/mcp"}],
                "permissions":{"allow":["gaviero:*"]}
            }}"#,
        );
        assert!(mcp_servers_for_session(dir.path(), dsh_caps(), None).is_empty());
    }
}
