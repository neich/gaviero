//! Claude Code: the read policy as a `--settings` layer.
//!
//! Passed at command-line precedence (above project/user files, below
//! managed settings), so `.gaviero/settings.json` decides without writing a
//! provider file. Verified live on Claude Code 2.1.306 (2026-10-31): `Read`
//! deny rules from `--settings` block the Read tool, Claude's Grep tool and
//! `cat`; `permissions.blockReadsOutsideWorkingDirectories` also covers Bash;
//! `grep -r` still leaks, which the `PostToolUse` Bash hook
//! (`gaviero-cli --hook claude:post-tool-use`) redacts.
//!
//! Rule anchors follow the Claude docs: unanchored `**/x` is relative to the
//! session cwd, `//c/…` is absolute (Windows paths normalize to `/c/…`),
//! `~/…` is the home directory. A `Read(!glob)` carve-out only reaches
//! unanchored rules from the same source, so `sensitivePaths.allow`
//! exemptions do not apply inside sibling roots.

use std::path::Path;

use serde_json::{Value, json};

use crate::scope_enforcer::{
    BLOCKED, ENV_TEMPLATE_SUFFIXES, GAVIERO_STATE_FLOOR, GAVIERO_STATE_FLOOR_PREFIXES,
    HOME_FLOOR, READ_ONLY_EXTRA_EXTENSIONS, READ_ONLY_EXTRA_NAMES, READ_ONLY_EXTRA_SUFFIX_PATHS,
    SensitivePolicy,
};

/// `C:\x\y` → `c/x/y`; `/home/x` → `home/x`. Prefix with `//` for an
/// absolute Claude rule.
pub fn claude_posix_root(root: &Path) -> String {
    let s = root.to_string_lossy().replace('\\', "/");
    let bytes = s.as_bytes();
    let s = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        format!("{}{}", s[..1].to_ascii_lowercase(), &s[2..])
    } else {
        s
    };
    s.trim_start_matches('/').trim_end_matches('/').to_string()
}

/// The `Read(...)` deny and carve-out rules for `workspace_root` plus its
/// sibling roots, compiled from [`SensitivePolicy::resolve`].
pub fn claude_read_rules(workspace_root: &Path, additional_roots: &[&Path]) -> Vec<String> {
    let policy = SensitivePolicy::resolve(workspace_root);
    let roots: Vec<String> = additional_roots
        .iter()
        .filter(|r| !r.as_os_str().is_empty() && **r != workspace_root)
        .map(|r| claude_posix_root(r))
        .collect();
    let mut rules: Vec<String> = Vec::new();
    let mut everywhere = |tail: &str| {
        rules.push(format!("Read(**/{tail})"));
        for root in &roots {
            rules.push(format!("Read(//{root}/**/{tail})"));
        }
    };
    for name in BLOCKED.iter().chain(READ_ONLY_EXTRA_NAMES.iter()) {
        everywhere(name);
        everywhere(&format!("{name}/**"));
    }
    for ext in READ_ONLY_EXTRA_EXTENSIONS {
        everywhere(&format!("*.{ext}"));
    }
    everywhere(".env.*");
    for seq in READ_ONLY_EXTRA_SUFFIX_PATHS {
        everywhere(&seq.join("/"));
    }
    for (seq, _) in HOME_FLOOR {
        let tail = seq.join("/");
        // `~/.gaviero/remote` is a directory.
        rules.push(format!("Read(~/{tail})"));
        rules.push(format!("Read(~/{tail}/**)"));
    }
    let primary = claude_posix_root(workspace_root);
    for root in std::iter::once(&primary).chain(roots.iter()) {
        for (entry, _) in GAVIERO_STATE_FLOOR {
            rules.push(format!("Read(//{root}/.gaviero/{entry})"));
            rules.push(format!("Read(//{root}/.gaviero/{entry}/**)"));
        }
        for (prefix, _) in GAVIERO_STATE_FLOOR_PREFIXES {
            rules.push(format!("Read(//{root}/.gaviero/{prefix}*)"));
        }
    }
    for glob in policy.read_deny() {
        rules.push(format!("Read({glob})"));
    }
    // Committed `.env` templates stay readable, as in the block-list.
    for suffix in ENV_TEMPLATE_SUFFIXES {
        rules.push(format!("Read(!**/.env{suffix})"));
    }
    for glob in policy.allow_globs() {
        rules.push(format!("Read(!{glob})"));
    }
    rules.sort();
    rules.dedup();
    rules
}

/// The full `--settings` JSON for one Claude session.
pub fn claude_session_settings(
    workspace_root: &Path,
    additional_roots: &[&Path],
    skip_native_instructions: bool,
    block_outside: bool,
    cli_binary: &str,
) -> Value {
    let mut root = json!({
        "permissions": {
            "deny": claude_read_rules(workspace_root, additional_roots),
            "blockReadsOutsideWorkingDirectories": block_outside,
        },
        "hooks": {
            "PostToolUse": [{
                "matcher": "Bash",
                "hooks": [{
                    "type": "command",
                    "command": cli_binary,
                    "args": ["--hook", "claude:post-tool-use"],
                    "timeout": 30,
                    "onFailure": "block",
                }],
            }],
        },
    });
    if skip_native_instructions
        && let Ok(Value::Object(extra)) =
            serde_json::from_str::<Value>(&crate::instructions::claude_settings_arg())
        && let Some(obj) = root.as_object_mut()
    {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }
    root
}

/// `agent.permissions.read.blockOutsideWorkspace` from
/// `<root>/.gaviero/settings.json`, read directly like
/// [`SensitivePolicy::resolve`]. Default `true`.
pub fn read_block_outside(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".gaviero").join("settings.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|doc| {
            doc.pointer("/agent/permissions/read/blockOutsideWorkspace")
                .and_then(Value::as_bool)
        })
        .unwrap_or(true)
}

/// User-level `~/.claude/settings.json` allow rules that pre-approve file or
/// shell access, bypassing gaviero's permission prompts. (Deny rules from
/// `.gaviero/settings.json` still apply; Claude evaluates deny first.)
pub fn user_allow_overlaps(home: &Path) -> Vec<String> {
    let Some(doc) = std::fs::read_to_string(home.join(".claude").join("settings.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
    else {
        return Vec::new();
    };
    doc.pointer("/permissions/allow")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|rule| {
                    let tool = rule.split('(').next().unwrap_or(rule).trim();
                    matches!(tool, "Read" | "Grep" | "Glob" | "Bash")
                })
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deny(v: &Value) -> Vec<String> {
        v["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn claude_session_settings_denies_env_and_agent_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let v = claude_session_settings(dir.path(), &[], false, true, "gaviero-cli");
        let rules = deny(&v);
        assert!(rules.contains(&"Read(**/.env)".to_string()));
        assert!(rules.contains(&"Read(~/.codex/auth.json)".to_string()));
        assert!(rules.contains(&"Read(**/*.pem)".to_string()));
        assert!(rules.contains(&"Read(!**/.env.example)".to_string()));
        assert_eq!(v["permissions"]["blockReadsOutsideWorkingDirectories"], true);
        assert!(v.get("claudeMdExcludes").is_none());
    }

    #[test]
    fn claude_session_settings_anchors_state_floor_at_each_root() {
        let dir = tempfile::tempdir().unwrap();
        let sibling = Path::new(r"C:\x\y");
        let v = claude_session_settings(dir.path(), &[sibling], false, false, "gaviero-cli");
        let rules = deny(&v);
        assert!(rules.contains(&"Read(//c/x/y/.gaviero/history/**)".to_string()));
        assert!(rules.contains(&"Read(//c/x/y/**/.env)".to_string()));
        assert_eq!(v["permissions"]["blockReadsOutsideWorkingDirectories"], false);
    }

    #[test]
    fn claude_session_settings_carries_exemptions_and_read_denies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            r#"{"agent":{"permissions":{
                "sensitivePaths":{"allow":["tests/fixtures/**"]},
                "read":{"deny":["private/**"]}}}}"#,
        )
        .unwrap();
        let rules = deny(&claude_session_settings(dir.path(), &[], false, true, "x"));
        assert!(rules.contains(&"Read(!tests/fixtures/**)".to_string()));
        assert!(rules.contains(&"Read(private/**)".to_string()));
    }

    #[test]
    fn claude_session_settings_wires_the_bash_hook() {
        let dir = tempfile::tempdir().unwrap();
        let v = claude_session_settings(dir.path(), &[], false, true, "C:/bin/gaviero-cli.exe");
        let hook = &v["hooks"]["PostToolUse"][0];
        assert_eq!(hook["matcher"], "Bash");
        assert_eq!(hook["hooks"][0]["command"], "C:/bin/gaviero-cli.exe");
        assert_eq!(
            hook["hooks"][0]["args"],
            json!(["--hook", "claude:post-tool-use"])
        );
        assert_eq!(hook["hooks"][0]["onFailure"], "block");
    }

    #[test]
    fn claude_session_settings_merges_native_instruction_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let v = claude_session_settings(dir.path(), &[], true, true, "x");
        assert!(v["claudeMdExcludes"].is_array());
        assert!(v["permissions"]["deny"].is_array());
    }

    #[test]
    fn claude_posix_root_normalizes_drive_and_unix_paths() {
        assert_eq!(claude_posix_root(Path::new(r"C:\Users\a")), "c/Users/a");
        assert_eq!(claude_posix_root(Path::new("/home/a/")), "home/a");
    }

    #[test]
    fn read_block_outside_defaults_true_and_reads_settings() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_block_outside(dir.path()));
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            r#"{"agent":{"permissions":{"read":{"blockOutsideWorkspace":false}}}}"#,
        )
        .unwrap();
        assert!(!read_block_outside(dir.path()));
    }

    #[test]
    fn user_allow_overlaps_lists_file_and_shell_rules() {
        let dir = tempfile::tempdir().unwrap();
        assert!(user_allow_overlaps(dir.path()).is_empty());
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"permissions":{"allow":["Bash(cat:*)","Read","WebFetch","mcp__x__y","Glob(**)"]}}"#,
        )
        .unwrap();
        assert_eq!(
            user_allow_overlaps(dir.path()),
            vec!["Bash(cat:*)", "Read", "Glob(**)"]
        );
    }
}
