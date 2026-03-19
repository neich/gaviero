use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Well-known setting keys. Use these constants instead of raw strings
/// to get compile-time typo detection.
pub mod settings {
    pub const TAB_SIZE: &str = "editor.tabSize";
    pub const INSERT_SPACES: &str = "editor.insertSpaces";
    pub const FORMAT_ON_SAVE: &str = "editor.formatOnSave";
    pub const FILES_EXCLUDE: &str = "files.exclude";
    pub const FILE_TREE_WIDTH: &str = "panels.fileTree.width";
    pub const SIDE_PANEL_WIDTH: &str = "panels.sidePanel.width";
    pub const TERMINAL_SPLIT_PERCENT: &str = "panels.terminal.splitPercent";
    pub const GIT_TREE_ALLOW_LIST: &str = "git.treeAllowList";

    // Agent / Claude settings
    pub const AGENT_MODEL: &str = "agent.model";
    pub const AGENT_EFFORT: &str = "agent.effort";
    pub const AGENT_MAX_TOKENS: &str = "agent.maxTokens";

    // Memory settings
    /// The namespace to write memories to.
    pub const MEMORY_NAMESPACE: &str = "memory.namespace";
    /// Additional namespaces to search when reading (the write namespace is always included).
    pub const MEMORY_READ_NAMESPACES: &str = "memory.readNamespaces";
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

impl WorkspaceFolder {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .unwrap_or_else(|| self.path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkspaceFile {
    folders: Vec<WorkspaceFolder>,
    #[serde(default)]
    settings: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    folders: Vec<WorkspaceFolder>,
    workspace_settings: serde_json::Value,
    workspace_path: Option<PathBuf>,
}

impl Workspace {
    /// Load a `.gaviero-workspace` file.
    pub fn load(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).context("reading .gaviero-workspace file")?;
        let file: WorkspaceFile =
            serde_json::from_str(&content).context("parsing .gaviero-workspace file")?;

        Ok(Self {
            folders: file.folders,
            workspace_settings: file.settings,
            workspace_path: Some(path.to_path_buf()),
        })
    }

    /// Create a temporary single-root workspace (no file on disk).
    pub fn single_folder(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // In single-folder mode, load .gaviero/settings.json as workspace settings
        // so that resolve_setting(key, None) finds them.
        let settings_path = path.join(".gaviero").join("settings.json");
        let workspace_settings = match std::fs::read_to_string(&settings_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(val) => val,
                Err(e) => {
                    tracing::warn!(
                        "Invalid JSON in {}: {} — all settings will use defaults",
                        settings_path.display(),
                        e
                    );
                    serde_json::Value::Object(serde_json::Map::new())
                }
            },
            Err(_) => serde_json::Value::Object(serde_json::Map::new()),
        };

        Self {
            folders: vec![WorkspaceFolder {
                path,
                name: Some(name),
            }],
            workspace_settings,
            workspace_path: None,
        }
    }

    /// Ensure `.gaviero/settings.json` exists for all workspace roots.
    /// Creates the directory and a default settings file if missing.
    pub fn ensure_settings(&mut self) {
        for folder in &self.folders {
            let gaviero_dir = folder.path.join(".gaviero");
            let settings_path = gaviero_dir.join("settings.json");

            if settings_path.exists() {
                continue;
            }

            // Derive namespace from folder name
            let namespace = folder.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();

            let defaults = serde_json::json!({
                "files": {
                    "exclude": {
                        ".DS_Store": true,
                        ".cache": true,
                        ".gradle": true,
                        ".idea": true,
                        ".mvn": true,
                        ".mypy_cache": true,
                        ".next": true,
                        ".nuxt": true,
                        ".parcel-cache": true,
                        ".pytest_cache": true,
                        ".tox": true,
                        ".venv": true,
                        "Thumbs.db": true,
                        "__pycache__": true,
                        "build": true,
                        "coverage": true,
                        "dist": true,
                        "node_modules": true,
                        "out": true,
                        "target": true,
                        "venv": true
                    }
                },
                "git": {
                    "treeAllowList": ["config", "description", "HEAD", "hooks", "info"]
                },
                "memory": {
                    "namespace": namespace,
                    "readNamespaces": []
                },
                "panels": {
                    "fileTree": { "width": 25 },
                    "sidePanel": { "width": 50 },
                    "layouts": {
                        "1": [15, 60, 25],
                        "2": [0, 100, 0],
                        "3": [20, 80, 0],
                        "4": [0, 70, 30],
                        "5": [15, 45, 40]
                    }
                }
            });

            if let Err(e) = std::fs::create_dir_all(&gaviero_dir) {
                tracing::warn!("failed to create {}: {}", gaviero_dir.display(), e);
                continue;
            }

            let content = serde_json::to_string_pretty(&defaults).unwrap_or_default();
            if let Err(e) = std::fs::write(&settings_path, &content) {
                tracing::warn!("failed to write {}: {}", settings_path.display(), e);
                continue;
            }

            tracing::info!("created default settings at {}", settings_path.display());

            // Reload into workspace_settings so the current session uses them
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                self.workspace_settings = val;
            }
        }
    }

    /// Return all workspace root paths.
    pub fn roots(&self) -> Vec<&Path> {
        self.folders.iter().map(|f| f.path.as_path()).collect()
    }

    /// Return workspace folders.
    pub fn folders(&self) -> &[WorkspaceFolder] {
        &self.folders
    }

    /// Add a root folder and optionally save to disk.
    pub fn add_root(&mut self, path: PathBuf, name: Option<String>) {
        self.folders.push(WorkspaceFolder { path, name });
    }

    /// Save the workspace to its file (if it has one).
    pub fn save(&self) -> Result<()> {
        let path = self
            .workspace_path
            .as_ref()
            .context("no workspace file path (single-folder mode)")?;
        let file = WorkspaceFile {
            folders: self.folders.clone(),
            settings: self.workspace_settings.clone(),
        };
        let content = serde_json::to_string_pretty(&file)?;
        std::fs::write(path, content).context("writing workspace file")?;
        Ok(())
    }

    /// Resolve a setting using the cascade:
    /// 1. Per-folder `.gaviero/settings.json` (if root provided)
    /// 2. Workspace-level settings
    /// 3. User-level `~/.config/gaviero/settings.json`
    /// 4. Hardcoded defaults
    pub fn resolve_setting(&self, key: &str, root: Option<&Path>) -> serde_json::Value {
        // 1. Per-folder settings
        if let Some(root) = root {
            let folder_settings_path = root.join(".gaviero").join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&folder_settings_path) {
                if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(val) = dot_get(&settings, key) {
                        return val.clone();
                    }
                }
            }
        }

        // 2. Workspace-level settings
        if let Some(val) = dot_get(&self.workspace_settings, key) {
            return val.clone();
        }

        // 3. User-level settings
        if let Some(config_dir) = dirs::config_dir() {
            let user_settings_path = config_dir.join("gaviero").join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&user_settings_path) {
                if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(val) = dot_get(&settings, key) {
                        return val.clone();
                    }
                }
            }
        }

        // 4. Hardcoded defaults
        hardcoded_default(key)
    }

    /// Resolve a language-specific setting.
    /// Checks `[language].key` before `key` at each cascade level.
    pub fn resolve_language_setting(
        &self,
        key: &str,
        language: &str,
        root: Option<&Path>,
    ) -> serde_json::Value {
        let lang_key = format!("[{language}].{key}");
        let val = self.resolve_setting(&lang_key, root);
        if !val.is_null() {
            return val;
        }
        self.resolve_setting(key, root)
    }

    /// Resolve the write namespace (where new memories are stored).
    ///
    /// Cascade: per-folder `memory.namespace` → workspace-level → folder name fallback.
    pub fn resolve_namespace(&self, root: Option<&Path>) -> String {
        let val = self.resolve_setting(settings::MEMORY_NAMESPACE, root);
        if let Some(ns) = val.as_str() {
            return ns.to_string();
        }
        // Fallback: derive from the first folder name
        let folder = root
            .or_else(|| self.roots().first().copied());
        folder
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string()
    }

    /// Resolve the read namespaces (searched when querying memory).
    ///
    /// Always includes the write namespace. Additional namespaces come from
    /// `memory.readNamespaces` in settings (a JSON array of strings).
    pub fn resolve_read_namespaces(&self, root: Option<&Path>) -> Vec<String> {
        let write_ns = self.resolve_namespace(root);
        let mut namespaces = vec![write_ns];

        let val = self.resolve_setting(settings::MEMORY_READ_NAMESPACES, root);
        if let Some(arr) = val.as_array() {
            for item in arr {
                if let Some(ns) = item.as_str() {
                    let ns = ns.to_string();
                    if !namespaces.contains(&ns) {
                        namespaces.push(ns);
                    }
                }
            }
        }

        namespaces
    }

    /// List all distinct namespaces across workspace folders.
    pub fn all_namespaces(&self) -> Vec<(String, PathBuf)> {
        self.folders
            .iter()
            .map(|f| {
                let ns = self.resolve_namespace(Some(&f.path));
                (ns, f.path.clone())
            })
            .collect()
    }
}

/// Resolve a dot-notation key like `"editor.tabSize"` into a nested JSON value.
fn dot_get<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match value.get(parts[0]) {
        Some(inner) if parts.len() == 2 => dot_get(inner, parts[1]),
        Some(inner) if parts.len() == 1 => Some(inner),
        _ => None,
    }
}

fn hardcoded_default(key: &str) -> serde_json::Value {
    match key {
        settings::TAB_SIZE => serde_json::json!(4),
        settings::INSERT_SPACES => serde_json::json!(true),
        settings::FORMAT_ON_SAVE => serde_json::json!(false),
        settings::FILES_EXCLUDE => serde_json::json!({}),
        settings::FILE_TREE_WIDTH => serde_json::json!(30),
        settings::SIDE_PANEL_WIDTH => serde_json::json!(40),
        settings::TERMINAL_SPLIT_PERCENT => serde_json::json!(30),
        settings::AGENT_MODEL => serde_json::json!("sonnet"),
        settings::AGENT_EFFORT => serde_json::json!("off"),
        settings::AGENT_MAX_TOKENS => serde_json::json!(16384),
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_single_folder_workspace() {
        let ws = Workspace::single_folder(PathBuf::from("/tmp/test-project"));
        assert_eq!(ws.roots().len(), 1);
        assert_eq!(ws.roots()[0], Path::new("/tmp/test-project"));
    }

    #[test]
    fn test_load_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("test.gaviero-workspace");
        let content = r#"{
            "folders": [
                { "path": "/home/user/project-a", "name": "Project A" },
                { "path": "/home/user/project-b" },
                { "path": "/home/user/project-c", "name": "Project C" }
            ],
            "settings": {
                "editor": { "tabSize": 2 }
            }
        }"#;
        fs::write(&ws_path, content).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        assert_eq!(ws.roots().len(), 3);
        assert_eq!(ws.folders()[0].display_name(), "Project A");
        assert_eq!(ws.folders()[1].display_name(), "project-b");
    }

    #[test]
    fn test_resolve_setting_workspace_level() {
        let ws = Workspace {
            folders: vec![],
            workspace_settings: serde_json::json!({
                "editor": { "tabSize": 2 }
            }),
            workspace_path: None,
        };
        assert_eq!(ws.resolve_setting("editor.tabSize", None), serde_json::json!(2));
    }

    #[test]
    fn test_resolve_setting_falls_to_default() {
        let ws = Workspace::single_folder(PathBuf::from("/tmp/test"));
        assert_eq!(ws.resolve_setting("editor.tabSize", None), serde_json::json!(4));
    }

    #[test]
    fn test_resolve_setting_folder_override() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let gaviero_dir = root.join(".gaviero");
        fs::create_dir_all(&gaviero_dir).unwrap();
        fs::write(
            gaviero_dir.join("settings.json"),
            r#"{ "editor": { "tabSize": 8 } }"#,
        )
        .unwrap();

        let ws = Workspace {
            folders: vec![],
            workspace_settings: serde_json::json!({
                "editor": { "tabSize": 2 }
            }),
            workspace_path: None,
        };
        assert_eq!(
            ws.resolve_setting("editor.tabSize", Some(root)),
            serde_json::json!(8)
        );
    }

    #[test]
    fn test_add_root_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("test.gaviero-workspace");
        fs::write(&ws_path, r#"{"folders":[],"settings":{}}"#).unwrap();

        let mut ws = Workspace::load(&ws_path).unwrap();
        ws.add_root(PathBuf::from("/new/root"), Some("New Root".into()));
        ws.save().unwrap();

        let ws2 = Workspace::load(&ws_path).unwrap();
        assert_eq!(ws2.roots().len(), 1);
    }

    #[test]
    fn test_dot_get() {
        let val = serde_json::json!({"a": {"b": {"c": 42}}});
        assert_eq!(dot_get(&val, "a.b.c"), Some(&serde_json::json!(42)));
        assert_eq!(dot_get(&val, "a.b"), Some(&serde_json::json!({"c": 42})));
        assert_eq!(dot_get(&val, "x.y"), None);
    }
}
