//! Cursor: `.cursor/hooks.json` entries that route file reads and shell
//! commands through `gaviero-cli --hook`.
//!
//! Probed on the cursor-agent CLI (2026-10-31): `beforeReadFile` and
//! `beforeShellExecution` fire in headless `-p` runs and a
//! `{"permission":"deny"}` answer blocks the call, while a `.cursor/cli.json`
//! `Read(...)` deny did not stop either the read tool or `cat`. Hooks are the
//! only read barrier Cursor honours.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Marker every gaviero-managed entry's command contains.
const GAVIERO_HOOK_MARKER: &str = "--hook cursor:";

/// The hook command line for `event` (`before-read-file`, …). Paths use `/`;
/// on Windows a path with a space is wrapped in PowerShell's call operator.
fn hook_command(cli_binary: &str, event: &str) -> String {
    let cli = cli_binary.replace('\\', "/");
    if cfg!(windows) && cli.contains(' ') {
        format!("& '{cli}' --hook cursor:{event}")
    } else {
        format!("{cli} --hook cursor:{event}")
    }
}

/// Merge gaviero's `beforeReadFile` / `beforeShellExecution` entries into
/// `<root>/.cursor/hooks.json`, replacing any older gaviero entry and
/// keeping every other key and entry. Returns the path when the file changed.
pub fn ensure_cursor_hooks(workspace_root: &Path, cli_binary: &str) -> anyhow::Result<Option<PathBuf>> {
    let path = workspace_root.join(".cursor").join("hooks.json");
    let mut doc: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let obj = doc.as_object_mut().expect("object");
    obj.entry("version").or_insert(json!(1));
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().expect("object");
    for (event_key, event_arg) in [
        ("beforeReadFile", "before-read-file"),
        ("beforeShellExecution", "before-shell-execution"),
    ] {
        let list = hooks.entry(event_key).or_insert_with(|| json!([]));
        if !list.is_array() {
            *list = json!([]);
        }
        let list = list.as_array_mut().expect("array");
        list.retain(|entry| {
            !entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| c.contains(GAVIERO_HOOK_MARKER))
        });
        list.push(json!({ "command": hook_command(cli_binary, event_arg) }));
    }
    let body = serde_json::to_string_pretty(&doc)?;
    if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
        return Ok(None);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, body)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn ensure_cursor_hooks_writes_both_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = ensure_cursor_hooks(dir.path(), "C:/bin/gaviero-cli.exe")
            .unwrap()
            .unwrap();
        let v = read(&path);
        assert_eq!(v["version"], 1);
        assert_eq!(
            v["hooks"]["beforeReadFile"][0]["command"],
            "C:/bin/gaviero-cli.exe --hook cursor:before-read-file"
        );
        assert_eq!(
            v["hooks"]["beforeShellExecution"][0]["command"],
            "C:/bin/gaviero-cli.exe --hook cursor:before-shell-execution"
        );
    }

    #[test]
    fn ensure_cursor_hooks_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_cursor_hooks(dir.path(), "gaviero-cli").unwrap().is_some());
        assert!(ensure_cursor_hooks(dir.path(), "gaviero-cli").unwrap().is_none());
    }

    #[test]
    fn ensure_cursor_hooks_keeps_user_entries_and_replaces_old_gaviero_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".cursor")).unwrap();
        std::fs::write(
            dir.path().join(".cursor/hooks.json"),
            r#"{"version":1,"hooks":{
                "beforeReadFile":[{"command":"my-audit.sh"},{"command":"/old/gaviero-cli --hook cursor:before-read-file"}],
                "afterFileEdit":[{"command":"fmt.sh"}]},
              "custom":true}"#,
        )
        .unwrap();
        let path = ensure_cursor_hooks(dir.path(), "/new/gaviero-cli").unwrap().unwrap();
        let v = read(&path);
        let reads = v["hooks"]["beforeReadFile"].as_array().unwrap();
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0]["command"], "my-audit.sh");
        assert_eq!(reads[1]["command"], "/new/gaviero-cli --hook cursor:before-read-file");
        assert_eq!(v["hooks"]["afterFileEdit"][0]["command"], "fmt.sh");
        assert_eq!(v["custom"], true);
    }

    #[test]
    fn hook_command_quotes_spaced_paths_on_windows() {
        let cmd = hook_command(r"C:\Program Files\gaviero\gaviero-cli.exe", "before-read-file");
        if cfg!(windows) {
            assert_eq!(
                cmd,
                "& 'C:/Program Files/gaviero/gaviero-cli.exe' --hook cursor:before-read-file"
            );
        } else {
            assert!(cmd.ends_with("--hook cursor:before-read-file"));
        }
    }
}
