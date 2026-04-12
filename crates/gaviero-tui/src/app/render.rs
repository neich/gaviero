use super::*;

pub(super) fn render(app: &mut App, frame: &mut Frame) {
    let size = frame.area();

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(size);

    let tab_area = main_layout[0];
    let main_area = main_layout[1];
    let status_area = main_layout[2];

    app.layout.tab_area = tab_area;
    app.layout.status_area = status_area;

    app.render_tab_bar(frame, tab_area);

    if let Some(fs_panel) = app.fullscreen_panel {
        app.render_fullscreen(frame, main_area, fs_panel);
        app.render_status_bar(frame, status_area);
        if fs_panel == Focus::Editor {
            app.update_cursor_position(frame, app.layout.editor_area);
        }
        if app.quit_confirm {
            app.render_quit_confirm(frame, size);
        }
        if app.first_run_dialog.is_some() {
            app.render_first_run_dialog(frame, size);
        }
        return;
    }

    app.layout.terminal_area = None;
    let (panels_area, terminal_area) = if app.panel_visible.terminal {
        let term_pct = app.terminal_split_percent;
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(100 - term_pct),
                Constraint::Percentage(term_pct),
            ])
            .split(main_area);
        app.layout.terminal_area = Some(v_split[1]);
        (v_split[0], Some(v_split[1]))
    } else {
        (main_area, None)
    };

    let (eff_ft_w, eff_sp_w) = app.effective_panel_constraints(panels_area.width);

    let mut constraints = Vec::new();
    if app.panel_visible.file_tree {
        constraints.push(Constraint::Length(eff_ft_w));
    }
    constraints.push(Constraint::Min(20));
    if app.panel_visible.side_panel {
        constraints.push(Constraint::Length(eff_sp_w));
    }

    let h_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(panels_area);

    let mut panel_idx = 0;

    app.layout.file_tree_area = None;
    app.layout.left_header_area = None;
    if app.panel_visible.file_tree {
        let full_area = h_layout[panel_idx];
        let left_focused = app.focus == Focus::FileTree;
        let title = app.left_panel_title(false);
        let (header_area, content_area) =
            App::render_panel_header(frame, full_area, title, left_focused, true);
        app.layout.left_header_area = Some(header_area);
        app.layout.file_tree_area = Some(content_area);
        app.render_left_panel_content(frame, content_area, left_focused);

        panel_idx += 1;
    }

    let editor_full_area = h_layout[panel_idx];
    let editor_focused = app.focus == Focus::Editor;
    let editor_title = app
        .buffers
        .get(app.active_buffer)
        .map(|b| b.display_name().to_string())
        .unwrap_or_else(|| "EDITOR".to_string());
    let (_editor_header, editor_content) =
        App::render_panel_header(frame, editor_full_area, &editor_title, editor_focused, false);

    let (actual_editor_area, preview_area) = if app.preview_visible && app.is_current_buffer_markdown()
    {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(editor_content);
        (split[0], Some(split[1]))
    } else {
        (editor_content, None)
    };

    app.layout.editor_area = actual_editor_area;
    app.render_editor(frame, actual_editor_area);

    if let Some(preview_area) = preview_area {
        app.render_markdown_preview(frame, preview_area);
    }
    panel_idx += 1;

    app.layout.side_panel_area = None;
    app.layout.side_header_area = None;
    if app.panel_visible.side_panel && panel_idx < h_layout.len() {
        let full_area = h_layout[panel_idx];
        let side_focused = app.focus == Focus::SidePanel;
        let side_title = app.side_panel_title(false);
        let (side_header, content_area) =
            App::render_panel_header(frame, full_area, side_title, side_focused, true);
        app.layout.side_header_area = Some(side_header);
        app.layout.side_panel_area = Some(content_area);
        app.render_side_panel(frame, content_area);
    }

    if let Some(term_area) = terminal_area {
        app.render_terminal(frame, term_area);
    }

    app.render_status_bar(frame, status_area);
    app.update_cursor_position(frame, app.layout.editor_area);

    if app.quit_confirm {
        app.render_quit_confirm(frame, size);
    }
    if app.first_run_dialog.is_some() {
        app.render_first_run_dialog(frame, size);
    }
}

pub(super) fn render_fullscreen(app: &mut App, frame: &mut Frame, area: Rect, panel: Focus) {
    let title = match panel {
        Focus::FileTree => app.left_panel_title(true).to_string(),
        Focus::Editor => app
            .buffers
            .get(app.active_buffer)
            .map(|b| format!("{} (fullscreen)", b.display_name()))
            .unwrap_or_else(|| "EDITOR (fullscreen)".to_string()),
        Focus::SidePanel => app.side_panel_title(true).to_string(),
        Focus::Terminal => "TERMINAL (fullscreen)".to_string(),
    };

    let (_header, content) = App::render_panel_header(frame, area, &title, true, false);

    match panel {
        Focus::FileTree => {
            app.layout.file_tree_area = Some(content);
            app.render_left_panel_content(frame, content, true);
        }
        Focus::Editor => {
            app.layout.editor_area = content;
            app.render_editor(frame, content);
        }
        Focus::SidePanel => {
            app.layout.side_panel_area = Some(content);
            app.render_side_panel(frame, content);
        }
        Focus::Terminal => {
            app.render_terminal(frame, content);
        }
    }
}

pub(super) fn left_panel_title(app: &App, fullscreen: bool) -> &'static str {
    match (app.left_panel, fullscreen) {
        (LeftPanelMode::FileTree, false) => "EXPLORER",
        (LeftPanelMode::FileTree, true) => "EXPLORER (fullscreen)",
        (LeftPanelMode::Search, false) => "SEARCH",
        (LeftPanelMode::Search, true) => "SEARCH (fullscreen)",
        (LeftPanelMode::Review, false) => "REVIEW",
        (LeftPanelMode::Review, true) => "REVIEW (fullscreen)",
        (LeftPanelMode::Changes, false) => "CHANGES",
        (LeftPanelMode::Changes, true) => "CHANGES (fullscreen)",
    }
}

pub(super) fn side_panel_title(app: &App, fullscreen: bool) -> &'static str {
    match (app.side_panel, fullscreen) {
        (SidePanelMode::AgentChat, false) => "AGENT CHAT",
        (SidePanelMode::AgentChat, true) => "AGENT CHAT (fullscreen)",
        (SidePanelMode::SwarmDashboard, false) => "SWARM",
        (SidePanelMode::SwarmDashboard, true) => "SWARM (fullscreen)",
        (SidePanelMode::GitPanel, false) => "GIT",
        (SidePanelMode::GitPanel, true) => "GIT (fullscreen)",
    }
}

pub(super) fn render_left_panel_content(
    app: &mut App,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
) {
    match app.left_panel {
        LeftPanelMode::FileTree => {
            let move_src = app.current_move_source();
            app.file_tree
                .render(area, frame.buffer_mut(), focused, move_src.as_deref());
            if let Some(ref dialog) = app.tree_dialog {
                app.render_tree_dialog(frame, area, dialog);
            }
            if app.move_state.is_some() {
                app.render_move_panel_info(frame, area);
            }
        }
        LeftPanelMode::Search => {
            app.search_panel.render(area, frame.buffer_mut(), focused);
        }
        LeftPanelMode::Review => {
            app.render_review_file_list(frame, area, focused);
        }
        LeftPanelMode::Changes => {
            app.render_changes_file_list(frame, area, focused);
        }
    }
}

pub(super) fn current_move_source(app: &App) -> Option<std::path::PathBuf> {
    match &app.move_state {
        Some(MoveState::SelectingDest(path)) | Some(MoveState::Confirming(path, _)) => {
            Some(path.clone())
        }
        _ => None,
    }
}

pub(super) fn render_tab_bar(app: &App, frame: &mut Frame, area: Rect) {
    let titles: Vec<(String, bool)> = app
        .buffers
        .iter()
        .map(|b| (b.display_name().to_string(), b.modified))
        .collect();
    let tab_bar = TabBar {
        titles: &titles,
        active: app.active_buffer,
    };
    tab_bar.render(area, frame.buffer_mut());
}

pub(super) fn render_editor(app: &mut App, frame: &mut Frame, area: Rect) {
    if app.batch_review.is_some() {
        app.render_batch_review_diff(frame, area);
        return;
    }

    if app.left_panel == LeftPanelMode::Changes && app.changes_state.is_some() {
        app.render_changes_diff(frame, area);
        return;
    }

    if let Some(ref mut review) = app.diff_review {
        diff_overlay::render_diff_overlay(area, frame.buffer_mut(), review, &app.theme);
        return;
    }

    if app.buffers.is_empty() {
        let msg = " Press Ctrl+\\ to focus file tree, then Enter to open a file";
        let style = app.theme.default_style();
        let y = area.y + area.height / 2;
        if y < area.bottom() {
            for (i, ch) in msg.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, y)].set_char(ch).set_style(style);
                }
            }
        }
        return;
    }

    let editor_area = if app.find_bar_active {
        app.render_find_bar(frame, area);
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        }
    } else {
        area
    };

    let buf = &app.buffers[app.active_buffer];
    let highlight_config = buf
        .lang_name
        .as_ref()
        .and_then(|name| app.highlight_configs.get(name));

    let view = EditorView::new(buf, &app.theme, highlight_config, app.focus == Focus::Editor);
    view.render(editor_area, frame.buffer_mut());
}

pub(super) fn render_find_bar(app: &App, frame: &mut Frame, area: Rect) {
    let bar_y = area.y;
    let buf = frame.buffer_mut();
    let bg = theme::INPUT_BG;
    let fg = theme::TEXT_FG;
    let label_fg = theme::FOCUS_BORDER;

    for x in area.x..area.right() {
        if bar_y < buf.area().bottom() {
            buf[(x, bar_y)]
                .set_char(' ')
                .set_style(Style::default().bg(bg));
        }
    }

    let label = " Find: ";
    let label_style = Style::default()
        .fg(label_fg)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let mut x = area.x;
    for ch in label.chars() {
        if x < area.right() && bar_y < buf.area().bottom() {
            buf[(x, bar_y)].set_char(ch).set_style(label_style);
        }
        x += 1;
    }
    let text_start = x;

    let input_style = Style::default().fg(fg).bg(bg);
    for ch in app.find_input.text.chars() {
        if x >= area.right() {
            break;
        }
        if bar_y < buf.area().bottom() {
            buf[(x, bar_y)].set_char(ch).set_style(input_style);
        }
        x += 1;
    }

    if app.find_bar_active {
        let cursor_x = text_start + app.find_input.cursor as u16;
        if cursor_x < area.right() && bar_y < buf.area().bottom() {
            let cursor_style = Style::default().fg(bg).bg(fg);
            buf[(cursor_x, bar_y)].set_style(cursor_style);
        }
    }

    if let Some(editor_buf) = app.buffers.get(app.active_buffer) {
        let total = editor_buf.search_match_count();
        if total > 0 {
            let current = editor_buf.current_match_index();
            let indicator = format!(" {}/{} ", current, total);
            let ind_style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            let ind_start = area.right().saturating_sub(indicator.len() as u16);
            for (i, ch) in indicator.chars().enumerate() {
                let ix = ind_start + i as u16;
                if ix < area.right() && bar_y < buf.area().bottom() {
                    buf[(ix, bar_y)].set_char(ch).set_style(ind_style);
                }
            }
        } else if !app.find_input.is_empty() {
            let indicator = " No matches ";
            let ind_style = Style::default().fg(theme::ERROR).bg(bg);
            let ind_start = area.right().saturating_sub(indicator.len() as u16);
            for (i, ch) in indicator.chars().enumerate() {
                let ix = ind_start + i as u16;
                if ix < area.right() && bar_y < buf.area().bottom() {
                    buf[(ix, bar_y)].set_char(ch).set_style(ind_style);
                }
            }
        }
    }
}

pub(super) fn render_tree_dialog(_app: &App, frame: &mut Frame, tree_area: Rect, dialog: &TreeDialog) {
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

    let bg_style = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .bg(theme::INPUT_BG);

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

    let sep_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .bg(theme::INPUT_BG);
    for col in 0..dialog_area.width {
        let cx = dialog_area.x + col;
        if cx < frame.area().right() {
            frame.buffer_mut()[(cx, y)].set_char('─').set_style(sep_style);
        }
    }

    let input_y = y + 1;
    let prompt = dialog.prompt();
    let prompt_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .bg(theme::INPUT_BG);

    let mut x = dialog_area.x;
    for ch in prompt.chars() {
        if x < dialog_area.x + dialog_area.width {
            frame.buffer_mut()[(x, input_y)]
                .set_char(ch)
                .set_style(prompt_style);
            x += 1;
        }
    }

    let input_start_x = x;
    let is_delete = matches!(dialog.kind, TreeDialogKind::Delete);
    let display_text = if is_delete {
        dialog
            .original_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(&dialog.input)
    } else {
        &dialog.input
    };
    for ch in display_text.chars() {
        if x < dialog_area.x + dialog_area.width {
            frame.buffer_mut()[(x, input_y)].set_char(ch).set_style(bg_style);
            x += 1;
        }
    }

    if !is_delete {
        let cursor_x = input_start_x + dialog.input[..dialog.cursor].chars().count() as u16;
        if cursor_x < dialog_area.x + dialog_area.width && input_y < frame.area().bottom() {
            frame.set_cursor_position((cursor_x, input_y));
        }
    }
}

pub(super) fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(ref review) = app.diff_review {
        let proposal = &review.proposal;
        let total = proposal.structural_hunks.len();
        let filename = proposal
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        let (status_text, style) = match review.source {
            DiffSource::External => {
                let current = review.current_hunk + 1;
                (
                    format!(
                        " CHANGED  {} modified externally [{}/{}]  |  ]h/[h: navigate  q: dismiss",
                        filename, current, total,
                    ),
                    Style::default().fg(Color::Black).bg(theme::FOCUS_BORDER),
                )
            }
            DiffSource::Acp => {
                let accepted = proposal
                    .structural_hunks
                    .iter()
                    .filter(|h| h.status == gaviero_core::types::HunkStatus::Accepted)
                    .count();
                (
                    format!(
                        " REVIEW  {} [{}/{} accepted] from {}  |  a/r: hunk  A/R: all  ]h/[h: nav  f: finalize  q: dismiss",
                        filename, accepted, total, proposal.source
                    ),
                    Style::default().fg(Color::Black).bg(theme::WARNING),
                )
            }
        };

        for (i, ch) in status_text.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, area.y)].set_char(ch).set_style(style);
            }
        }
        for x in (area.x + status_text.len() as u16)..area.right() {
            frame.buffer_mut()[(x, area.y)].set_style(style);
        }
        return;
    }

    if let Some(ref br) = app.batch_review {
        let n = br.proposals.len();
        let current = br.selected_index + 1;
        let status_text = format!(
            " REVIEW ({} files)  [{}/{}]  |  a: accept  r: reject  f: apply all  Esc: discard  Ctrl+←→: panel",
            n, current, n,
        );
        let style = Style::default().fg(Color::Black).bg(theme::WARNING);
        for (i, ch) in status_text.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, area.y)].set_char(ch).set_style(style);
            }
        }
        for x in (area.x + status_text.len() as u16)..area.right() {
            frame.buffer_mut()[(x, area.y)].set_style(style);
        }
        return;
    }

    let focus_label = match app.focus {
        Focus::Editor => "EDIT",
        Focus::FileTree => match app.left_panel {
            LeftPanelMode::FileTree => "TREE",
            LeftPanelMode::Search => "FIND",
            LeftPanelMode::Review => "REVIEW",
            LeftPanelMode::Changes => "CHANGES",
        },
        Focus::SidePanel => "CHAT",
        Focus::Terminal => "TERM",
    };

    let model = app.chat_state.effective_model().to_string();
    let effort = app.chat_state.effective_effort();
    let (_chars, ctx_pct) = app.chat_state.estimate_context();
    let model_info = format!("{}|{} ctx:{}%", model, effort, ctx_pct);

    let current_buffer = if app.focus == Focus::Editor {
        app.buffers.get(app.active_buffer)
    } else {
        None
    };

    let transient_msg = app.status_message.as_ref().and_then(|(msg, when)| {
        if when.elapsed().as_secs() < 3 {
            Some(msg.as_str())
        } else {
            None
        }
    });

    let context_info = if let Some(msg) = transient_msg {
        msg.to_string()
    } else {
        match app.focus {
            Focus::FileTree => match app.left_panel {
                LeftPanelMode::FileTree => match &app.move_state {
                    Some(MoveState::SelectingSource) => {
                        "MOVE  Navigate and Enter to select source file  Esc: cancel".to_string()
                    }
                    Some(MoveState::SelectingDest(src)) => {
                        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        format!(
                            "MOVE {}  Navigate and Enter to select destination folder  Esc: cancel",
                            name
                        )
                    }
                    Some(MoveState::Confirming(src, dest)) => {
                        let s = src.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let d = dest.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        format!("Move {} → {}? (y/N)  any other key: cancel", s, d)
                    }
                    None => {
                        "n: new file  N: new folder  r: rename  d: delete  m: move  Enter: open  F7: search"
                            .to_string()
                    }
                },
                LeftPanelMode::Review => {
                    let n = app
                        .batch_review
                        .as_ref()
                        .map(|br| br.proposals.len())
                        .unwrap_or(0);
                    format!("REVIEW ({} files)  f: apply all  Esc: discard  ↑↓: navigate", n)
                }
                LeftPanelMode::Search => {
                    if app.search_panel.editing {
                        "Type to search  ↓/Enter: results  Esc: clear/back".to_string()
                    } else {
                        let count = app.search_panel.results.len();
                        format!("{} results  Enter: open  ↑: input  Esc: input  F7: cycle", count)
                    }
                }
                LeftPanelMode::Changes => {
                    let n = app
                        .changes_state
                        .as_ref()
                        .map(|cs| cs.entries.len())
                        .unwrap_or(0);
                    format!(
                        "{} changed files  ↑↓: navigate  Enter: open  R: refresh  Esc: back  F7: cycle",
                        n
                    )
                }
            },
            Focus::SidePanel => {
                let conv_count = app.chat_state.conversations.len();
                let conv_idx = app.chat_state.active_conv + 1;
                format!(
                    "Chat {}/{}  F2: rename  Ctrl+T: new  /help: commands",
                    conv_idx, conv_count
                )
            }
            Focus::Terminal => "Terminal (M4)".to_string(),
            Focus::Editor => String::new(),
        }
    };

    let status = StatusBar {
        buffer: current_buffer,
        theme: &app.theme,
        focus_label,
        model_info: &model_info,
        context_info: &context_info,
    };
    status.render(area, frame.buffer_mut());
}

pub(super) fn render_panel_header(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    show_cycle_arrow: bool,
) -> (Rect, Rect) {
    if area.height < 2 {
        return (Rect::default(), area);
    }

    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let content_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height - 1,
    };

    let (bg, fg) = if focused {
        (theme::FOCUSED_SELECTION_BG, theme::PANEL_HEADER_FOCUSED_FG)
    } else {
        (theme::CODE_BLOCK_BG, theme::PANEL_HEADER_UNFOCUSED_FG)
    };
    let style = Style::default().fg(fg).bg(bg);

    let buf = frame.buffer_mut();
    for col in 0..header_area.width {
        let cx = header_area.x + col;
        if cx < buf.area().right() && header_area.y < buf.area().bottom() {
            buf[(cx, header_area.y)].set_char(' ').set_style(style);
        }
    }

    let padded = format!(" {}", title);
    for (i, ch) in padded.chars().enumerate() {
        let cx = header_area.x + i as u16;
        if cx < header_area.x + header_area.width && cx < buf.area().right() {
            buf[(cx, header_area.y)].set_char(ch).set_style(style);
        }
    }

    if show_cycle_arrow && header_area.width >= 4 {
        let arrow_x = header_area.x + header_area.width - 2;
        if arrow_x < buf.area().right() && header_area.y < buf.area().bottom() {
            let arrow_style = Style::default()
                .fg(if focused {
                    theme::PANEL_HEADER_FOCUSED_FG
                } else {
                    theme::PANEL_HEADER_UNFOCUSED_FG
                })
                .bg(bg);
            buf[(arrow_x, header_area.y)]
                .set_char('▸')
                .set_style(arrow_style);
        }
    }

    (header_area, content_area)
}

pub(super) fn render_terminal(app: &mut App, frame: &mut Frame, area: Rect) {
    if area.height < 2 {
        return;
    }

    let needs_spawn = app
        .terminal_manager
        .active_instance()
        .map_or(true, |i| !i.spawned);
    if needs_spawn {
        app.spawn_active_terminal();
    }

    let focused = app.focus == Focus::Terminal;
    let tab_count = app.terminal_manager.tab_count();
    let active_idx = app.terminal_manager.active_tab_index();

    if tab_count > 1 {
        let buf = frame.buffer_mut();
        let border_fg = if focused {
            theme::FOCUS_BORDER
        } else {
            theme::BORDER_DIM
        };
        let border_style = Style::default().fg(border_fg);
        let active_style = Style::default()
            .fg(theme::NUMERIC_ORANGE)
            .add_modifier(ratatui::style::Modifier::BOLD);

        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() && area.y < buf.area().bottom() {
                buf[(cx, area.y)].set_char('─').set_style(border_style);
            }
        }

        let mut x = area.x + 1;
        for i in 0..tab_count {
            let label = format!(" Term {} ", i + 1);
            let style = if i == active_idx {
                active_style
            } else {
                border_style
            };
            for ch in label.chars() {
                if x < area.x + area.width && x < buf.area().right() && area.y < buf.area().bottom()
                {
                    buf[(x, area.y)].set_char(ch).set_style(style);
                }
                x += 1;
            }
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, area.y)].set_char('│').set_style(border_style);
            }
            x += 1;
        }
    }

    let content_rows = area.height.saturating_sub(1);
    let selection = app.terminal_selection.clone();
    if let Some(inst) = app.terminal_manager.active_instance_mut() {
        inst.resize(content_rows, area.width);
        let screen = inst.screen();
        if tab_count > 1 {
            let content_area = Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: content_rows,
            };
            crate::panels::terminal::render_terminal_screen(
                screen,
                content_area,
                frame.buffer_mut(),
                focused,
                &selection,
            );
        } else {
            crate::panels::terminal::render_terminal_with_border(
                screen,
                area,
                frame.buffer_mut(),
                focused,
                &selection,
            );
        }
    }
}

pub(super) fn render_markdown_preview(app: &App, frame: &mut Frame, area: Rect) {
    use crate::editor::markdown;
    use ratatui::widgets::{Block, Borders};

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(theme::BORDER_DIM))
        .title(" Preview (Ctrl+M) ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(buf) = app.buffers.get(app.active_buffer) {
        let source = buf.text.to_string();
        markdown::render_markdown_preview(
            &source,
            inner,
            frame.buffer_mut(),
            &app.theme,
            app.preview_scroll,
        );
    }
}

pub(super) fn render_side_panel(app: &mut App, frame: &mut Frame, area: Rect) {
    match app.side_panel {
        SidePanelMode::AgentChat => {
            app.chat_state
                .render(area, frame.buffer_mut(), app.focus == Focus::SidePanel, &app.theme);
        }
        SidePanelMode::SwarmDashboard => {
            app.swarm_dashboard
                .render(area, frame.buffer_mut(), app.focus == Focus::SidePanel);
        }
        SidePanelMode::GitPanel => {
            app.git_panel
                .render(area, frame.buffer_mut(), app.focus == Focus::SidePanel, &app.theme);
        }
    }
}

pub(super) fn update_cursor_position(app: &App, frame: &mut Frame, editor_area: Rect) {
    if app.diff_review.is_some() {
        return;
    }
    if app.focus != Focus::Editor || app.buffers.is_empty() {
        return;
    }

    let buf = &app.buffers[app.active_buffer];
    let gutter_w = gutter_width(buf.line_count());
    let cursor_line = buf.cursor.line;
    let cursor_col = buf.cursor.col;
    let top = buf.scroll.top_line;
    let left = buf.scroll.left_col;

    if cursor_line >= top && cursor_line < top + editor_area.height as usize && cursor_col >= left {
        let x = editor_area.x + gutter_w + (cursor_col - left) as u16;
        let y = editor_area.y + (cursor_line - top) as u16;
        if x < editor_area.right() && y < editor_area.bottom() {
            frame.set_cursor_position((x, y));
        }
    }
}

pub(super) fn render_quit_confirm(app: &App, frame: &mut Frame, area: Rect) {
    use gaviero_core::swarm::models::AgentStatus;

    let unsaved: Vec<String> = app
        .buffers
        .iter()
        .filter(|b| b.modified)
        .map(|b| b.display_name().to_string())
        .collect();

    let streaming: Vec<String> = app
        .chat_state
        .conversations
        .iter()
        .filter(|c| c.is_streaming)
        .map(|c| c.title.clone())
        .collect();

    let running_swarm: Vec<String> = app
        .swarm_dashboard
        .agents
        .iter()
        .filter(|a| matches!(a.status, AgentStatus::Running))
        .map(|a| a.id.clone())
        .collect();

    let mut lines: Vec<String> = Vec::new();
    lines.push(String::new());
    lines.push("  Quit gaviero?".to_string());
    lines.push(String::new());
    if !unsaved.is_empty() {
        lines.push("  Unsaved files:".to_string());
        for name in &unsaved {
            lines.push(format!("    • {} [+]", name));
        }
        lines.push(String::new());
    }
    if !streaming.is_empty() {
        lines.push("  Active agents (streaming):".to_string());
        for name in &streaming {
            lines.push(format!("    • {}", name));
        }
        lines.push(String::new());
    }
    if !running_swarm.is_empty() {
        lines.push("  Running swarm agents:".to_string());
        for id in &running_swarm {
            lines.push(format!("    • {}", id));
        }
        lines.push(String::new());
    }
    lines.push("  [y] Quit anyway   [n / Esc] Cancel".to_string());
    lines.push(String::new());

    let dialog_w: u16 = lines
        .iter()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(40)
        .max(40)
        + 2;
    let dialog_h = lines.len() as u16;

    if area.width < dialog_w + 4 || area.height < dialog_h + 2 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;

    let bg_style = Style::default().fg(theme::TEXT_BRIGHT).bg(theme::INPUT_BG);
    let title_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .bg(theme::INPUT_BG)
        .add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(theme::TEXT_DIM).bg(theme::INPUT_BG);

    for row in 0..dialog_h {
        for col in 0..dialog_w {
            let cx = x + col;
            let cy = y + row;
            if cx < frame.area().right() && cy < frame.area().bottom() {
                frame.buffer_mut()[(cx, cy)]
                    .set_char(' ')
                    .set_style(bg_style);
            }
        }
    }

    for col in 0..dialog_w {
        let cx = x + col;
        if cx < frame.area().right() {
            let ch = if col == 0 {
                '┌'
            } else if col == dialog_w - 1 {
                '┐'
            } else {
                '─'
            };
            if y < frame.area().bottom() {
                frame.buffer_mut()[(cx, y)]
                    .set_char(ch)
                    .set_style(title_style);
            }
            let bottom_y = y + dialog_h - 1;
            let ch = if col == 0 {
                '└'
            } else if col == dialog_w - 1 {
                '┘'
            } else {
                '─'
            };
            if bottom_y < frame.area().bottom() {
                frame.buffer_mut()[(cx, bottom_y)]
                    .set_char(ch)
                    .set_style(title_style);
            }
        }
    }
    for row in 1..dialog_h.saturating_sub(1) {
        let cy = y + row;
        if cy < frame.area().bottom() {
            if x < frame.area().right() {
                frame.buffer_mut()[(x, cy)]
                    .set_char('│')
                    .set_style(title_style);
            }
            let right_x = x + dialog_w - 1;
            if right_x < frame.area().right() {
                frame.buffer_mut()[(right_x, cy)]
                    .set_char('│')
                    .set_style(title_style);
            }
        }
    }

    for (i, line) in lines.iter().enumerate() {
        let cy = y + i as u16;
        if cy >= frame.area().bottom() {
            break;
        }
        let is_title = line.trim_start().starts_with("Quit");
        let is_hint = line.contains('[');
        let style = if is_title {
            title_style
        } else if is_hint {
            hint_style
        } else {
            bg_style
        };
        let mut cx = x + 1;
        for ch in line.chars() {
            if cx >= x + dialog_w - 1 {
                break;
            }
            if cx < frame.area().right() {
                frame.buffer_mut()[(cx, cy)].set_char(ch).set_style(style);
            }
            cx += 1;
        }
    }
}

pub(super) fn render_first_run_dialog(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dialog) = &app.first_run_dialog else {
        return;
    };

    let mut lines: Vec<String> = Vec::new();
    lines.push(String::new());
    lines.push("  First-time setup".to_string());
    lines.push("  No .gaviero/ configuration found in this folder.".to_string());
    lines.push(String::new());

    match dialog.step {
        FirstRunStep::AskSettings => {
            lines.push("  Create initial settings.json?".to_string());
            lines.push(String::new());
            lines.push("  [y] Yes   [n / Esc] No".to_string());
        }
        FirstRunStep::AskMemory => {
            lines.push(format!(
                "  settings.json: {}",
                if dialog.create_settings {
                    "will be created"
                } else {
                    "skipped"
                }
            ));
            lines.push(String::new());
            lines.push("  Initialize knowledge graph (memory.db)?".to_string());
            lines.push(String::new());
            lines.push("  [y] Yes   [n / Esc] No".to_string());
        }
    }
    lines.push(String::new());

    let dialog_w: u16 = lines
        .iter()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(50)
        .max(50)
        + 2;
    let dialog_h = lines.len() as u16;

    if area.width < dialog_w + 4 || area.height < dialog_h + 2 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;

    let bg_style = Style::default().fg(theme::TEXT_BRIGHT).bg(theme::INPUT_BG);
    let title_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .bg(theme::INPUT_BG)
        .add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(theme::TEXT_DIM).bg(theme::INPUT_BG);

    for row in 0..dialog_h {
        for col in 0..dialog_w {
            let cx = x + col;
            let cy = y + row;
            if cx < frame.area().right() && cy < frame.area().bottom() {
                frame.buffer_mut()[(cx, cy)]
                    .set_char(' ')
                    .set_style(bg_style);
            }
        }
    }

    for col in 0..dialog_w {
        let cx = x + col;
        if cx < frame.area().right() {
            let ch = if col == 0 {
                '┌'
            } else if col == dialog_w - 1 {
                '┐'
            } else {
                '─'
            };
            if y < frame.area().bottom() {
                frame.buffer_mut()[(cx, y)]
                    .set_char(ch)
                    .set_style(title_style);
            }
            let bottom_y = y + dialog_h - 1;
            let ch = if col == 0 {
                '└'
            } else if col == dialog_w - 1 {
                '┘'
            } else {
                '─'
            };
            if bottom_y < frame.area().bottom() {
                frame.buffer_mut()[(cx, bottom_y)]
                    .set_char(ch)
                    .set_style(title_style);
            }
        }
    }
    for row in 1..dialog_h.saturating_sub(1) {
        let cy = y + row;
        if cy < frame.area().bottom() {
            if x < frame.area().right() {
                frame.buffer_mut()[(x, cy)]
                    .set_char('│')
                    .set_style(title_style);
            }
            let right_x = x + dialog_w - 1;
            if right_x < frame.area().right() {
                frame.buffer_mut()[(right_x, cy)]
                    .set_char('│')
                    .set_style(title_style);
            }
        }
    }

    for (i, line) in lines.iter().enumerate() {
        let cy = y + i as u16;
        if cy >= frame.area().bottom() {
            break;
        }
        let is_title = line.trim_start().starts_with("First-time");
        let is_hint = line.contains('[');
        let style = if is_title {
            title_style
        } else if is_hint {
            hint_style
        } else {
            bg_style
        };
        let mut cx = x + 1;
        for ch in line.chars() {
            if cx >= x + dialog_w - 1 {
                break;
            }
            if cx < frame.area().right() {
                frame.buffer_mut()[(cx, cy)].set_char(ch).set_style(style);
            }
            cx += 1;
        }
    }
}
