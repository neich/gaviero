//! Graph-backed go-to-definition for the editor.
//!
//! A definition query runs on a query-only connection to the folder's code
//! graph (`GraphService::with_query_reader`), never through `flush()`: the
//! graph is whatever the writer last committed, and a modified buffer says so
//! instead of guessing. The click path (`open_file_at`) is shared with the
//! Phase 2 graph panel's links.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use gaviero_core::repo_map::store::DefinitionHit;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::*;

/// The modal definition picker for an ambiguous symbol.
pub struct DefinitionPicker {
    pub ident: String,
    pub hits: Vec<DefinitionHit>,
    pub selected: usize,
    /// Folder root the `hits`' workspace-relative paths are relative to.
    /// Kept so the picker can rebuild an absolute path on a multi-folder
    /// workspace, where `GraphServices` routes by root.
    pub root: PathBuf,
}

/// Workspace-relative `/`-separated key for `abs`, plus the owning folder
/// index. Shared with the graph panel, which keys its witness cache on it.
pub(super) fn rel_for(app: &App, abs: &std::path::Path) -> Option<(usize, String)> {
    let (idx, service) = app.graph_services.containing(abs)?;
    let rel = abs.strip_prefix(service.root()).ok()?;
    let mut parts: Vec<&str> = Vec::new();
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            return None;
        };
        parts.push(name.to_str()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some((idx, parts.join("/")))
}

/// Resolve the definition candidates for `line` (0-based) and `ident`.
fn resolve_targets(
    app: &App,
    abs: &std::path::Path,
    line: usize,
    ident: &str,
) -> anyhow::Result<Vec<DefinitionHit>> {
    let (_, service) = app
        .graph_services
        .containing(abs)
        .ok_or_else(|| anyhow::anyhow!("file is outside the workspace folders"))?;
    let (_, rel) = rel_for(app, abs)
        .ok_or_else(|| anyhow::anyhow!("file is outside its workspace folder"))?;

    let mut hits = service.with_query_reader(|store| {
        let mut hits = store.definitions_at_line(&rel, line as i64)?;
        if hits.len() > 1 {
            let named: Vec<DefinitionHit> =
                hits.iter().filter(|h| h.name == ident).cloned().collect();
            if !named.is_empty() {
                hits = named;
            }
        }
        if hits.is_empty() {
            hits = store.definitions_by_name(ident, &rel)?;
        }
        Ok(hits)
    })?;

    hits.dedup_by(|a, b| a.qualified_name == b.qualified_name);
    hits.sort_by(|a, b| {
        (a.file_path != rel)
            .cmp(&(b.file_path != rel))
            .then_with(|| {
                (dir_of(&a.file_path) != dir_of(&rel))
                    .cmp(&(dir_of(&b.file_path) != dir_of(&rel)))
            })
            .then_with(|| {
                a.line_start
                    .unwrap_or(i64::MAX)
                    .cmp(&b.line_start.unwrap_or(i64::MAX))
            })
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
    });
    Ok(hits)
}

/// The directory part of a `/`-separated relative path.
fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// Where `ident` starts on `line`, matching whole words only.
fn word_start_in_line(line: &str, ident: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i <= chars.len() {
        match gaviero_core::repo_map::word_span_at(line, i) {
            Some((start, end)) => {
                let word: String = chars[start..end].iter().collect();
                if word == ident {
                    return Some(start);
                }
                i = end.max(i + 1);
            }
            None => i += 1,
        }
    }
    None
}

fn status(app: &mut App, text: impl Into<String>) {
    app.status_message = Some((text.into(), std::time::Instant::now()));
}

pub(super) fn goto_definition(app: &mut App) {
    if app.focus != Focus::Editor || app.diff_review.is_some() {
        return;
    }
    let Some(path) = app.buffers.get(app.active_buffer).and_then(|b| b.path.clone()) else {
        status(app, "This buffer has no file path");
        return;
    };
    let Some(ident) = app
        .buffers
        .get(app.active_buffer)
        .and_then(|b| b.identifier_at_cursor())
    else {
        status(app, "No identifier under the cursor");
        return;
    };
    let Some(buf) = app.buffers.get(app.active_buffer) else {
        return;
    };
    let line = buf.cursor.line;
    let modified = buf.modified;

    let hits = match resolve_targets(app, &path, line, &ident) {
        Ok(hits) => hits,
        Err(e) => {
            status(app, format!("Graph lookup failed: {e}"));
            return;
        }
    };
    if hits.is_empty() {
        let mut message = format!("No graph definition for '{ident}'");
        if modified {
            message.push_str(" (save the file to update the graph)");
        }
        status(app, message);
        return;
    }
    let Some((_, service)) = app.graph_services.containing(&path) else {
        status(app, "Graph lookup failed: file is outside the workspace folders");
        return;
    };
    let root = service.root().to_path_buf();
    if hits.len() == 1 {
        let hit = &hits[0];
        let target = root.join(&hit.file_path);
        let (line, name) = (hit.line_start.unwrap_or(0).max(0) as usize, hit.name.clone());
        open_file_at(app, &target, line, 0, Some(&name));
        return;
    }
    app.definition_picker = Some(DefinitionPicker {
        ident,
        hits,
        selected: 0,
        root,
    });
}

pub(super) fn open_file_at(
    app: &mut App,
    path: &std::path::Path,
    line: usize,
    col: usize,
    ident: Option<&str>,
) {
    super::editing::open_file(app, path);
    let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
        return;
    };
    let line = line.min(buf.line_count().saturating_sub(1));
    buf.cursor.line = line;
    buf.cursor.anchor = None;
    let text = buf.line_text(line);
    let col = match ident {
        Some(ident) => {
            word_start_in_line(&text, ident).unwrap_or_else(|| col.min(buf.line_len(line)))
        }
        None => col.min(buf.line_len(line)),
    };
    buf.cursor.col = col;
    buf.scroll.top_line = line.saturating_sub(10);
    app.ensure_editor_cursor_visible();
    app.focus = Focus::Editor;
}

/// Consume every key while the picker is open. Returns false when it is closed.
pub(super) fn handle_picker_key(app: &mut App, key: &KeyEvent) -> bool {
    if app.definition_picker.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(p) = app.definition_picker.as_mut() {
                p.selected = p.selected.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(p) = app.definition_picker.as_mut() {
                p.selected = (p.selected + 1).min(p.hits.len().saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            if let Some(p) = app.definition_picker.take()
                && let Some(hit) = p.hits.get(p.selected)
            {
                let target = p.root.join(&hit.file_path);
                let line = hit.line_start.unwrap_or(0).max(0) as usize;
                let name = hit.name.clone();
                open_file_at(app, &target, line, 0, Some(&name));
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => app.definition_picker = None,
        _ => {}
    }
    true
}

pub(super) fn render_picker(app: &App, frame: &mut Frame, area: Rect) {
    let Some(picker) = app.definition_picker.as_ref() else {
        return;
    };
    let width = (area.width * 6 / 10).max(30).min(area.width);
    let height = (area.height / 2).max(8).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_DIM))
        .title(format!(" DEFINITIONS — {} ", picker.ident));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let rows = inner.height.saturating_sub(1) as usize;
    let offset = picker.selected.saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (i, hit) in picker.hits.iter().enumerate().skip(offset).take(rows) {
        let text = format!(
            "{}:{}  {}  {}",
            hit.file_path,
            hit.line_start.unwrap_or(0).max(0) + 1,
            hit.kind,
            hit.name
        );
        let style = if i == picker.selected {
            Style::default()
                .fg(theme::PANEL_BG)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    if inner.height >= 2 {
        let footer_y = inner.y + inner.height - 1;
        crate::widgets::render_utils::write_text(
            frame.buffer_mut(),
            inner.x,
            footer_y,
            inner.right(),
            "Enter open  Esc cancel",
            Style::default().fg(theme::TOOL_DIM),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use gaviero_core::workspace::Workspace;

    fn graph_app(dir: &std::path::Path) -> App {
        std::fs::create_dir_all(dir.join(".gaviero")).unwrap();
        std::fs::write(dir.join(".gaviero/settings.json"), "{}").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(Workspace::single_folder(dir.to_path_buf()), tx)
    }

    fn cleanup_session(dir: &std::path::Path) {
        if let Some(state_dir) = gaviero_core::session_state::state_dir_for(dir) {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }

    fn write(dir: &std::path::Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn active_rel(app: &App) -> String {
        app.buffers[app.active_buffer]
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn nav_goto_definition_opens_target() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "pub fn helper() {}\npub fn caller() { helper(); }\n",
        );
        gaviero_core::repo_map::graph_builder::build_graph(dir.path(), &[]).unwrap();
        let mut app = graph_app(dir.path());
        super::super::editing::open_file(&mut app, &dir.path().join("src/lib.rs"));
        app.focus = Focus::Editor;
        app.buffers[app.active_buffer].cursor.line = 1;
        app.buffers[app.active_buffer].cursor.col = 22;

        goto_definition(&mut app);

        assert!(app.definition_picker.is_none());
        assert_eq!(app.buffers[app.active_buffer].cursor.line, 0);
        assert_eq!(app.buffers[app.active_buffer].cursor.col, 7);
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_goto_definition_cross_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "pub fn caller() { helper(); }\n");
        write(dir.path(), "src/b.rs", "pub fn helper() {}\n");
        gaviero_core::repo_map::graph_builder::build_graph(dir.path(), &[]).unwrap();
        let mut app = graph_app(dir.path());
        super::super::editing::open_file(&mut app, &dir.path().join("src/a.rs"));
        app.focus = Focus::Editor;
        app.buffers[app.active_buffer].cursor.line = 0;
        app.buffers[app.active_buffer].cursor.col = 18;

        goto_definition(&mut app);

        assert!(active_rel(&app).ends_with("/src/b.rs"), "{}", active_rel(&app));
        assert_eq!(app.buffers[app.active_buffer].cursor.line, 0);
        cleanup_session(dir.path());
    }

    fn picker_fixture(dir: &std::path::Path) -> App {
        write(
            dir,
            "src/a.rs",
            "// helper\npub fn unrelated() {}\n",
        );
        write(dir, "src/b.rs", "pub fn helper() {}\n");
        write(dir, "src/c.rs", "pub fn helper() {}\n");
        gaviero_core::repo_map::graph_builder::build_graph(dir, &[]).unwrap();
        let mut app = graph_app(dir);
        super::super::editing::open_file(&mut app, &dir.join("src/a.rs"));
        app.focus = Focus::Editor;
        app.buffers[app.active_buffer].cursor.line = 0;
        app.buffers[app.active_buffer].cursor.col = 4;
        goto_definition(&mut app);
        app
    }

    #[test]
    fn nav_goto_definition_opens_picker_for_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let app = picker_fixture(dir.path());
        let picker = app.definition_picker.as_ref().expect("picker");
        assert_eq!(picker.hits.len(), 2);
        assert_eq!(picker.hits[0].file_path, "src/b.rs");
        assert_eq!(picker.hits[1].file_path, "src/c.rs");
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_picker_enter_opens_selected() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = picker_fixture(dir.path());
        assert!(handle_picker_key(&mut app, &key(KeyCode::Down)));
        assert_eq!(app.definition_picker.as_ref().unwrap().selected, 1);
        assert!(handle_picker_key(&mut app, &key(KeyCode::Enter)));
        assert!(app.definition_picker.is_none());
        assert!(active_rel(&app).ends_with("/src/c.rs"), "{}", active_rel(&app));
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_goto_definition_reports_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "// zzz_missing\n");
        gaviero_core::repo_map::graph_builder::build_graph(dir.path(), &[]).unwrap();
        let mut app = graph_app(dir.path());
        super::super::editing::open_file(&mut app, &dir.path().join("src/a.rs"));
        app.focus = Focus::Editor;
        app.buffers[app.active_buffer].cursor.col = 4;

        goto_definition(&mut app);

        assert!(app.definition_picker.is_none());
        let message = app
            .status_message
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        assert!(message.contains("No graph definition"), "{message}");
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_goto_definition_needs_editor_focus() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "pub fn helper() {}\n");
        gaviero_core::repo_map::graph_builder::build_graph(dir.path(), &[]).unwrap();
        let mut app = graph_app(dir.path());
        super::super::editing::open_file(&mut app, &dir.path().join("src/a.rs"));
        app.focus = Focus::SidePanel;
        app.status_message = None;

        goto_definition(&mut app);

        assert!(app.status_message.is_none());
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_picker_renders_a_titled_box() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let dir = tempfile::tempdir().unwrap();
        let app = picker_fixture(dir.path());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| render_picker(&app, frame, frame.area()))
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(rendered.contains("DEFINITIONS"), "buffer: {rendered}");
        assert!(rendered.contains("Enter open"), "buffer: {rendered}");
        cleanup_session(dir.path());
    }

    #[test]
    fn nav_rel_for_is_slash_separated() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "pub fn helper() {}\n");
        let app = graph_app(dir.path());
        let abs = dir.path().join("src").join("lib.rs");
        assert_eq!(rel_for(&app, &abs).map(|(_, r)| r).as_deref(), Some("src/lib.rs"));
        assert!(rel_for(&app, &dir.path().join("..").join("outside.rs")).is_none());
        cleanup_session(dir.path());
    }
}
