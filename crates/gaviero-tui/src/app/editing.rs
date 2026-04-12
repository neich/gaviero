use super::*;

pub(super) fn handle_find_bar_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(ch) => {
            app.find_input.insert_char(ch);
            app.update_find_highlight();
        }
        Action::Backspace => {
            app.find_input.backspace();
            app.update_find_highlight();
        }
        Action::Delete => {
            app.find_input.delete();
            app.update_find_highlight();
        }
        Action::DeleteWordBack => {
            app.find_input.delete_word_back();
            app.update_find_highlight();
        }
        Action::CursorLeft => app.find_input.move_left(),
        Action::CursorRight => app.find_input.move_right(),
        Action::WordLeft => app.find_input.move_word_left(),
        Action::WordRight => app.find_input.move_word_right(),
        Action::SelectLeft => app.find_input.select_left(),
        Action::SelectRight => app.find_input.select_right(),
        Action::SelectWordLeft => app.find_input.select_word_left(),
        Action::SelectWordRight => app.find_input.select_word_right(),
        Action::Home => app.find_input.move_home(),
        Action::End => app.find_input.move_end(),
        Action::SelectAll => app.find_input.select_all(),
        Action::Paste => {
            let text = app.get_clipboard();
            if !text.is_empty() {
                app.find_input.insert_str(&text);
                app.update_find_highlight();
            }
        }
        Action::Enter | Action::CursorDown => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.find_next_match();
            }
            app.ensure_editor_cursor_visible();
        }
        Action::CursorUp => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.find_prev_match();
            }
            app.ensure_editor_cursor_visible();
        }
        Action::Quit | Action::FindInBuffer => {
            app.find_bar_active = false;
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.set_search_highlight(None);
            }
        }
        _ => {}
    }
}

pub(super) fn update_find_highlight(app: &mut App) {
    let query = app.find_input.text.clone();
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        if query.is_empty() {
            buf.set_search_highlight(None);
        } else {
            buf.set_search_highlight(Some(query));
            buf.find_next_match();
        }
    }
    app.ensure_editor_cursor_visible();
}

pub(super) fn ensure_editor_cursor_visible(app: &mut App) {
    let area = app.layout.editor_area;
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        let line_count = buf.line_count();
        let gutter_w = gutter_width(line_count) as usize;
        let vp_h = area.height as usize;
        let vp_w = (area.width as usize).saturating_sub(gutter_w);
        buf.ensure_cursor_visible(vp_h, vp_w);
    }
}

pub(super) fn handle_editor_action(app: &mut App, action: Action) {
    match action {
        Action::Copy => {
            app.clipboard_copy();
            return;
        }
        Action::Cut => {
            app.clipboard_cut();
        }
        Action::Paste => {
            app.clipboard_paste();
        }
        Action::SelectAll => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.select_all();
            }
        }
        Action::FormatBuffer => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                let msg = if buf.selection_range().is_some() {
                    buf.format_selection()
                } else {
                    buf.format()
                };
                app.status_message = Some((msg, std::time::Instant::now()));
            }
        }
        Action::CycleFormatLevel => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                let msg = buf.cycle_format_level();
                app.status_message = Some((msg, std::time::Instant::now()));
            }
        }
        _ => {
            let area = app.layout.editor_area;
            let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
                return;
            };
            let line_count = buf.line_count();
            let gutter_w = gutter_width(line_count) as usize;
            let vp_h = area.height as usize;
            let vp_w = (area.width as usize).saturating_sub(gutter_w);
            match action {
                Action::Tab => buf.insert_tab(),
                Action::InsertChar(ch) => buf.insert_char(ch),
                Action::Backspace => {
                    buf.backspace();
                }
                Action::Delete => {
                    buf.delete();
                }
                Action::Enter => buf.insert_newline(),
                Action::CursorUp => buf.move_cursor_up(),
                Action::CursorDown => buf.move_cursor_down(),
                Action::CursorLeft => buf.move_cursor_left(),
                Action::CursorRight => buf.move_cursor_right(),
                Action::WordLeft => buf.move_word_left(),
                Action::WordRight => buf.move_word_right(),
                Action::SelectLeft => buf.select_left(),
                Action::SelectRight => buf.select_right(),
                Action::SelectUp => buf.select_up(),
                Action::SelectDown => buf.select_down(),
                Action::SelectWordLeft => buf.select_word_left(),
                Action::SelectWordRight => buf.select_word_right(),
                Action::PageUp => buf.page_up(vp_h),
                Action::PageDown => buf.page_down(vp_h),
                Action::Home => buf.move_cursor_home(),
                Action::End => buf.move_cursor_end(),
                Action::Undo => {
                    buf.undo();
                }
                Action::Redo => {
                    buf.redo();
                }
                Action::DeleteLine => buf.delete_line(),
                Action::DuplicateLine => buf.duplicate_line(),
                Action::MoveLineUp => buf.move_line_up(),
                Action::MoveLineDown => buf.move_line_down(),
                Action::GoToLineEnd => buf.move_cursor_end(),
                Action::DeleteToLineEnd => buf.delete_to_line_end(),
                Action::DeleteWordBack => buf.delete_word_back(),
                _ => {}
            }
            buf.ensure_cursor_visible(vp_h, vp_w);
        }
    }

    let area = app.layout.editor_area;
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        let line_count = buf.line_count();
        let gutter_w = gutter_width(line_count) as usize;
        let vp_h = area.height as usize;
        let vp_w = (area.width as usize).saturating_sub(gutter_w);
        buf.ensure_cursor_visible(vp_h, vp_w);
    }
}

pub(super) fn clipboard_copy(app: &mut App) {
    let Some(buf) = app.buffers.get(app.active_buffer) else {
        return;
    };
    let text = buf.selected_text();
    if text.is_empty() {
        return;
    }
    let n = text.chars().count();
    let suffix = if n == 1 { "" } else { "s" };
    let msg = match app.set_clipboard(&text) {
        ClipboardResult::System => format!("Copied {} char{}", n, suffix),
        ClipboardResult::Osc52 => format!("Copied {} char{} (via terminal)", n, suffix),
        ClipboardResult::Unavailable => format!(
            "Copied {} char{} (internal only — terminal does not support OSC 52)",
            n, suffix
        ),
    };
    app.status_message = Some((msg, std::time::Instant::now()));
}

pub(super) fn clipboard_cut(app: &mut App) {
    let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
        return;
    };
    let text = buf.delete_selection();
    if text.is_empty() {
        return;
    }
    let n = text.chars().count();
    let suffix = if n == 1 { "" } else { "s" };
    let msg = match app.set_clipboard(&text) {
        ClipboardResult::System => format!("Cut {} char{}", n, suffix),
        ClipboardResult::Osc52 => format!("Cut {} char{} (via terminal)", n, suffix),
        ClipboardResult::Unavailable => format!(
            "Cut {} char{} (internal only — terminal does not support OSC 52)",
            n, suffix
        ),
    };
    app.status_message = Some((msg, std::time::Instant::now()));
}

pub(super) fn clipboard_paste(app: &mut App) {
    let text = app.get_clipboard();
    if text.is_empty() {
        return;
    }
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        buf.paste_text(&text);
    }
}

pub(super) fn set_clipboard(app: &mut App, text: &str) -> ClipboardResult {
    app.internal_clipboard = text.to_string();
    if let Some(cb) = &mut app.clipboard {
        if cb.set_text(text).is_ok() {
            return ClipboardResult::System;
        }
        tracing::warn!("arboard set_text failed, falling back to OSC 52");
    }
    if osc52_copy(text) {
        ClipboardResult::Osc52
    } else {
        ClipboardResult::Unavailable
    }
}

pub(super) fn get_clipboard(app: &mut App) -> String {
    if let Some(cb) = &mut app.clipboard {
        if let Ok(text) = cb.get_text() {
            return text;
        }
    }
    app.internal_clipboard.clone()
}

pub(super) fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    let col = mouse.column;
    let row = mouse.row;

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(ref mut review) = app.diff_review {
                if review.is_interactive()
                    && app.layout.editor_area.contains((col, row).into())
                    && col < app.layout.editor_area.x + DIFF_GUTTER_WIDTH
                {
                    let relative_row = (row - app.layout.editor_area.y) as usize;
                    if let Some(hunk_idx) = diff_overlay::hunk_at_row(review, relative_row) {
                        let current = review
                            .proposal
                            .structural_hunks
                            .get(hunk_idx)
                            .map(|h| h.status.clone());
                        match current {
                            Some(gaviero_core::types::HunkStatus::Accepted) => {
                                review.reject_hunk(hunk_idx);
                            }
                            _ => {
                                review.accept_hunk(hunk_idx);
                            }
                        }
                        return;
                    }
                }
            }

            if let Some(hdr) = app.layout.left_header_area {
                if hdr.contains((col, row).into()) {
                    let arrow_zone = hdr.x + hdr.width.saturating_sub(3);
                    if col >= arrow_zone {
                        app.focus = Focus::FileTree;
                        app.left_panel = match app.left_panel {
                            LeftPanelMode::FileTree => LeftPanelMode::Search,
                            LeftPanelMode::Search => LeftPanelMode::Review,
                            LeftPanelMode::Review => LeftPanelMode::Changes,
                            LeftPanelMode::Changes => LeftPanelMode::FileTree,
                        };
                        return;
                    }
                    app.focus = Focus::FileTree;
                    return;
                }
            }

            if let Some(hdr) = app.layout.side_header_area {
                if hdr.contains((col, row).into()) {
                    let arrow_zone = hdr.x + hdr.width.saturating_sub(3);
                    if col >= arrow_zone {
                        app.focus = Focus::SidePanel;
                        app.side_panel = match app.side_panel {
                            SidePanelMode::AgentChat => SidePanelMode::SwarmDashboard,
                            SidePanelMode::SwarmDashboard => SidePanelMode::GitPanel,
                            SidePanelMode::GitPanel => SidePanelMode::AgentChat,
                        };
                        return;
                    }
                    app.focus = Focus::SidePanel;
                    return;
                }
            }

            if let Some(area) = app.layout.file_tree_area {
                if area.contains((col, row).into()) {
                    app.focus = Focus::FileTree;

                    let scrollbar_x = area.x + area.width.saturating_sub(1);
                    if col == scrollbar_x {
                        app.scrollbar_dragging = Some(ScrollbarTarget::LeftPanel);
                        app.scroll_panel_to_row(ScrollbarTarget::LeftPanel, row);
                        return;
                    }

                    let relative_row = (row - area.y) as usize;

                    match app.left_panel {
                        LeftPanelMode::FileTree => {
                            app.file_tree.click_row(relative_row);
                            let is_file = app.file_tree.selected_is_file();
                            if is_file {
                                if let Some(path) = app.file_tree.selected_path() {
                                    let path = path.to_path_buf();
                                    app.open_file(&path);
                                    app.focus = Focus::Editor;
                                }
                            } else {
                                app.file_tree.toggle_expand();
                            }
                        }
                        LeftPanelMode::Search => {
                            if relative_row == 0 {
                                app.search_panel.editing = true;
                                return;
                            }
                            let idx = app
                                .search_panel
                                .scroll
                                .offset
                                + relative_row.saturating_sub(2);
                            if idx < app.search_panel.results.len() {
                                app.search_panel.scroll.selected = idx;
                                let result = app.search_panel.results[idx].clone();
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
                                        let target = result.line_number.saturating_sub(1);
                                        let max = buf.line_count().saturating_sub(1);
                                        buf.cursor.line = target.min(max);
                                        buf.cursor.col = 0;
                                        buf.cursor.anchor = None;
                                        buf.scroll.top_line = target.saturating_sub(10);
                                    }
                                }
                            }
                        }
                        LeftPanelMode::Review => {
                            if let Some(ref mut br) = app.batch_review {
                                let idx = br.scroll_offset + relative_row;
                                if idx < br.proposals.len() {
                                    br.selected_index = idx;
                                    br.diff_scroll = 0;
                                }
                            }
                        }
                        LeftPanelMode::Changes => {
                            if let Some(ref mut cs) = app.changes_state {
                                let idx = cs.scroll_offset + relative_row;
                                if idx < cs.entries.len() {
                                    cs.selected_index = idx;
                                    cs.diff_scroll = 0;
                                }
                            }
                        }
                    }
                    return;
                }
            }
            if app.layout.editor_area.contains((col, row).into()) {
                app.focus = Focus::Editor;

                let scrollbar_x =
                    app.layout.editor_area.x + app.layout.editor_area.width.saturating_sub(1);
                if col == scrollbar_x {
                    app.scrollbar_dragging = Some(ScrollbarTarget::Editor);
                    app.scroll_panel_to_row(ScrollbarTarget::Editor, row);
                    return;
                }

                if app.diff_review.is_none() {
                    let is_double_click = app
                        .last_click
                        .map(|(lc, lr, lt)| lc == col && lr == row && lt.elapsed().as_millis() < 400)
                        .unwrap_or(false);

                    if is_double_click {
                        app.last_click = None;
                        if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                            buf.select_word_at_cursor();
                        }
                    } else {
                        app.last_click = Some((col, row, std::time::Instant::now()));
                        app.set_cursor_from_mouse(col, row);
                        if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                            buf.cursor.anchor = None;
                        }
                        app.mouse_dragging = true;
                    }
                }
                return;
            }
            if let Some(area) = app.layout.side_panel_area {
                if area.contains((col, row).into()) {
                    app.focus = Focus::SidePanel;

                    if app.side_panel == SidePanelMode::SwarmDashboard {
                        use crate::panels::swarm_dashboard::DashboardFocus;
                        let dash = &mut app.swarm_dashboard;
                        let pos = ratatui::layout::Position::new(col, row);
                        if dash.table_rect.contains(pos) {
                            dash.focus = DashboardFocus::Table;
                            let clicked_row = (row - dash.table_rect.y) as usize;
                            let idx = dash.scroll.offset + clicked_row;
                            if idx < dash.agents.len() {
                                dash.scroll.selected = idx;
                                dash.detail_scroll = 0;
                                dash.detail_auto_scroll = true;
                            }
                        } else if dash.detail_rect.contains(pos) {
                            dash.focus = DashboardFocus::Detail;
                        }
                        return;
                    }

                    if app.side_panel == SidePanelMode::AgentChat && row == area.y {
                        let tab_area_x = area.x + 1;
                        if let Some(idx) = app.chat_state.conv_tab_at_x(col, tab_area_x) {
                            if idx == app.chat_state.conversations.len() {
                                app.chat_state.new_conversation();
                            } else if idx != app.chat_state.active_conv {
                                app.chat_state.switch_conversation(idx);
                            }
                            app.needs_full_redraw = true;
                            return;
                        }
                    }

                    let scrollbar_x = area.x + area.width.saturating_sub(1);
                    if col == scrollbar_x {
                        app.scrollbar_dragging = Some(ScrollbarTarget::Chat);
                        app.scroll_panel_to_row(ScrollbarTarget::Chat, row);
                        return;
                    }

                    if app.side_panel == SidePanelMode::AgentChat {
                        if let Some((line, ci)) = app.chat_state.screen_to_text_pos(col, row) {
                            app.chat_state.start_text_selection(line, ci);
                        } else {
                            app.chat_state.clear_text_selection();
                        }
                    }
                    return;
                }
            }
            if let Some(area) = app.layout.terminal_area {
                if area.contains((col, row).into()) {
                    app.focus = Focus::Terminal;
                    app.terminal_selection.clear();
                    let content_y_start = area.y + 1;
                    if row >= content_y_start && row < area.y + area.height {
                        let vt_row = row - content_y_start;
                        let vt_col = col.saturating_sub(area.x);
                        app.terminal_selection.start(vt_row, vt_col);
                    }
                    return;
                }
            }
            if app.layout.tab_area.contains((col, row).into()) {
                let titles: Vec<(String, bool)> = app
                    .buffers
                    .iter()
                    .map(|b| (b.display_name().to_string(), b.modified))
                    .collect();
                let tab_bar = TabBar {
                    titles: &titles,
                    active: app.active_buffer,
                };
                if let Some(idx) = tab_bar.tab_at_x(col, app.layout.tab_area.x) {
                    if idx < app.buffers.len() && idx != app.active_buffer {
                        app.active_buffer = idx;
                        app.focus = Focus::Editor;
                        app.needs_full_redraw = true;
                    }
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if let Some(area) = app.layout.file_tree_area {
                if area.contains((col, row).into()) {
                    match app.left_panel {
                        LeftPanelMode::FileTree => app.file_tree.scroll_up(3),
                        LeftPanelMode::Search => app.search_panel.scroll.scroll_up(3),
                        LeftPanelMode::Review => {
                            if let Some(ref mut br) = app.batch_review {
                                br.scroll_offset = br.scroll_offset.saturating_sub(3);
                            }
                        }
                        LeftPanelMode::Changes => {
                            if let Some(ref mut cs) = app.changes_state {
                                cs.scroll_offset = cs.scroll_offset.saturating_sub(3);
                            }
                        }
                    }
                }
            }
            if let Some(area) = app.layout.side_panel_area {
                if area.contains((col, row).into()) {
                    match app.side_panel {
                        SidePanelMode::SwarmDashboard => {
                            let dash = &mut app.swarm_dashboard;
                            let pos = ratatui::layout::Position::new(col, row);
                            if dash.table_rect.contains(pos) {
                                dash.scroll.scroll_up(1);
                            } else if dash.detail_rect.contains(pos) {
                                dash.detail_auto_scroll = false;
                                dash.detail_scroll = dash.detail_scroll.saturating_sub(3);
                            }
                        }
                        _ => {
                            app.chat_state.scroll_offset =
                                app.chat_state.scroll_offset.saturating_sub(3);
                        }
                    }
                }
            }
            if app.layout.editor_area.contains((col, row).into()) {
                if let Some(ref mut br) = app.batch_review {
                    br.diff_scroll = br.diff_scroll.saturating_sub(3);
                } else if let Some(ref mut cs) = app.changes_state {
                    if app.left_panel == LeftPanelMode::Changes {
                        cs.diff_scroll = cs.diff_scroll.saturating_sub(3);
                    }
                } else if let Some(ref mut review) = app.diff_review {
                    review.scroll_top = review.scroll_top.saturating_sub(3);
                } else if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    buf.scroll.top_line = buf.scroll.top_line.saturating_sub(3);
                }
            }
            if let Some(area) = app.layout.terminal_area {
                if area.contains((col, row).into()) {
                    if let Some(inst) = app.terminal_manager.active_instance_mut() {
                        let current = inst.screen().scrollback();
                        inst.screen_mut().set_scrollback(current + 3);
                    }
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(area) = app.layout.file_tree_area {
                if area.contains((col, row).into()) {
                    match app.left_panel {
                        LeftPanelMode::FileTree => app.file_tree.scroll_down(3),
                        LeftPanelMode::Search => {
                            let count = app.search_panel.results.len();
                            app.search_panel.scroll.scroll_down(3, count);
                        }
                        LeftPanelMode::Review => {
                            if let Some(ref mut br) = app.batch_review {
                                let max = br.proposals.len().saturating_sub(1);
                                br.scroll_offset = (br.scroll_offset + 3).min(max);
                            }
                        }
                        LeftPanelMode::Changes => {
                            if let Some(ref mut cs) = app.changes_state {
                                let max = cs.entries.len().saturating_sub(1);
                                cs.scroll_offset = (cs.scroll_offset + 3).min(max);
                            }
                        }
                    }
                }
            }
            if let Some(area) = app.layout.side_panel_area {
                if area.contains((col, row).into()) {
                    match app.side_panel {
                        SidePanelMode::SwarmDashboard => {
                            let dash = &mut app.swarm_dashboard;
                            let pos = ratatui::layout::Position::new(col, row);
                            if dash.table_rect.contains(pos) {
                                dash.scroll.scroll_down(1, dash.agents.len());
                            } else if dash.detail_rect.contains(pos) {
                                if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                                    let w = dash.detail_rect.width.saturating_sub(1) as usize;
                                    let total = crate::panels::swarm_dashboard::count_display_lines(
                                        &agent.activity,
                                        w,
                                    );
                                    dash.detail_scroll =
                                        (dash.detail_scroll + 3).min(total.saturating_sub(1));
                                }
                            }
                        }
                        _ => {
                            app.chat_state.scroll_offset =
                                app.chat_state.scroll_offset.saturating_add(3);
                        }
                    }
                }
            }
            if app.layout.editor_area.contains((col, row).into()) {
                if let Some(ref mut br) = app.batch_review {
                    br.diff_scroll += 3;
                } else if app.left_panel == LeftPanelMode::Changes {
                    if let Some(ref mut cs) = app.changes_state {
                        cs.diff_scroll += 3;
                    }
                } else if let Some(ref mut review) = app.diff_review {
                    review.scroll_top += 3;
                } else if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    let max = buf.line_count().saturating_sub(1);
                    buf.scroll.top_line = (buf.scroll.top_line + 3).min(max);
                }
            }
            if let Some(area) = app.layout.terminal_area {
                if area.contains((col, row).into()) {
                    if let Some(inst) = app.terminal_manager.active_instance_mut() {
                        let current = inst.screen().scrollback();
                        inst.screen_mut().set_scrollback(current.saturating_sub(3));
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(target) = app.scrollbar_dragging {
                app.scroll_panel_to_row(target, row);
                return;
            }
            if app.chat_state.chat_dragging {
                if let Some((line, ci)) = app.chat_state.screen_to_text_pos(col, row) {
                    app.chat_state.extend_text_selection(line, ci);
                }
                return;
            }
            if app.terminal_selection.dragging {
                if let Some(area) = app.layout.terminal_area {
                    let content_y_start = area.y + 1;
                    let content_height = area.height.saturating_sub(1);
                    let last_content_row = area.y + area.height - 1;

                    if row < content_y_start {
                        if let Some(inst) = app.terminal_manager.active_instance_mut() {
                            let current = inst.screen().scrollback();
                            inst.screen_mut().set_scrollback(current + 1);
                        }
                        let vt_col = col.saturating_sub(area.x);
                        app.terminal_selection.extend(0, vt_col);
                    } else if row > last_content_row {
                        if let Some(inst) = app.terminal_manager.active_instance_mut() {
                            let current = inst.screen().scrollback();
                            inst.screen_mut().set_scrollback(current.saturating_sub(1));
                        }
                        let vt_col = col.saturating_sub(area.x);
                        app.terminal_selection
                            .extend(content_height.saturating_sub(1), vt_col);
                    } else {
                        let vt_row = row - content_y_start;
                        let vt_col = col.saturating_sub(area.x);
                        app.terminal_selection.extend(vt_row, vt_col);
                    }
                }
                return;
            }
            if app.mouse_dragging && app.layout.editor_area.contains((col, row).into()) {
                if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    if buf.cursor.anchor.is_none() {
                        buf.cursor.anchor = Some((buf.cursor.line, buf.cursor.col));
                    }
                }
                app.set_cursor_from_mouse(col, row);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if app.terminal_selection.dragging {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    if let Some(text) = app.terminal_selection.extract_text(inst.screen()) {
                        app.set_clipboard(&text);
                    }
                }
                app.terminal_selection.dragging = false;
            }
            if app.chat_state.chat_dragging {
                if let Some(text) = app.chat_state.selected_chat_text() {
                    app.set_clipboard(&text);
                }
                app.chat_state.chat_dragging = false;
            }
            if app.mouse_dragging {
                if let Some(buf) = app.buffers.get(app.active_buffer) {
                    let text = buf.selected_text();
                    if !text.is_empty() {
                        app.set_clipboard(&text);
                    }
                }
            }
            app.mouse_dragging = false;
            app.scrollbar_dragging = None;
        }
        _ => {}
    }
}

pub(super) fn scroll_panel_to_row(app: &mut App, target: ScrollbarTarget, row: u16) {
    match target {
        ScrollbarTarget::Editor => {
            let area = app.layout.editor_area;
            let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
                return;
            };
            let track_height = area.height as usize;
            if track_height == 0 {
                return;
            }
            let total = buf.line_count();
            if total <= track_height {
                return;
            }
            let max_scroll = total.saturating_sub(track_height);
            let row_in_track = row.saturating_sub(area.y) as usize;
            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
            buf.scroll.top_line =
                (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
        }
        ScrollbarTarget::Chat => {
            let Some(area) = app.layout.side_panel_area else {
                return;
            };
            let conv_height = area.height.saturating_sub(4) as usize;
            if conv_height == 0 {
                return;
            }
            let total = app.chat_state.scroll_offset + conv_height + 10;
            if total <= conv_height {
                return;
            }
            let max_scroll = total.saturating_sub(conv_height);
            let row_in_track = row.saturating_sub(area.y + 1) as usize;
            let fraction = row_in_track as f64 / conv_height.saturating_sub(1).max(1) as f64;
            app.chat_state.scroll_offset =
                (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
        }
        ScrollbarTarget::LeftPanel => {
            let Some(area) = app.layout.file_tree_area else {
                return;
            };
            let track_height = area.height as usize;
            if track_height == 0 {
                return;
            }
            match app.left_panel {
                LeftPanelMode::FileTree => {
                    let total = app.file_tree.entries.len();
                    if total <= track_height {
                        return;
                    }
                    let max_scroll = total.saturating_sub(track_height);
                    let row_in_track = row.saturating_sub(area.y) as usize;
                    let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                    app.file_tree.scroll.offset =
                        (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                }
                LeftPanelMode::Search => {
                    let total = app.search_panel.results.len();
                    let viewport = track_height.saturating_sub(2);
                    if total <= viewport {
                        return;
                    }
                    let max_scroll = total.saturating_sub(viewport);
                    let row_in_track = row.saturating_sub(area.y) as usize;
                    let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                    app.search_panel.scroll.offset =
                        (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                }
                LeftPanelMode::Review => {
                    if let Some(ref mut br) = app.batch_review {
                        let total = br.proposals.len();
                        if total <= track_height {
                            return;
                        }
                        let max_scroll = total.saturating_sub(track_height);
                        let row_in_track = row.saturating_sub(area.y) as usize;
                        let fraction =
                            row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                        br.scroll_offset =
                            (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                    }
                }
                LeftPanelMode::Changes => {
                    if let Some(ref mut cs) = app.changes_state {
                        let total = cs.entries.len();
                        if total <= track_height {
                            return;
                        }
                        let max_scroll = total.saturating_sub(track_height);
                        let row_in_track = row.saturating_sub(area.y) as usize;
                        let fraction =
                            row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                        cs.scroll_offset =
                            (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                    }
                }
            }
        }
    }
}

pub(super) fn set_cursor_from_mouse(app: &mut App, col: u16, row: u16) {
    let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
        return;
    };
    let area = app.layout.editor_area;
    let gutter_w = gutter_width(buf.line_count());
    if col >= area.x + gutter_w {
        let visual_col = (col - area.x - gutter_w) as usize + buf.scroll.left_col;
        let click_line = (row - area.y) as usize + buf.scroll.top_line;
        let max_line = buf.line_count().saturating_sub(1);
        buf.cursor.line = click_line.min(max_line);
        let char_col = buf.visual_to_char_col(buf.cursor.line, visual_col);
        let line_len = buf.line_len(buf.cursor.line);
        buf.cursor.col = char_col.min(line_len);
    }
}

pub(super) fn handle_paste(app: &mut App, text: &str) {
    if app.diff_review.is_some() {
        return;
    }
    match app.focus {
        Focus::Editor => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.paste_text(text);
                let area = app.layout.editor_area;
                let line_count = buf.line_count();
                let gutter_w = gutter_width(line_count) as usize;
                let vp_h = area.height as usize;
                let vp_w = (area.width as usize).saturating_sub(gutter_w);
                buf.ensure_cursor_visible(vp_h, vp_w);
            }
        }
        Focus::SidePanel => {
            app.chat_state.insert_str(text);
        }
        _ => {}
    }
}

pub(super) fn search_selected_in_workspace(app: &mut App) {
    let query = if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        let selected = buf.selected_text();
        if selected.is_empty() {
            buf.select_word_at_cursor()
        } else {
            selected
        }
    } else {
        String::new()
    };

    if !app.search_panel.results.is_empty()
        && (query.trim().is_empty() || query.trim() == app.search_panel.query)
    {
        app.goto_next_search_result();
        return;
    }

    if query.trim().is_empty() {
        return;
    }

    app.search_panel.input.clear();
    app.search_panel.input.insert_str(&query);
    app.search_panel.editing = false;

    let roots = app.workspace.roots();
    let excludes: Vec<String> = app.file_tree.exclude_patterns.clone();
    app.search_panel.search(&query, &roots, &excludes);

    app.left_panel = LeftPanelMode::Search;
    if !app.panel_visible.file_tree {
        app.panel_visible.file_tree = true;
    }

    let count = app.search_panel.results.len();
    if count > 0 {
        app.goto_next_search_result();
    }
    app.status_message = Some((
        format!("Found {} results for '{}'", count, query),
        std::time::Instant::now(),
    ));
}

pub(super) fn goto_next_search_result(app: &mut App) {
    if app.search_panel.results.is_empty() {
        return;
    }

    let count = app.search_panel.results.len();
    app.search_panel.scroll.selected = (app.search_panel.scroll.selected + 1) % count;
    app.search_panel.scroll.ensure_visible();

    let result = app.search_panel.results[app.search_panel.scroll.selected].clone();
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
            let target = result.line_number.saturating_sub(1);
            let max = buf.line_count().saturating_sub(1);
            buf.cursor.line = target.min(max);
            buf.cursor.col = 0;
            buf.cursor.anchor = None;
            buf.scroll.top_line = target.saturating_sub(10);
            buf.set_search_highlight(Some(app.search_panel.query.clone()));
        }
    }

    let idx = app.search_panel.scroll.selected + 1;
    let total = app.search_panel.results.len();
    app.status_message = Some((
        format!(
            "Result {}/{}: {}:{}",
            idx,
            total,
            result.path.display(),
            result.line_number
        ),
        std::time::Instant::now(),
    ));
}

pub(super) fn handle_file_changed(app: &mut App, path: &Path) {
    if app.diff_review.is_some() {
        return;
    }

    let buf_idx = app
        .buffers
        .iter()
        .position(|b| b.path.as_deref() == Some(path) && !b.modified);
    let Some(buf_idx) = buf_idx else {
        return;
    };

    let new_content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let old_content = app.buffers[buf_idx].text.to_string();
    if old_content == new_content {
        return;
    }

    let proposal = WriteGatePipeline::build_proposal(0, "external", path, &old_content, &new_content);

    let _ = app.buffers[buf_idx].reload();

    app.active_buffer = buf_idx;
    app.focus = Focus::Editor;
    app.diff_review = Some(DiffReviewState::new(proposal, DiffSource::External));
}

pub(super) fn open_file(app: &mut App, path: &Path) {
    for (i, buf) in app.buffers.iter().enumerate() {
        if buf.path.as_deref() == Some(path) {
            app.active_buffer = i;
            app.needs_full_redraw = true;
            return;
        }
    }

    match Buffer::open(path) {
        Ok(mut buf) => {
            use gaviero_core::workspace::settings;
            let lang = buf.lang_name.as_deref();
            let tab_size = if let Some(lang) = lang {
                app.workspace
                    .resolve_language_setting(settings::TAB_SIZE, lang, None)
            } else {
                app.workspace.resolve_setting(settings::TAB_SIZE, None)
            };
            let insert_spaces = if let Some(lang) = lang {
                app.workspace
                    .resolve_language_setting(settings::INSERT_SPACES, lang, None)
            } else {
                app.workspace.resolve_setting(settings::INSERT_SPACES, None)
            };
            let tw = tab_size.as_u64().unwrap_or(4) as u8;
            buf.tab_width = tw;
            buf.indent_unit = if insert_spaces.as_bool().unwrap_or(true) {
                " ".repeat(tw as usize)
            } else {
                "\t".to_string()
            };

            if let (Some(lang_name), Some(language)) = (&buf.lang_name, &buf.language) {
                if !app.highlight_configs.contains_key(lang_name) {
                    match load_highlight_config(language.clone(), lang_name) {
                        Ok(config) => {
                            tracing::info!("Loaded highlight config for {}", lang_name);
                            app.highlight_configs.insert(lang_name.clone(), config);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load highlights for {}: {}", lang_name, e);
                        }
                    }
                }
                buf.indent_query = app.indent_query_cache.get_or_load(lang_name, language);
            }
            app.buffers.push(buf);
            app.active_buffer = app.buffers.len() - 1;
            app.needs_full_redraw = true;
        }
        Err(e) => {
            tracing::error!("Failed to open file {}: {}", path.display(), e);
        }
    }
}

pub(super) fn cycle_tab(app: &mut App, delta: i32) {
    if app.buffers.is_empty() {
        return;
    }
    let len = app.buffers.len() as i32;
    app.active_buffer = ((app.active_buffer as i32 + delta).rem_euclid(len)) as usize;
    app.needs_full_redraw = true;
}

pub(super) fn close_tab(app: &mut App) {
    if app.buffers.is_empty() {
        return;
    }
    app.buffers.remove(app.active_buffer);
    if app.active_buffer >= app.buffers.len() && !app.buffers.is_empty() {
        app.active_buffer = app.buffers.len() - 1;
    }
}

pub(super) fn spawn_active_terminal(app: &mut App) {
    if app.terminal_manager.is_empty() {
        let root = app
            .workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        app.terminal_manager.create_tab_lazy(&root);
    }
    let rows = app
        .layout
        .terminal_area
        .map(|a| a.height.saturating_sub(2))
        .unwrap_or(24)
        .max(2);
    let cols = app
        .layout
        .terminal_area
        .map(|a| a.width)
        .unwrap_or(80)
        .max(10);
    app.terminal_manager.handle_resize(rows, cols);
    if let Err(e) = app.terminal_manager.ensure_active_spawned() {
        app.status_message = Some((format!("Terminal: {}", e), std::time::Instant::now()));
    }
}

pub(super) fn save_current_buffer(app: &mut App) {
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        if let Err(e) = buf.save() {
            tracing::error!("Save failed: {}", e);
        }
    }
}

pub(super) fn is_current_buffer_markdown(app: &App) -> bool {
    app.buffers
        .get(app.active_buffer)
        .and_then(|b| b.lang_name.as_deref())
        == Some("markdown")
}
