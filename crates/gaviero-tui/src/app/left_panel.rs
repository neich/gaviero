use super::*;

pub(super) fn handle_search_action(app: &mut App, action: Action) {
    if app.search_panel.editing {
        match action {
            Action::InsertChar(ch) => {
                app.search_panel.input.insert_char(ch);
                app.run_search_from_input();
            }
            Action::Backspace => {
                app.search_panel.input.backspace();
                app.run_search_from_input();
            }
            Action::Delete => {
                app.search_panel.input.delete();
                app.run_search_from_input();
            }
            Action::DeleteWordBack => {
                app.search_panel.input.delete_word_back();
                app.run_search_from_input();
            }
            Action::CursorLeft => app.search_panel.input.move_left(),
            Action::CursorRight => app.search_panel.input.move_right(),
            Action::WordLeft => app.search_panel.input.move_word_left(),
            Action::WordRight => app.search_panel.input.move_word_right(),
            Action::SelectLeft => app.search_panel.input.select_left(),
            Action::SelectRight => app.search_panel.input.select_right(),
            Action::SelectWordLeft => app.search_panel.input.select_word_left(),
            Action::SelectWordRight => app.search_panel.input.select_word_right(),
            Action::Home => app.search_panel.input.move_home(),
            Action::End => app.search_panel.input.move_end(),
            Action::SelectAll => app.search_panel.input.select_all(),
            Action::Paste => {
                let text = app.get_clipboard();
                if !text.is_empty() {
                    app.search_panel.input.insert_str(&text);
                    app.run_search_from_input();
                }
            }
            Action::CursorDown | Action::Enter => {
                if !app.search_panel.results.is_empty() {
                    app.search_panel.editing = false;
                }
                if action == Action::Enter {
                    app.open_selected_search_result();
                }
            }
            Action::Quit => {
                if !app.search_panel.input.is_empty() {
                    app.search_panel.input.clear();
                    app.search_panel.results.clear();
                    app.search_panel.query.clear();
                    app.search_panel.scroll.reset();
                } else {
                    app.left_panel = LeftPanelMode::FileTree;
                }
            }
            _ => {}
        }
    } else {
        match action {
            Action::CursorDown => {
                let count = app.search_panel.results.len();
                app.search_panel.scroll.move_down(count);
            }
            Action::CursorUp => {
                if app.search_panel.scroll.selected == 0 {
                    app.search_panel.editing = true;
                } else {
                    app.search_panel.scroll.move_up();
                }
            }
            Action::Enter => {
                app.open_selected_search_result();
            }
            Action::InsertChar(ch) => {
                app.search_panel.editing = true;
                app.search_panel.input.insert_char(ch);
                app.run_search_from_input();
            }
            Action::Backspace => {
                app.search_panel.editing = true;
                app.search_panel.input.backspace();
                app.run_search_from_input();
            }
            Action::Quit => {
                app.search_panel.editing = true;
            }
            _ => {}
        }
    }
}

pub(super) fn run_search_from_input(app: &mut App) {
    let roots = app.workspace.roots();
    let excludes: Vec<String> = app.file_tree.exclude_patterns.clone();
    app.search_panel.search_from_input(&roots, &excludes);
}

pub(super) fn open_selected_search_result(app: &mut App) {
    if let Some(result) = app.search_panel.selected_result().cloned() {
        let root = app
            .workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let abs_path = root.join(&result.path);
        if abs_path.exists() {
            app.open_file(&abs_path);
            app.focus = Focus::Editor;
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                let target_line = result.line_number.saturating_sub(1);
                let max_line = buf.line_count().saturating_sub(1);
                buf.cursor.line = target_line.min(max_line);
                buf.cursor.col = 0;
                buf.cursor.anchor = None;
                buf.scroll.top_line = target_line.saturating_sub(10);
            }
        }
    }
}

pub(super) fn handle_file_tree_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown | Action::InsertChar('j') => app.file_tree.move_down(),
        Action::CursorUp | Action::InsertChar('k') => app.file_tree.move_up(),
        Action::Enter => {
            if app.file_tree.selected_is_file() {
                if let Some(path) = app.file_tree.selected_path() {
                    let path = path.to_path_buf();
                    app.open_file(&path);
                    app.focus = Focus::Editor;
                }
            } else {
                app.file_tree.toggle_expand();
            }
        }
        Action::InsertChar('n') => app.start_tree_dialog(TreeDialogKind::NewFile),
        Action::InsertChar('N') => app.start_tree_dialog(TreeDialogKind::NewFolder),
        Action::InsertChar('r') => app.start_tree_dialog(TreeDialogKind::Rename),
        Action::InsertChar('d') | Action::Delete => app.start_tree_dialog(TreeDialogKind::Delete),
        Action::InsertChar('m') => app.start_move(),
        _ => {}
    }
}

pub(super) fn selected_dir(app: &App) -> Option<std::path::PathBuf> {
    let entry = app.file_tree.entries.get(app.file_tree.scroll.selected)?;
    if entry.is_dir {
        Some(entry.path.clone())
    } else {
        entry.path.parent().map(|p| p.to_path_buf())
    }
}

pub(super) fn start_tree_dialog(app: &mut App, kind: TreeDialogKind) {
    let Some(target_dir) = app.selected_dir() else {
        return;
    };

    let mut dialog = TreeDialog::new(kind.clone(), target_dir);

    if matches!(kind, TreeDialogKind::Rename) {
        if let Some(entry) = app.file_tree.entries.get(app.file_tree.scroll.selected) {
            dialog.original_path = Some(entry.path.clone());
            dialog.input = entry.name.clone();
            dialog.cursor = dialog.input.len();
        }
    }

    if matches!(kind, TreeDialogKind::Delete) {
        if let Some(entry) = app.file_tree.entries.get(app.file_tree.scroll.selected) {
            dialog.original_path = Some(entry.path.clone());
        }
    }

    app.tree_dialog = Some(dialog);
}

pub(super) fn start_move(app: &mut App) {
    app.move_state = Some(MoveState::SelectingSource);
}

pub(super) fn handle_move_key(app: &mut App, key: &crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;

    match app.move_state.clone() {
        None => {}
        Some(MoveState::SelectingSource) => match key.code {
            KeyCode::Esc => {
                app.move_state = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.file_tree.move_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.file_tree.move_down();
            }
            KeyCode::Enter => {
                if app.file_tree.selected_is_file() {
                    if let Some(path) = app.file_tree.selected_path() {
                        let src = path.to_path_buf();
                        app.move_state = Some(MoveState::SelectingDest(src));
                    }
                } else {
                    app.file_tree.toggle_expand();
                }
            }
            _ => {}
        },
        Some(MoveState::SelectingDest(src)) => match key.code {
            KeyCode::Esc => {
                app.move_state = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.file_tree.move_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.file_tree.move_down();
            }
            KeyCode::Enter => {
                if let Some(path) = app.file_tree.selected_path() {
                    if path.is_dir() {
                        let dest = path.to_path_buf();
                        app.move_state = Some(MoveState::Confirming(src, dest));
                    } else {
                        app.file_tree.toggle_expand();
                    }
                }
            }
            _ => {}
        },
        Some(MoveState::Confirming(src, dest)) => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.execute_move(src, dest);
            }
            _ => {
                app.move_state = None;
            }
        },
    }
}

pub(super) fn execute_move(app: &mut App, src: std::path::PathBuf, dest_dir: std::path::PathBuf) {
    let file_name = match src.file_name() {
        Some(n) => n.to_os_string(),
        None => {
            app.status_message = Some((
                "Move failed: invalid source path".to_string(),
                std::time::Instant::now(),
            ));
            app.move_state = None;
            return;
        }
    };
    let dest_path = dest_dir.join(&file_name);
    match std::fs::rename(&src, &dest_path) {
        Ok(()) => {
            let name = file_name.to_string_lossy().into_owned();
            app.status_message = Some((
                format!("Moved {} → {}", name, dest_dir.display()),
                std::time::Instant::now(),
            ));
        }
        Err(e) => {
            app.status_message = Some((
                format!("Move failed: {}", e),
                std::time::Instant::now(),
            ));
        }
    }
    app.move_state = None;
}

pub(super) fn render_move_panel_info(app: &App, frame: &mut Frame, tree_area: Rect) {
    use ratatui::style::Modifier;

    let dialog_height: u16 = 2;
    if tree_area.height < dialog_height + 2 {
        return;
    }

    let y = tree_area.bottom() - dialog_height;
    let dialog_area = Rect {
        x: tree_area.x,
        y,
        width: tree_area.width.saturating_sub(1),
        height: dialog_height,
    };

    let bg_style = Style::default().fg(theme::TEXT_BRIGHT).bg(theme::INPUT_BG);
    for row in 0..dialog_area.height {
        for col in 0..dialog_area.width {
            let cx = dialog_area.x + col;
            let cy = dialog_area.y + row;
            if cx < frame.area().right() && cy < frame.area().bottom() {
                frame.buffer_mut()[(cx, cy)]
                    .set_char(' ')
                    .set_style(bg_style);
            }
        }
    }

    let sep_style = Style::default().fg(theme::WARNING).bg(theme::INPUT_BG);
    for col in 0..dialog_area.width {
        let cx = dialog_area.x + col;
        if cx < frame.area().right() {
            frame.buffer_mut()[(cx, y)].set_char('─').set_style(sep_style);
        }
    }

    let input_y = y + 1;
    let label_style = Style::default()
        .fg(theme::WARNING)
        .bg(theme::INPUT_BG)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme::TEXT_DIM).bg(theme::INPUT_BG);

    let mut write_text = |text: &str, style: Style, start_x: &mut u16| {
        for ch in text.chars() {
            if *start_x < dialog_area.x + dialog_area.width {
                frame.buffer_mut()[(*start_x, input_y)]
                    .set_char(ch)
                    .set_style(style);
                *start_x += 1;
            }
        }
    };

    let mut x = dialog_area.x;
    match &app.move_state {
        Some(MoveState::SelectingSource) => {
            write_text("MOVE  ", label_style, &mut x);
            write_text(
                "Navigate, Enter: select file  Esc: cancel",
                dim_style,
                &mut x,
            );
        }
        Some(MoveState::SelectingDest(src)) => {
            let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            write_text(&format!("MOVE {}  ", name), label_style, &mut x);
            write_text(
                "Navigate, Enter: select folder  Esc: cancel",
                dim_style,
                &mut x,
            );
        }
        Some(MoveState::Confirming(src, dest)) => {
            let s = src.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let d = dest.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            write_text(&format!("Move {} → {}? (y/N)", s, d), label_style, &mut x);
        }
        None => {}
    }
}

pub(super) fn handle_dialog_key(app: &mut App, key: &crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    if app
        .tree_dialog
        .as_ref()
        .is_some_and(|d| matches!(d.kind, TreeDialogKind::Delete))
    {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.confirm_tree_dialog();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.tree_dialog = None;
            }
            _ => {}
        }
        return;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Esc => {
            app.tree_dialog = None;
        }
        KeyCode::Enter => {
            app.confirm_tree_dialog();
        }
        KeyCode::Backspace => {
            if let Some(ref mut d) = app.tree_dialog {
                d.backspace();
            }
        }
        KeyCode::Delete => {
            if let Some(ref mut d) = app.tree_dialog {
                d.delete();
            }
        }
        KeyCode::Left => {
            if let Some(ref mut d) = app.tree_dialog {
                d.move_left();
            }
        }
        KeyCode::Right => {
            if let Some(ref mut d) = app.tree_dialog {
                d.move_right();
            }
        }
        KeyCode::Home => {
            if let Some(ref mut d) = app.tree_dialog {
                d.move_home();
            }
        }
        KeyCode::End => {
            if let Some(ref mut d) = app.tree_dialog {
                d.move_end();
            }
        }
        KeyCode::Char('u') if ctrl => {
            if let Some(ref mut d) = app.tree_dialog {
                d.input.clear();
                d.cursor = 0;
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(ref mut d) = app.tree_dialog {
                d.insert_char(c);
            }
        }
        _ => {}
    }
}

pub(super) fn confirm_tree_dialog(app: &mut App) {
    let Some(dialog) = app.tree_dialog.take() else {
        return;
    };

    match dialog.kind {
        TreeDialogKind::NewFile => {
            let name = dialog.input.trim();
            if name.is_empty() {
                return;
            }
            let path = dialog.target_dir.join(name);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, "") {
                tracing::error!("Failed to create file {}: {}", path.display(), e);
            } else {
                app.refresh_file_tree();
                app.select_path_in_tree(&path);
                app.open_file(&path);
                app.focus = Focus::Editor;
            }
        }
        TreeDialogKind::NewFolder => {
            let name = dialog.input.trim();
            if name.is_empty() {
                return;
            }
            let path = dialog.target_dir.join(name);
            if let Err(e) = std::fs::create_dir_all(&path) {
                tracing::error!("Failed to create folder {}: {}", path.display(), e);
            } else {
                app.refresh_file_tree();
                app.select_path_in_tree(&path);
            }
        }
        TreeDialogKind::Rename => {
            let name = dialog.input.trim();
            if name.is_empty() {
                return;
            }
            if let Some(original) = dialog.original_path {
                let new_path = original
                    .parent()
                    .map(|p| p.join(name))
                    .unwrap_or_else(|| std::path::PathBuf::from(name));
                if let Err(e) = std::fs::rename(&original, &new_path) {
                    tracing::error!("Failed to rename: {}", e);
                } else {
                    for buf in &mut app.buffers {
                        if buf.path.as_deref() == Some(&original) {
                            buf.path = Some(new_path.clone());
                        }
                    }
                    app.refresh_file_tree();
                }
            }
        }
        TreeDialogKind::Delete => {
            if let Some(path) = dialog.original_path {
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if let Err(e) = result {
                    tracing::error!("Failed to delete {}: {}", path.display(), e);
                } else {
                    app.buffers.retain(|b| b.path.as_deref() != Some(&path));
                    if app.active_buffer >= app.buffers.len() && !app.buffers.is_empty() {
                        app.active_buffer = app.buffers.len() - 1;
                    }
                    app.refresh_file_tree();
                }
            }
        }
    }
}

pub(super) fn select_path_in_tree(app: &mut App, path: &std::path::Path) {
    for (i, entry) in app.file_tree.entries.iter().enumerate() {
        if entry.path == path {
            app.file_tree.scroll.selected = i;
            return;
        }
    }
}

pub(super) fn refresh_file_tree(app: &mut App) {
    let excludes = parse_exclude_patterns(&app.workspace);
    let git_allow = parse_git_allow_list(&app.workspace);
    let roots: Vec<&std::path::Path> = app.workspace.roots();
    let expanded = app.file_tree.expanded_paths();
    let selected = app.file_tree.scroll.selected;
    app.file_tree = FileTreeState::from_roots(&roots, &excludes, &git_allow);
    app.file_tree.restore_expanded(&expanded);
    app.file_tree.scroll.selected =
        selected.min(app.file_tree.entries.len().saturating_sub(1));
}
