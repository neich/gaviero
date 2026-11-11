//! What a session-host tab runs: gaviero on a local folder or workspace
//! file, or gaviero on a remote machine reached with the system `ssh`.
//!
//! Saved (state file, restart record) as a string: a local target is its
//! path, exactly as before SSH tabs existed, and an SSH target is
//! `ssh://<host>` or `ssh://<host><path>` (`ssh://<host>/~/…` for a path
//! under the remote home).
//!
//! An SSH tab runs [`ssh_program`] with [`ssh_args`]: the remote command
//! starts the user's login shell (so `~/.local/bin` and the like are on
//! PATH) and execs gaviero under it, marked as a session-host child. The
//! command is built from validated text only — no quote, backslash, `$` or
//! backtick ever reaches it from a host, path or override — so its single-
//! and double-quoting holds in sh, bash, zsh and fish alike.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SSH_PREFIX: &str = "ssh://";

/// Characters never accepted in a host, a remote path or a remote gaviero
/// path: they would break out of the remote command's quoting.
const REFUSED: [char; 8] = ['\'', '"', '\\', '$', '`', '\n', '\r', '\0'];

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub(crate) enum Target {
    /// A local folder or `.gaviero-workspace` file.
    Local(PathBuf),
    /// gaviero on `host` (an `ssh` destination: an alias from
    /// `~/.ssh/config` or `[user@]hostname`). `path`: the remote folder or
    /// workspace file (absolute, or `~`-relative); `None` lets the user pick
    /// it on the remote (`gaviero --pick`).
    Ssh { host: String, path: Option<String> },
}

impl From<Target> for String {
    fn from(target: Target) -> String {
        match target {
            Target::Local(path) => path.to_string_lossy().into_owned(),
            Target::Ssh { host, path: None } => format!("{SSH_PREFIX}{host}"),
            Target::Ssh {
                host,
                path: Some(path),
            } => {
                if path.starts_with('/') {
                    format!("{SSH_PREFIX}{host}{path}")
                } else {
                    format!("{SSH_PREFIX}{host}/{path}")
                }
            }
        }
    }
}

impl TryFrom<String> for Target {
    type Error = String;

    fn try_from(text: String) -> Result<Target, String> {
        let Some(rest) = text.strip_prefix(SSH_PREFIX) else {
            return Ok(Target::Local(PathBuf::from(text)));
        };
        let (host, remainder) = match rest.split_once('/') {
            Some((host, remainder)) => (host, remainder),
            None => (rest, ""),
        };
        validate_host(host)?;
        let path = if remainder.is_empty() {
            None
        } else if remainder.starts_with('~') {
            Some(remainder.to_string())
        } else {
            Some(format!("/{remainder}"))
        };
        Ok(Target::Ssh {
            host: host.to_string(),
            path,
        })
    }
}

impl Target {
    pub fn is_ssh(&self) -> bool {
        matches!(self, Target::Ssh { .. })
    }

    pub fn host(&self) -> Option<&str> {
        match self {
            Target::Ssh { host, .. } => Some(host),
            Target::Local(_) => None,
        }
    }

    /// How the picker and messages show it: the local path, or `host` /
    /// `host:path`.
    pub fn display(&self) -> String {
        match self {
            Target::Local(path) => path.display().to_string(),
            Target::Ssh { host, path: None } => host.clone(),
            Target::Ssh {
                host,
                path: Some(path),
            } => format!("{host}:{path}"),
        }
    }

    /// What the user typed in the picker's SSH mode: `host` or `host:path`.
    pub fn parse_ssh_input(text: &str) -> Result<Target, String> {
        let text = text.trim();
        let (host, path) = match text.split_once(':') {
            Some((host, path)) => (host, Some(path)),
            None => (text, None),
        };
        validate_host(host)?;
        let path = match path {
            None | Some("") => None,
            Some(path) => {
                if !(path.starts_with('/') || path.starts_with('~')) {
                    return Err("remote path must start with / or ~".to_string());
                }
                validate_remote_text("remote path", path)?;
                Some(path.to_string())
            }
        };
        Ok(Target::Ssh {
            host: host.to_string(),
            path,
        })
    }

    /// The command `ssh` runs on the remote machine (one argument):
    /// `exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 <gaviero> <arg>'`,
    /// with `<arg>` `--pick` when there is no path yet.
    pub fn remote_command(&self, gaviero_override: Option<&str>) -> Result<String, String> {
        let Target::Ssh { host, path } = self else {
            return Err("not an SSH target".to_string());
        };
        validate_host(host)?;
        let gaviero = match gaviero_override {
            Some(program) => {
                validate_remote_text("remote gaviero path", program)?;
                remote_word(program)
            }
            None => "gaviero".to_string(),
        };
        let arg = match path {
            Some(path) => {
                validate_remote_text("remote path", path)?;
                remote_word(path)
            }
            None => "--pick".to_string(),
        };
        Ok(format!(
            "exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 {gaviero} {arg}'"
        ))
    }
}

/// A remote path as one double-quoted word for the login shell; `~` and a
/// leading `~/` become `$HOME`, which double quotes still expand.
pub fn remote_word(text: &str) -> String {
    if text == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = text.strip_prefix("~/") {
        format!("\"$HOME/{rest}\"")
    } else {
        format!("\"{text}\"")
    }
}

/// An `ssh` destination the remote command can carry: not empty, not an
/// option (`-…`), no whitespace, `/`, `:` or refused character.
pub fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("type a host".to_string());
    }
    if host.starts_with('-') {
        return Err("a host cannot start with -".to_string());
    }
    if let Some(c) = host
        .chars()
        .find(|c| c.is_whitespace() || *c == '/' || *c == ':' || REFUSED.contains(c))
    {
        return Err(format!("unsupported character {} in host", shown(c)));
    }
    Ok(())
}

/// `what` (a remote path or program) holds no refused character.
pub fn validate_remote_text(what: &str, text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err(format!("empty {what}"));
    }
    match text.chars().find(|c| REFUSED.contains(c)) {
        Some(c) => Err(format!("unsupported character {} in {what}", shown(c))),
        None => Ok(()),
    }
}

fn shown(c: char) -> String {
    match c {
        '\n' => "newline".to_string(),
        '\r' => "carriage return".to_string(),
        '\0' => "NUL".to_string(),
        c if c.is_whitespace() => "space".to_string(),
        c => c.to_string(),
    }
}

/// The arguments for `ssh`: a terminal, keepalives so a dead link is
/// noticed, the destination and the remote command.
pub fn ssh_args(host: &str, remote_command: &str) -> Vec<String> {
    [
        "-t",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        host,
        remote_command,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The `ssh` client: `GAVIERO_SSH` when set (tests), else Windows' own
/// OpenSSH — Git's `ssh`, often first on PATH, handles the console
/// differently — or `ssh` from PATH.
pub fn ssh_program() -> std::ffi::OsString {
    if let Some(program) = std::env::var_os("GAVIERO_SSH") {
        return program;
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let inbox = PathBuf::from(root)
            .join("System32")
            .join("OpenSSH")
            .join("ssh.exe");
        if inbox.is_file() {
            return inbox.into_os_string();
        }
    }
    "ssh".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(host: &str, path: Option<&str>) -> Target {
        Target::Ssh {
            host: host.to_string(),
            path: path.map(str::to_string),
        }
    }

    fn round_trip(target: Target) -> String {
        let text: String = target.clone().into();
        assert_eq!(Target::try_from(text.clone()).unwrap(), target, "{text}");
        text
    }

    #[test]
    fn targets_round_trip_through_their_string_form() {
        assert_eq!(round_trip(Target::Local(PathBuf::from("C:\\x"))), "C:\\x");
        assert_eq!(round_trip(ssh("vps", None)), "ssh://vps");
        assert_eq!(
            round_trip(ssh("u@vps", Some("/srv/a b"))),
            "ssh://u@vps/srv/a b"
        );
        assert_eq!(round_trip(ssh("vps", Some("~/proj"))), "ssh://vps/~/proj");
        assert_eq!(round_trip(ssh("vps", Some("~"))), "ssh://vps/~");
    }

    #[test]
    fn a_legacy_plain_path_is_a_local_target() {
        let t: Target = serde_json::from_str("\"/home/u/proj\"").unwrap();
        assert_eq!(t, Target::Local(PathBuf::from("/home/u/proj")));
        let json = serde_json::to_string(&ssh("vps", Some("/srv/a"))).unwrap();
        assert_eq!(json, "\"ssh://vps/srv/a\"");
    }

    #[test]
    fn a_saved_target_with_a_bad_host_is_refused() {
        assert!(Target::try_from("ssh://-oProxyCommand=x".to_string()).is_err());
        assert!(Target::try_from("ssh:///srv".to_string()).is_err());
    }

    #[test]
    fn remote_command_picks_when_there_is_no_path() {
        assert_eq!(
            ssh("vps", None).remote_command(None).unwrap(),
            "exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 gaviero --pick'"
        );
    }

    #[test]
    fn remote_command_quotes_the_path() {
        assert_eq!(
            ssh("vps", Some("/srv/my proj")).remote_command(None).unwrap(),
            "exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 gaviero \"/srv/my proj\"'"
        );
    }

    #[test]
    fn remote_command_expands_home_in_path_and_override() {
        assert_eq!(
            ssh("vps", Some("~/proj"))
                .remote_command(Some("~/.cargo/bin/gaviero"))
                .unwrap(),
            "exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 \"$HOME/.cargo/bin/gaviero\" \"$HOME/proj\"'"
        );
        assert_eq!(remote_word("~"), "\"$HOME\"");
        assert_eq!(remote_word("/opt/g"), "\"/opt/g\"");
    }

    #[test]
    fn remote_command_refuses_quotes_and_dollars() {
        assert!(ssh("vps", Some("/srv/a'b")).remote_command(None).is_err());
        assert!(ssh("vps", Some("/srv/$x")).remote_command(None).is_err());
        assert!(ssh("vps", None).remote_command(Some("/opt/\"g")).is_err());
        assert!(Target::Local(PathBuf::from("/x")).remote_command(None).is_err());
    }

    #[test]
    fn hosts_that_are_options_or_hold_separators_are_refused() {
        assert!(validate_host("-oProxyCommand=x").is_err());
        assert!(validate_host("a b").is_err());
        assert!(validate_host("").is_err());
        assert!(validate_host("a:b").is_err());
        assert!(validate_host("user@vps.example").is_ok());
    }

    #[test]
    fn ssh_args_put_options_before_the_destination() {
        assert_eq!(
            ssh_args("vps", "CMD"),
            ["-t", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", "vps", "CMD"]
        );
    }

    #[test]
    fn picker_input_parses_host_and_optional_path() {
        assert_eq!(
            Target::parse_ssh_input("vps:/srv/a").unwrap(),
            ssh("vps", Some("/srv/a"))
        );
        assert_eq!(Target::parse_ssh_input(" vps ").unwrap(), ssh("vps", None));
        assert_eq!(Target::parse_ssh_input("vps:").unwrap(), ssh("vps", None));
        assert_eq!(
            Target::parse_ssh_input("vps:~/p").unwrap(),
            ssh("vps", Some("~/p"))
        );
        assert!(Target::parse_ssh_input("vps:srv/a").is_err());
        assert!(Target::parse_ssh_input("vps:/a'b").is_err());
    }

    #[test]
    fn display_and_host() {
        assert_eq!(ssh("vps", Some("/srv/a")).display(), "vps:/srv/a");
        assert_eq!(ssh("vps", None).display(), "vps");
        assert_eq!(ssh("vps", None).host(), Some("vps"));
        assert!(ssh("vps", None).is_ssh());
        assert!(!Target::Local(PathBuf::from("x")).is_ssh());
    }
}
