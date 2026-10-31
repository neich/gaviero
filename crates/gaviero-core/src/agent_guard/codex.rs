//! Codex app-server: the read policy as a `thread/start` / `thread/resume`
//! `config` override.
//!
//! Probed on codex-cli 0.163.0 (2026-10-31): both requests accept a `config`
//! object; a `default_permissions` profile extending `:read-only` with
//! `filesystem` `"deny"` entries is applied to sandboxed commands. Under the
//! Windows `unelevated` sandbox Codex refuses deny-read and fails the thread
//! ("cannot enforce deny-read restrictions directly; refusing to run
//! unsandboxed"); with `windows.sandbox = "mxc"` the thread starts. Gaviero
//! therefore always asks for MXC on Windows and fails closed otherwise.
//! Codex hooks are not used: `config.hooks` never ran in the probe.
//!
//! Every workspace entry is a glob: under MXC, a *literal* deny path that
//! does not exist fails thread creation ("CreateProcessSecurityEnvironment
//! failed (HRESULT = 0x80070003)", probed 2026-10-31), so literal entries
//! (home credentials, user `read.deny` paths without a wildcard) are only
//! emitted when the path exists.
//!
//! `sensitivePaths.allow` exemptions are **not** emitted: Codex precedence
//! between a `read` and a `deny` entry for the same path is unverified, so an
//! exempted file stays denied for Codex.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::scope_enforcer::{
    BLOCKED, GAVIERO_STATE_FLOOR, GAVIERO_STATE_FLOOR_PREFIXES, HOME_FLOOR,
    READ_ONLY_EXTRA_EXTENSIONS, READ_ONLY_EXTRA_NAMES, SensitivePolicy,
};

/// Name of the permission profile gaviero defines.
pub const CODEX_PROFILE: &str = "gaviero";

fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

/// The `config` object for `thread/start` / `thread/resume`.
pub fn codex_thread_config(workspace_root: &Path, allow_network: bool) -> Value {
    let policy = SensitivePolicy::resolve(workspace_root);
    let mut in_roots = Map::new();
    let mut deny = |glob: String| {
        in_roots.insert(glob, json!("deny"));
    };
    for name in BLOCKED.iter().chain(READ_ONLY_EXTRA_NAMES.iter()) {
        deny(format!("**/{name}"));
        deny(format!("**/{name}/**"));
    }
    for ext in READ_ONLY_EXTRA_EXTENSIONS {
        deny(format!("**/*.{ext}"));
    }
    deny("**/.env.*".to_string());
    for (entry, _) in GAVIERO_STATE_FLOOR {
        deny(format!("**/.gaviero/{entry}"));
        deny(format!("**/.gaviero/{entry}/**"));
    }
    for (prefix, _) in GAVIERO_STATE_FLOOR_PREFIXES {
        deny(format!("**/.gaviero/{prefix}*"));
    }
    for glob in policy.read_deny() {
        if is_glob(glob) || workspace_root.join(glob).exists() {
            deny(glob.clone());
        }
    }

    let mut filesystem = Map::new();
    filesystem.insert(":workspace_roots".to_string(), Value::Object(in_roots));
    if let Some(home) = dirs::home_dir() {
        for (seq, _) in HOME_FLOOR {
            let path = seq.iter().fold(home.clone(), |p, c| p.join(c));
            if path.exists() {
                filesystem.insert(path.to_string_lossy().replace('\\', "/"), json!("deny"));
            }
        }
    }

    let mut config = json!({
        "default_permissions": CODEX_PROFILE,
        "permissions": {
            CODEX_PROFILE: {
                "extends": ":read-only",
                "network": { "enabled": allow_network },
                "filesystem": Value::Object(filesystem),
            }
        },
        "shell_environment_policy": { "ignore_default_excludes": false },
    });
    if cfg!(windows) {
        config["windows"] = json!({ "sandbox": "mxc" });
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_thread_config_selects_the_gaviero_profile() {
        let dir = tempfile::tempdir().unwrap();
        let v = codex_thread_config(dir.path(), false);
        assert_eq!(v["default_permissions"], "gaviero");
        let profile = &v["permissions"]["gaviero"];
        assert_eq!(profile["extends"], ":read-only");
        assert_eq!(profile["network"]["enabled"], false);
        assert_eq!(profile["filesystem"][":workspace_roots"]["**/.env"], "deny");
        assert_eq!(profile["filesystem"][":workspace_roots"]["**/.gaviero/history/**"], "deny");
        // No literal workspace entry: MXC rejects a literal path that is missing.
        for key in profile["filesystem"][":workspace_roots"].as_object().unwrap().keys() {
            assert!(is_glob(key), "literal entry {key}");
        }
        assert_eq!(v["shell_environment_policy"]["ignore_default_excludes"], false);
        assert!(v.get("sandbox").is_none());
        if cfg!(windows) {
            assert_eq!(v["windows"]["sandbox"], "mxc");
        }
    }

    #[test]
    fn codex_thread_config_denies_home_credentials_and_read_denies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            r#"{"agent":{"permissions":{"read":{"deny":["private/**","missing.txt"]}}}}"#,
        )
        .unwrap();
        let v = codex_thread_config(dir.path(), true);
        let fs = &v["permissions"]["gaviero"]["filesystem"];
        assert_eq!(fs[":workspace_roots"]["private/**"], "deny");
        assert_eq!(v["permissions"]["gaviero"]["network"]["enabled"], true);
        let home = dirs::home_dir().unwrap();
        let auth_path = home.join(".codex").join("auth.json");
        let auth = auth_path.to_string_lossy().replace('\\', "/");
        if auth_path.exists() {
            assert_eq!(fs[auth.as_str()], "deny");
        } else {
            assert!(fs.get(auth.as_str()).is_none(), "missing literal path emitted");
        }
        // A literal read.deny path that does not exist is not emitted.
        assert!(fs[":workspace_roots"].get("missing.txt").is_none());
    }
}
