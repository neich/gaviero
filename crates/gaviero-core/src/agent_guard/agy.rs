//! agy: gaviero's entries in `<folder>/.agents/hooks.json` and
//! `<folder>/.agents/skills.json`.
//!
//! Probed on agy 1.5.18 (2026-11-26): a `PreToolUse` hook in the workspace
//! `.agents/hooks.json` runs under `agy acp`, receives `toolCall {name, args}`
//! and blocks the call with `{"decision":"deny"}` — whatever any other named
//! hook answered. agy takes no per-session hook or permission flag on
//! `agy acp`, so this file is the only read/tool barrier gaviero has for agy.
//! A `skills.json` entry catalogues a skill folder that lies inside the
//! session's repository (paths outside it are ignored), which is how agy's
//! own skill list learns `.gaviero/skills`.
//!
//! Both files are **merged, never replaced**: gaviero owns the `"gaviero"` key
//! of `hooks.json` and the `.gaviero/skills` entry of `skills.json`, and keeps
//! every other key and entry the user wrote.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The `hooks.json` key gaviero owns.
pub const GAVIERO_HOOK_NAME: &str = "gaviero";

/// The `skills.json` entry path gaviero owns.
pub const GAVIERO_SKILLS_ENTRY: &str = ".gaviero/skills";

/// The hook command line. Paths use `/`; on Windows a path with a space is
/// wrapped in PowerShell's call operator (same rule as Cursor's hook).
fn hook_command(cli_binary: &str) -> String {
    let cli = cli_binary.replace('\\', "/");
    if cfg!(windows) && cli.contains(' ') {
        format!("& '{cli}' --hook agy:pre-tool-use")
    } else {
        format!("{cli} --hook agy:pre-tool-use")
    }
}

/// Read `path` as a JSON object, or `{}` when it is missing or not an object.
fn read_object(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Write `doc` (pretty) to `path` unless the bytes are already there.
/// Returns whether the file changed.
fn write_if_changed(path: &Path, doc: &Value) -> anyhow::Result<bool> {
    let body = serde_json::to_string_pretty(doc)?;
    if std::fs::read_to_string(path).ok().as_deref() == Some(body.as_str()) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)?;
    Ok(true)
}

/// Merge gaviero's agy entries into `<folder>/.agents/`. Returns the paths it
/// actually changed (empty when everything was already current).
pub fn ensure_agy_files(folder: &Path, cli_binary: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    let agents = folder.join(".agents");

    let hooks_path = agents.join("hooks.json");
    let mut hooks = read_object(&hooks_path);
    hooks.as_object_mut().expect("object").insert(
        GAVIERO_HOOK_NAME.to_string(),
        json!({
            "PreToolUse": [{
                "matcher": ".*",
                "hooks": [{
                    "type": "command",
                    "command": hook_command(cli_binary),
                    "timeout": 10
                }]
            }]
        }),
    );
    if write_if_changed(&hooks_path, &hooks)? {
        changed.push(hooks_path);
    }

    if folder.join(".gaviero").join("skills").is_dir() {
        let skills_path = agents.join("skills.json");
        let mut skills = read_object(&skills_path);
        let obj = skills.as_object_mut().expect("object");
        let entries = obj.entry("entries").or_insert_with(|| json!([]));
        if !entries.is_array() {
            *entries = json!([]);
        }
        let list = entries.as_array_mut().expect("array");
        let present = list.iter().any(|entry| {
            entry.get("path").and_then(Value::as_str) == Some(GAVIERO_SKILLS_ENTRY)
        });
        if !present {
            list.push(json!({ "path": GAVIERO_SKILLS_ENTRY }));
        }
        if write_if_changed(&skills_path, &skills)? {
            changed.push(skills_path);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn writes_hooks_and_keeps_user_entries() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("hooks.json"),
            r#"{"mine": {"PostToolUse": [{"matcher": "run_command", "hooks": [{"type": "command", "command": "lint"}]}]}}"#,
        )
        .unwrap();
        let changed = ensure_agy_files(dir.path(), "/usr/bin/gaviero-cli").unwrap();
        assert_eq!(changed, vec![agents.join("hooks.json")]);
        let doc = read(&agents.join("hooks.json"));
        assert_eq!(doc["mine"]["PostToolUse"][0]["hooks"][0]["command"], "lint");
        let entry = &doc[GAVIERO_HOOK_NAME]["PreToolUse"][0];
        assert_eq!(entry["matcher"], ".*");
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert_eq!(entry["hooks"][0]["timeout"], 10);
    }

    #[test]
    fn rewrite_is_a_noop_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero/skills")).unwrap();
        let first = ensure_agy_files(dir.path(), "gaviero-cli").unwrap();
        assert_eq!(first.len(), 2, "{first:?}");
        let second = ensure_agy_files(dir.path(), "gaviero-cli").unwrap();
        assert!(second.is_empty(), "{second:?}");
        // A new CLI path rewrites only the hook.
        let third = ensure_agy_files(dir.path(), "/opt/gaviero-cli").unwrap();
        assert_eq!(third, vec![dir.path().join(".agents/hooks.json")]);
    }

    #[test]
    fn skills_entry_added_only_when_folder_exists() {
        let dir = tempfile::tempdir().unwrap();
        ensure_agy_files(dir.path(), "gaviero-cli").unwrap();
        assert!(!dir.path().join(".agents/skills.json").exists());
        std::fs::create_dir_all(dir.path().join(".gaviero/skills")).unwrap();
        ensure_agy_files(dir.path(), "gaviero-cli").unwrap();
        let doc = read(&dir.path().join(".agents/skills.json"));
        assert_eq!(doc["entries"], json!([{ "path": ".gaviero/skills" }]));
    }

    #[test]
    fn skills_entry_not_duplicated_and_user_entries_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero/skills")).unwrap();
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        std::fs::write(
            dir.path().join(".agents/skills.json"),
            r#"{"entries": [{"path": "tools/agent-skills", "exclude": ["load-test"]}, {"path": ".gaviero/skills"}], "exclude": ["old"]}"#,
        )
        .unwrap();
        ensure_agy_files(dir.path(), "gaviero-cli").unwrap();
        let doc = read(&dir.path().join(".agents/skills.json"));
        let entries = doc["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{doc}");
        assert_eq!(entries[0]["path"], "tools/agent-skills");
        assert_eq!(entries[0]["exclude"], json!(["load-test"]));
        assert_eq!(doc["exclude"], json!(["old"]));
    }

    #[test]
    fn hook_command_names_agy_pre_tool_use() {
        assert_eq!(
            hook_command("/usr/local/bin/gaviero-cli"),
            "/usr/local/bin/gaviero-cli --hook agy:pre-tool-use"
        );
        assert!(hook_command(r"C:\tools\gaviero-cli.exe").starts_with("C:/tools/"));
    }
}
