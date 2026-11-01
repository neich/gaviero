//! `.gaviero/settings.json` layers in the editor.
//!
//! Every gaviero instance under one user account shares the global file,
//! `$GAVIERO_ROOT/.gaviero/settings.json` (`~/.gaviero/…` when unset).
//! `/settings` opens it in a violet `global:settings.json` tab; `/settings
//! workspace` and `/settings repo` open the other two layers. Any open layer
//! highlights the keys another layer also sets: struck out when a more
//! specific layer overrides them, amber when they override a broader one,
//! with an end-of-line note naming the layers
//! ([`gaviero_core::settings_overrides`]).

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::*;
use crate::editor::buffer::SettingsOverlay;

/// Tab label of the global settings file.
pub(super) const GLOBAL_TAB_LABEL: &str = "global:settings.json";

/// One settings layer file of this workspace.
#[derive(Clone, Debug)]
pub(super) struct LayerFile {
    /// `global`, `workspace`, `repo <folder>`.
    pub label: String,
    pub path: PathBuf,
    /// 0 global, 1 workspace, 2 repo — broadest first.
    pub rank: u8,
}

/// The layer files that apply here, broadest first. A file that is two
/// layers at once (single-folder mode, or a workspace rooted at
/// `$GAVIERO_ROOT`) is listed once, under its broadest role.
pub(super) fn layer_files(app: &App) -> Vec<LayerFile> {
    let mut out: Vec<LayerFile> = Vec::new();
    let push = |out: &mut Vec<LayerFile>, label: String, path: PathBuf, rank: u8| {
        if !out
            .iter()
            .any(|l| Buffer::paths_refer_to_same_file(&l.path, &path))
        {
            out.push(LayerFile { label, path, rank });
        }
    };
    if let Some(global) = gaviero_core::workspace::user_settings_path() {
        push(&mut out, "global".to_string(), global, 0);
    }
    push(
        &mut out,
        "workspace".to_string(),
        app.workspace.workspace_layer_path(),
        1,
    );
    for folder in app.workspace.folders() {
        push(
            &mut out,
            format!("repo {}", folder.display_name()),
            folder.path.join(".gaviero").join("settings.json"),
            2,
        );
    }
    out
}

/// The layer `path` is, if any.
fn layer_for(app: &App, path: &Path) -> Option<LayerFile> {
    layer_files(app)
        .into_iter()
        .find(|l| Buffer::paths_refer_to_same_file(&l.path, path))
}

/// Mark buffer `idx` as a settings layer when its file is one. Called when a
/// file is opened.
pub(super) fn tag_buffer(app: &mut App, idx: usize) {
    let Some(path) = app.buffers.get(idx).and_then(|b| b.path.clone()) else {
        return;
    };
    let Some(layer) = layer_for(app, &path) else {
        return;
    };
    let buf = &mut app.buffers[idx];
    buf.settings = Some(SettingsOverlay {
        global: layer.rank == 0,
        ..SettingsOverlay::default()
    });
    if layer.rank == 0 {
        buf.tab_label = Some(GLOBAL_TAB_LABEL.to_string());
    }
}

fn text_hash(buf: &Buffer) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for chunk in buf.text.chunks() {
        chunk.hash(&mut hasher);
    }
    hasher.finish()
}

fn read_layer(path: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
}

/// Recompute the active buffer's highlights when its text changed or another
/// layer was saved. Runs after every event; costs one hash of the buffer
/// unless something changed.
pub(super) fn sync(app: &mut App) {
    let idx = app.active_buffer;
    let Some(buf) = app.buffers.get(idx) else {
        return;
    };
    let (Some(overlay), Some(path)) = (buf.settings.as_ref(), buf.path.clone()) else {
        return;
    };
    let hash = text_hash(buf);
    if overlay.text_hash == Some(hash) {
        return;
    }
    let layers = layer_files(app);
    let Some(me) = layers
        .iter()
        .find(|l| Buffer::paths_refer_to_same_file(&l.path, &path))
        .cloned()
    else {
        return;
    };
    let docs = |pick: &dyn Fn(&LayerFile) -> bool| -> Vec<(String, serde_json::Value)> {
        layers
            .iter()
            .filter(|l| pick(l))
            .filter_map(|l| Some((l.label.clone(), read_layer(&l.path)?)))
            .collect()
    };
    let upper = docs(&|l| l.rank < me.rank);
    let lower = docs(&|l| l.rank > me.rank);
    let text = app.buffers[idx].text.to_string();
    let marks = gaviero_core::settings_overrides::override_marks(&text, &upper, &lower);
    if let Some(overlay) = app.buffers[idx].settings.as_mut() {
        overlay.marks = marks;
        overlay.text_hash = Some(hash);
    }
}

/// After a save: when `path` is a settings layer, reload the settings and
/// recompute every open layer's highlights.
pub(super) fn after_save(app: &mut App, path: &Path) {
    if layer_for(app, path).is_none() {
        return;
    }
    app.workspace.reload_settings_cache();
    for buf in &mut app.buffers {
        if let Some(overlay) = buf.settings.as_mut() {
            overlay.text_hash = None;
        }
    }
}

/// `/settings [global|workspace|repo]`: open that layer in an editor tab.
/// The global file is created (`{}`) when missing; the others must exist.
pub(super) fn handle_settings_command(app: &mut App, line: &str) {
    let arg = line.trim().strip_prefix("/settings").unwrap_or("").trim();
    let target = match arg {
        "" | "global" => gaviero_core::workspace::user_settings_path(),
        "workspace" | "ws" => Some(app.workspace.workspace_layer_path()),
        "repo" => focused_folder(app).map(|f| f.join(".gaviero").join("settings.json")),
        _ => {
            app.status_message = Some((
                "Usage: /settings [global|workspace|repo]".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
    };
    let Some(path) = target else {
        app.status_message = Some((
            "No settings location: no home directory or workspace folder".to_string(),
            std::time::Instant::now(),
        ));
        return;
    };
    if !path.is_file() {
        if !matches!(arg, "" | "global") {
            app.status_message = Some((
                format!("{} does not exist", path.display()),
                std::time::Instant::now(),
            ));
            return;
        }
        let created = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, "{}\n"));
        if let Err(e) = created {
            app.status_message = Some((
                format!("Cannot create {}: {e}", path.display()),
                std::time::Instant::now(),
            ));
            return;
        }
        super::turn_review::note_host_write(app, &path);
    }
    app.open_file(&path);
    app.focus = Focus::Editor;
}

/// The folder of the active buffer, else the first workspace folder.
fn focused_folder(app: &App) -> Option<PathBuf> {
    app.buffers
        .get(app.active_buffer)
        .and_then(|b| b.path.as_deref())
        .and_then(|p| app.workspace.folder_for_path(p))
        .or_else(|| app.workspace.roots().first().copied())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<root>/ws.gaviero-workspace` over `<root>/app` and `<root>/lib`.
    fn workspace_app(root: &Path) -> App {
        let root = gaviero_core::util::fs::canonicalize_simplified(root).unwrap();
        for (dir, body) in [
            (root.clone(), r#"{"editor": {"tabSize": 4, "wordWrap": true}}"#),
            (root.join("app"), r#"{"editor": {"tabSize": 2}, "only": 1}"#),
            (root.join("lib"), "{}"),
        ] {
            std::fs::create_dir_all(dir.join(".gaviero")).unwrap();
            std::fs::write(dir.join(".gaviero/settings.json"), body).unwrap();
        }
        let ws_file = root.join("ws.gaviero-workspace");
        let doc = serde_json::json!({"folders": [{"path": root.join("app")}, {"path": root.join("lib")}]});
        std::fs::write(&ws_file, doc.to_string()).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(Workspace::load(&ws_file).unwrap(), tx)
    }

    fn cleanup_session(app: &App) {
        if let Some(dir) = app.workspace.roots().first()
            && let Some(state_dir) = session_state::state_dir_for(dir)
        {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }

    fn marks(app: &App) -> Vec<(String, String)> {
        app.buffers[app.active_buffer]
            .settings
            .as_ref()
            .unwrap()
            .marks
            .iter()
            .map(|m| (m.path.clone(), m.hint()))
            .collect()
    }

    #[test]
    fn layer_files_list_each_layer_once_broadest_first() {
        let dir = tempfile::tempdir().unwrap();
        let app = workspace_app(dir.path());
        let labels: Vec<(String, u8)> = layer_files(&app)
            .into_iter()
            .filter(|l| l.rank > 0)
            .map(|l| (l.label, l.rank))
            .collect();
        assert_eq!(
            labels,
            [
                ("workspace".to_string(), 1),
                ("repo app".to_string(), 2),
                ("repo lib".to_string(), 2)
            ]
        );
        cleanup_session(&app);
    }

    #[test]
    fn open_layers_highlight_overridden_and_overriding_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = workspace_app(dir.path());
        let root = app.workspace.workspace_layer_path();
        app.open_file(&root);
        sync(&mut app);
        assert_eq!(
            marks(&app),
            [("editor.tabSize".to_string(), "← overridden by repo app".to_string())]
        );
        assert!(!app.buffers[app.active_buffer].settings.as_ref().unwrap().global);

        let repo = app.workspace.roots()[0].join(".gaviero/settings.json");
        app.open_file(&repo);
        sync(&mut app);
        assert_eq!(
            marks(&app),
            [("editor.tabSize".to_string(), "overrides workspace".to_string())]
        );

        // Saving a layer refreshes the others: the repo stops overriding.
        std::fs::write(&repo, r#"{"only": 1}"#).unwrap();
        after_save(&mut app, &repo);
        app.open_file(&root);
        sync(&mut app);
        assert!(marks(&app).is_empty());
        cleanup_session(&app);
    }

    #[test]
    fn settings_command_rejects_unknown_layers_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = workspace_app(dir.path());
        let open_before = app.buffers.len();
        handle_settings_command(&mut app, "/settings bogus");
        assert!(app.status_message.as_ref().unwrap().0.starts_with("Usage"));
        std::fs::remove_file(app.workspace.workspace_layer_path()).unwrap();
        handle_settings_command(&mut app, "/settings workspace");
        assert!(app.status_message.as_ref().unwrap().0.ends_with("does not exist"));
        assert_eq!(app.buffers.len(), open_before);
        handle_settings_command(&mut app, "/settings repo");
        let active = &app.buffers[app.active_buffer];
        assert!(active.path.as_deref().is_some_and(|p| p.ends_with("app/.gaviero/settings.json")));
        assert!(active.settings.is_some());
        cleanup_session(&app);
    }

    #[test]
    fn same_relative_paths_in_different_roots_get_a_root_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = workspace_app(dir.path());
        let roots: Vec<PathBuf> = app.workspace.roots().iter().map(|r| r.to_path_buf()).collect();
        // Same name, different relative paths: in one folder and across folders.
        for rel in ["src/mod.rs", "tests/mod.rs"] {
            let path = roots[0].join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
        }
        std::fs::create_dir_all(roots[1].join("lib")).unwrap();
        std::fs::write(roots[1].join("lib/mod.rs"), "").unwrap();
        app.buffers.clear();
        for path in [
            app.workspace.workspace_layer_path(),
            roots[0].join(".gaviero/settings.json"),
            roots[1].join(".gaviero/settings.json"),
            roots[0].join("src/mod.rs"),
            roots[0].join("tests/mod.rs"),
            roots[1].join("lib/mod.rs"),
        ] {
            app.open_file(&path);
        }
        assert_eq!(
            super::super::render::buffer_labels(&app),
            [
                "workspace:settings.json",
                "app:settings.json",
                "lib:settings.json",
                "mod.rs",
                "mod.rs",
                "mod.rs"
            ]
        );
        // Alone again, a file loses its prefix.
        app.buffers.truncate(2);
        assert_eq!(
            super::super::render::buffer_labels(&app),
            ["workspace:settings.json", "app:settings.json"]
        );
        app.buffers.remove(0);
        assert_eq!(super::super::render::buffer_labels(&app), ["settings.json"]);
        cleanup_session(&app);
    }

    #[test]
    fn the_global_tab_is_labelled_and_violet() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = workspace_app(dir.path());
        let repo = app.workspace.roots()[0].join(".gaviero/settings.json");
        app.open_file(&repo);
        let buf = &mut app.buffers[app.active_buffer];
        buf.settings.as_mut().unwrap().global = true;
        buf.tab_label = Some(GLOBAL_TAB_LABEL.to_string());
        let titles = super::super::render::tab_titles(&app);
        let (label, _, kind) = titles.last().unwrap();
        assert_eq!(label, GLOBAL_TAB_LABEL);
        assert_eq!(*kind, crate::widgets::tabs::TabKind::Global);
        cleanup_session(&app);
    }
}
