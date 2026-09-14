//! User-scope MCP registration via vendor CLIs (plan P1.4).
//!
//! Never hand-edits `~/.claude.json` or `~/.codex/config.toml`. The
//! entry is always the stdio `--resolve` shim form so it can address
//! whichever workspace the vendor process has as cwd.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::reach::{NestingPolicy, ReachPolicy};

pub const USER_SCOPE_SERVER_NAME: &str = "gaviero-memory";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserScopeVendor {
    Claude,
    Codex,
}

impl UserScopeVendor {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            other => bail!("user-scope vendor {other:?}: expected claude | codex"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserScopeOutcome {
    Added,
    AlreadyPresent,
    Removed,
    Missing,
}

/// Offer user-scope registration when the workspace record is not
/// `verified` (Allowed) for that vendor — or when the user asks.
pub fn should_offer_registration(root: &Path, vendor: UserScopeVendor) -> bool {
    !matches!(
        ReachPolicy::for_workspace(root).for_provider(vendor.as_str()),
        NestingPolicy::Allowed
    )
}

/// `claude mcp add -s user gaviero-memory -- <shim> --resolve`
pub fn claude_register_argv(shim_abs: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "add".into(),
        "-s".into(),
        "user".into(),
        USER_SCOPE_SERVER_NAME.into(),
        "--".into(),
        shim_abs.into(),
        "--resolve".into(),
    ]
}

/// `codex mcp add gaviero-memory -- <shim> --resolve`
pub fn codex_register_argv(shim_abs: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "add".into(),
        USER_SCOPE_SERVER_NAME.into(),
        "--".into(),
        shim_abs.into(),
        "--resolve".into(),
    ]
}

pub fn claude_unregister_argv() -> Vec<String> {
    vec![
        "mcp".into(),
        "remove".into(),
        "-s".into(),
        "user".into(),
        USER_SCOPE_SERVER_NAME.into(),
    ]
}

pub fn codex_unregister_argv() -> Vec<String> {
    vec![
        "mcp".into(),
        "remove".into(),
        USER_SCOPE_SERVER_NAME.into(),
    ]
}

pub fn claude_get_argv() -> Vec<String> {
    vec!["mcp".into(), "get".into(), USER_SCOPE_SERVER_NAME.into()]
}

pub fn codex_list_argv() -> Vec<String> {
    vec!["mcp".into(), "list".into()]
}

pub fn register_user_scope(vendor: UserScopeVendor, shim_abs: &Path) -> Result<UserScopeOutcome> {
    if !shim_abs.is_absolute() || !shim_abs.is_file() {
        bail!("user-scope registration requires an existing absolute shim path: {}", shim_abs.display());
    }
    let shim = shim_abs
        .to_str()
        .with_context(|| format!("shim path is not UTF-8: {}", shim_abs.display()))?;
    match vendor {
        UserScopeVendor::Claude => {
            if status_user_scope(UserScopeVendor::Claude)?.contains(USER_SCOPE_SERVER_NAME) {
                return Ok(UserScopeOutcome::AlreadyPresent);
            }
            run_vendor("claude", &claude_register_argv(shim))?;
            Ok(UserScopeOutcome::Added)
        }
        UserScopeVendor::Codex => {
            if status_user_scope(UserScopeVendor::Codex)?.contains(USER_SCOPE_SERVER_NAME) {
                return Ok(UserScopeOutcome::AlreadyPresent);
            }
            run_vendor("codex", &codex_register_argv(shim))?;
            Ok(UserScopeOutcome::Added)
        }
    }
}

pub fn unregister_user_scope(vendor: UserScopeVendor) -> Result<UserScopeOutcome> {
    match vendor {
        UserScopeVendor::Claude => {
            if !status_user_scope(UserScopeVendor::Claude)?.contains(USER_SCOPE_SERVER_NAME) {
                return Ok(UserScopeOutcome::Missing);
            }
            run_vendor("claude", &claude_unregister_argv())?;
            Ok(UserScopeOutcome::Removed)
        }
        UserScopeVendor::Codex => {
            if !status_user_scope(UserScopeVendor::Codex)?.contains(USER_SCOPE_SERVER_NAME) {
                return Ok(UserScopeOutcome::Missing);
            }
            run_vendor("codex", &codex_unregister_argv())?;
            Ok(UserScopeOutcome::Removed)
        }
    }
}

pub fn status_user_scope(vendor: UserScopeVendor) -> Result<String> {
    match vendor {
        UserScopeVendor::Claude => run_vendor_capture("claude", &claude_get_argv()),
        UserScopeVendor::Codex => run_vendor_capture("codex", &codex_list_argv()),
    }
}

fn run_vendor(bin: &str, args: &[String]) -> Result<()> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("spawning {bin}"))?;
    if !out.status.success() {
        bail!(
            "{bin} {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn run_vendor_capture(bin: &str, args: &[String]) -> Result<String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("spawning {bin}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        // A missing named server is expected for `mcp get`; error text
        // mentioning its name must never be mistaken for a registered entry.
        if args.get(1).is_some_and(|arg| arg == "get") {
            return Ok(String::new());
        }
        bail!("{bin} {} failed ({}): {stderr}", args.join(" "), out.status);
    }
    Ok(stdout)
}

pub fn default_shim_abs() -> PathBuf {
    PathBuf::from(super::resolver::resolve_shim_binary(
        "gaviero-mcp-shim",
        super::resolver::sibling_shim_path(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_register_argv_is_scope_user_stdio_resolve() {
        let argv = claude_register_argv(r"C:\bin\gaviero-mcp-shim.exe");
        assert_eq!(
            argv,
            vec![
                "mcp",
                "add",
                "-s",
                "user",
                "gaviero-memory",
                "--",
                r"C:\bin\gaviero-mcp-shim.exe",
                "--resolve",
            ]
        );
    }

    #[test]
    fn codex_register_argv_is_stdio_resolve() {
        let argv = codex_register_argv("/usr/bin/gaviero-mcp-shim");
        assert_eq!(
            argv,
            vec![
                "mcp",
                "add",
                "gaviero-memory",
                "--",
                "/usr/bin/gaviero-mcp-shim",
                "--resolve",
            ]
        );
    }

    #[test]
    fn unregister_argv_names_the_server() {
        assert!(claude_unregister_argv().contains(&USER_SCOPE_SERVER_NAME.to_string()));
        assert!(codex_unregister_argv().contains(&USER_SCOPE_SERVER_NAME.to_string()));
    }

    #[test]
    #[ignore = "live vendor CLIs; run: cargo test -p gaviero-core --lib -- --ignored live_register"]
    fn live_register_claude_user_scope() {}
}
