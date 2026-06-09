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

/// Viewport height and code-area width (gutter + scrollbar excluded).
pub(super) fn editor_viewport(buf_line_count: usize, area: Rect) -> (usize, usize) {
    let gutter_w = gutter_width(buf_line_count) as usize;
    let vp_h = area.height as usize;
    let vp_w = (area.width as usize).saturating_sub(gutter_w + 1);
    (vp_h, vp_w)
}

pub(super) fn ensure_editor_cursor_visible(app: &mut App) {
    let area = app.layout.editor_area;
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        let (vp_h, vp_w) = editor_viewport(buf.line_count(), area);
        buf.ensure_cursor_visible(vp_h, vp_w);
    }
}

pub(super) fn scroll_preview_lines(app: &mut App, delta: i32) {
    let step = delta.unsigned_abs() as usize;
    if delta < 0 {
        app.preview_scroll = app.preview_scroll.saturating_sub(step);
    } else {
        let max = app
            .preview_line_count
            .saturating_sub(app.preview_viewport_lines);
        app.preview_scroll = (app.preview_scroll + step).min(max);
    }
}

pub(super) fn handle_editor_action(app: &mut App, action: Action) {
    let read_only = app
        .buffers
        .get(app.active_buffer)
        .map(|b| b.read_only)
        .unwrap_or(false);

    if !read_only {
        match action {
            Action::NextConflict | Action::PrevConflict => {
                let forward = matches!(action, Action::NextConflict);
                if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    if buf.jump_to_conflict(forward) {
                        let n = buf.conflict_regions.len();
                        app.status_message = Some((
                            format!(
                                "Conflict {}/{}  F8 next  F9 previous",
                                buf.conflict_index + 1,
                                n
                            ),
                            std::time::Instant::now(),
                        ));
                    } else {
                        app.status_message = Some((
                            "No merge conflicts in this file".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
                ensure_editor_cursor_visible(app);
                return;
            }
            _ => {}
        }
    }

    if app.preview_mode == MarkdownPreviewMode::PreviewOnly
        && is_current_buffer_markdown(app)
    {
        match action {
            Action::PageUp => {
                scroll_preview_lines(app, -(app.preview_viewport_lines as i32));
                return;
            }
            Action::PageDown => {
                scroll_preview_lines(app, app.preview_viewport_lines as i32);
                return;
            }
            Action::CursorUp => {
                scroll_preview_lines(app, -1);
                return;
            }
            Action::CursorDown => {
                scroll_preview_lines(app, 1);
                return;
            }
            _ => {}
        }
    }

    match action {
        Action::Copy => {
            app.clipboard_copy();
            return;
        }
        Action::Cut if !read_only => {
            app.clipboard_cut();
        }
        Action::Paste if !read_only => {
            app.clipboard_paste();
        }
        Action::SelectAll => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.select_all();
            }
        }
        Action::FormatBuffer if !read_only => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                let msg = if buf.selection_range().is_some() {
                    buf.format_selection()
                } else {
                    buf.format()
                };
                app.status_message = Some((msg, std::time::Instant::now()));
            }
        }
        Action::CycleFormatLevel if !read_only => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                let msg = buf.cycle_format_level();
                app.status_message = Some((msg, std::time::Instant::now()));
            }
        }
        _ if read_only => {
            // Read-only buffer: only allow non-mutating navigation / selection.
            let area = app.layout.editor_area;
            let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
                return;
            };
            let (vp_h, vp_w) = editor_viewport(buf.line_count(), area);
            match action {
                Action::CursorUp => buf.move_cursor_up(vp_w),
                Action::CursorDown => buf.move_cursor_down(vp_w),
                Action::CursorLeft => buf.move_cursor_left(),
                Action::CursorRight => buf.move_cursor_right(),
                Action::WordLeft => buf.move_word_left(),
                Action::WordRight => buf.move_word_right(),
                Action::SelectLeft => buf.select_left(),
                Action::SelectRight => buf.select_right(),
                Action::SelectUp => buf.select_up(vp_w),
                Action::SelectDown => buf.select_down(vp_w),
                Action::SelectWordLeft => buf.select_word_left(),
                Action::SelectWordRight => buf.select_word_right(),
                Action::PageUp => buf.page_up(vp_h, vp_w),
                Action::PageDown => buf.page_down(vp_h, vp_w),
                Action::Home => buf.move_cursor_home(),
                Action::End => buf.move_cursor_end(),
                Action::GoToLineEnd => buf.move_cursor_end(),
                _ => {}
            }
            buf.ensure_cursor_visible(vp_h, vp_w);
        }
        _ => {
            let area = app.layout.editor_area;
            let Some(buf) = app.buffers.get_mut(app.active_buffer) else {
                return;
            };
            let (vp_h, vp_w) = editor_viewport(buf.line_count(), area);
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
                Action::CursorUp => buf.move_cursor_up(vp_w),
                Action::CursorDown => buf.move_cursor_down(vp_w),
                Action::CursorLeft => buf.move_cursor_left(),
                Action::CursorRight => buf.move_cursor_right(),
                Action::WordLeft => buf.move_word_left(),
                Action::WordRight => buf.move_word_right(),
                Action::SelectLeft => buf.select_left(),
                Action::SelectRight => buf.select_right(),
                Action::SelectUp => buf.select_up(vp_w),
                Action::SelectDown => buf.select_down(vp_w),
                Action::SelectWordLeft => buf.select_word_left(),
                Action::SelectWordRight => buf.select_word_right(),
                Action::PageUp => buf.page_up(vp_h, vp_w),
                Action::PageDown => buf.page_down(vp_h, vp_w),
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

    ensure_editor_cursor_visible(app);
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

    if app.has_active_review() {
        handle_mouse_review(app, mouse);
        return;
    }

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
                            SidePanelMode::GitPanel => SidePanelMode::MemoryPanel,
                            SidePanelMode::MemoryPanel => SidePanelMode::AgentChat,
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
                            let idx =
                                app.search_panel.scroll.offset + relative_row.saturating_sub(2);
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
            if let Some(preview) = app.layout.preview_area {
                if preview.contains((col, row).into()) {
                    app.focus = Focus::Editor;
                    let scrollbar_x = preview.x + preview.width.saturating_sub(1);
                    if col == scrollbar_x {
                        app.scrollbar_dragging = Some(ScrollbarTarget::MarkdownPreview);
                        app.scroll_panel_to_row(ScrollbarTarget::MarkdownPreview, row);
                        return;
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
                        .map(|(lc, lr, lt)| {
                            lc == col && lr == row && lt.elapsed().as_millis() < 400
                        })
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

                    if app.side_panel == SidePanelMode::GitPanel {
                        let rel_y = row.saturating_sub(area.y);
                        if let Some((region, idx)) =
                            app.git_panel.hit_test_file(rel_y, area.height)
                        {
                            app.git_panel.select_file(region, idx);
                            super::side_panel::open_selected_git_file(app);
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
                        if let Some(inst) = app.terminal_manager.active_instance() {
                            app.terminal_selection.start(vt_row, vt_col, inst.screen());
                        }
                    }
                    return;
                }
            }
            if app.layout.tab_area.contains((col, row).into()) {
                let titles: Vec<(String, bool, bool)> = app
                    .buffers
                    .iter()
                    .map(|b| (b.display_name().to_string(), b.modified, false))
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
                            if app.chat_state.active_conv_streaming() {
                                app.chat_state.user_scrolled_during_stream = true;
                            }
                        }
                    }
                }
            }
            if let Some(preview) = app.layout.preview_area {
                if preview.contains((col, row).into()) {
                    scroll_preview_lines(app, -3);
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
                            if app.chat_state.active_conv_streaming() {
                                app.chat_state.user_scrolled_during_stream = true;
                            }
                        }
                    }
                }
            }
            if let Some(preview) = app.layout.preview_area {
                if preview.contains((col, row).into()) {
                    scroll_preview_lines(app, 3);
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
                    let (_, content_w) =
                        editor_viewport(buf.line_count(), app.layout.editor_area);
                    let max = buf.scroll_line_count(content_w).saturating_sub(1);
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
                // Auto-scroll if dragging above/below the chat area, then extend
                // the selection to the top/bottom visible line.
                if let Some(area) = app.chat_state.conv_area_cache {
                    if row < area.y {
                        app.chat_state.scroll_offset =
                            app.chat_state.scroll_offset.saturating_sub(1);
                        let line = app.chat_state.scroll_offset;
                        let col_in_line = col.saturating_sub(area.x) as usize;
                        let line_len = app
                            .chat_state
                            .rendered_lines_cache
                            .get(line)
                            .map(|(l, _)| l.chars().count())
                            .unwrap_or(0);
                        app.chat_state
                            .extend_text_selection(line, col_in_line.min(line_len));
                        return;
                    } else if row >= area.y + area.height {
                        let total = app.chat_state.rendered_lines_cache.len();
                        let viewport = area.height as usize;
                        let max_scroll = total.saturating_sub(viewport);
                        if app.chat_state.scroll_offset < max_scroll {
                            app.chat_state.scroll_offset += 1;
                        }
                        let line = (app.chat_state.scroll_offset + viewport)
                            .min(total)
                            .saturating_sub(1);
                        let col_in_line = col.saturating_sub(area.x) as usize;
                        let line_len = app
                            .chat_state
                            .rendered_lines_cache
                            .get(line)
                            .map(|(l, _)| l.chars().count())
                            .unwrap_or(0);
                        app.chat_state
                            .extend_text_selection(line, col_in_line.min(line_len));
                        return;
                    }
                }
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
                        if let Some(inst) = app.terminal_manager.active_instance() {
                            app.terminal_selection.extend(0, vt_col, inst.screen());
                        }
                    } else if row > last_content_row {
                        if let Some(inst) = app.terminal_manager.active_instance_mut() {
                            let current = inst.screen().scrollback();
                            inst.screen_mut().set_scrollback(current.saturating_sub(1));
                        }
                        let vt_col = col.saturating_sub(area.x);
                        if let Some(inst) = app.terminal_manager.active_instance() {
                            app.terminal_selection.extend(
                                content_height.saturating_sub(1),
                                vt_col,
                                inst.screen(),
                            );
                        }
                    } else {
                        let vt_row = row - content_y_start;
                        let vt_col = col.saturating_sub(area.x);
                        if let Some(inst) = app.terminal_manager.active_instance() {
                            app.terminal_selection.extend(vt_row, vt_col, inst.screen());
                        }
                    }
                }
                return;
            }
            if app.mouse_dragging {
                let area = app.layout.editor_area;
                if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    if buf.cursor.anchor.is_none() {
                        buf.cursor.anchor = Some((buf.cursor.line, buf.cursor.col));
                    }
                }
                if row < area.y {
                    // Dragging above editor: scroll up and move cursor to top visible line.
                    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                        buf.scroll.top_line = buf.scroll.top_line.saturating_sub(1);
                        let new_line = buf.scroll.top_line;
                        buf.cursor.line = new_line;
                        let line_len = buf.line_len(new_line);
                        buf.cursor.col = buf.cursor.col.min(line_len);
                    }
                } else if row >= area.y + area.height {
                    // Dragging below editor: scroll down and move cursor to bottom visible line.
                    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                        let total = buf.line_count();
                        let viewport = area.height as usize;
                        let max_scroll = total.saturating_sub(viewport);
                        if buf.scroll.top_line < max_scroll {
                            buf.scroll.top_line += 1;
                        }
                        let new_line = (buf.scroll.top_line + viewport)
                            .min(total)
                            .saturating_sub(1);
                        buf.cursor.line = new_line;
                        let line_len = buf.line_len(new_line);
                        buf.cursor.col = buf.cursor.col.min(line_len);
                    }
                } else if area.contains((col, row).into()) {
                    app.set_cursor_from_mouse(col, row);
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if app.terminal_selection.dragging {
                let text = if let Some(inst) = app.terminal_manager.active_instance_mut() {
                    app.terminal_selection.extract_text(inst.screen_mut())
                } else {
                    None
                };
                if let Some(text) = text {
                    app.set_clipboard(&text);
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

/// Mouse handling while a review is pending. Only review-relevant
/// interactions are honored: clicking a hunk gutter to toggle accept/reject,
/// clicking a row in the batch-review file list, and scrolling the diff or
/// the file list. Every other branch (focus changes, editor cursor, terminal
/// selection, side-panel clicks, etc.) is intentionally dropped so the rest
/// of the UI stays locked until the user finishes the review.
fn handle_mouse_review(app: &mut App, mouse: crossterm::event::MouseEvent) {
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
                    }
                }
                return;
            }

            if let (Some(area), Some(ref mut br)) =
                (app.layout.file_tree_area, app.batch_review.as_mut())
            {
                if app.left_panel == LeftPanelMode::Review && area.contains((col, row).into()) {
                    let relative_row = row.saturating_sub(area.y) as usize;
                    let idx = br.scroll_offset + relative_row;
                    if idx < br.proposals.len() {
                        br.selected_index = idx;
                        br.diff_scroll = 0;
                    }
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if let Some(ref mut br) = app.batch_review {
                if app.layout.editor_area.contains((col, row).into()) {
                    br.diff_scroll = br.diff_scroll.saturating_sub(3);
                } else if let Some(area) = app.layout.file_tree_area {
                    if area.contains((col, row).into()) {
                        br.scroll_offset = br.scroll_offset.saturating_sub(3);
                    }
                }
            } else if let Some(ref mut review) = app.diff_review {
                if app.layout.editor_area.contains((col, row).into()) {
                    review.scroll_top = review.scroll_top.saturating_sub(3);
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(ref mut br) = app.batch_review {
                if app.layout.editor_area.contains((col, row).into()) {
                    br.diff_scroll += 3;
                } else if let Some(area) = app.layout.file_tree_area {
                    if area.contains((col, row).into()) {
                        let max = br.proposals.len().saturating_sub(1);
                        br.scroll_offset = (br.scroll_offset + 3).min(max);
                    }
                }
            } else if let Some(ref mut review) = app.diff_review {
                if app.layout.editor_area.contains((col, row).into()) {
                    review.scroll_top += 3;
                }
            }
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
            let (_, content_w) = editor_viewport(buf.line_count(), area);
            let total = buf.scroll_line_count(content_w);
            if total <= track_height {
                return;
            }
            let max_scroll = total.saturating_sub(track_height);
            let row_in_track = row.saturating_sub(area.y) as usize;
            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
            buf.scroll.top_line = (fraction * max_scroll as f64)
                .round()
                .min(max_scroll as f64) as usize;
        }
        ScrollbarTarget::MarkdownPreview => {
            let Some(area) = app.layout.preview_area else {
                return;
            };
            let track_height = area.height as usize;
            if track_height == 0 {
                return;
            }
            let total = app.preview_line_count;
            if total <= app.preview_viewport_lines {
                return;
            }
            let max_scroll = total.saturating_sub(app.preview_viewport_lines);
            let row_in_track = row
                .saturating_sub(area.y)
                .min(area.height.saturating_sub(1)) as usize;
            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
            app.preview_scroll = (fraction * max_scroll as f64)
                .round()
                .min(max_scroll as f64) as usize;
        }
        ScrollbarTarget::Chat => {
            let Some(area) = app.chat_state.conv_area_cache else {
                return;
            };
            let track_height = area.height as usize;
            let total = app.chat_state.rendered_lines_cache.len();
            if track_height == 0 || total <= track_height {
                return;
            }
            let max_scroll = total - track_height;
            let row_in_track = row
                .saturating_sub(area.y)
                .min(area.height.saturating_sub(1)) as usize;
            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
            app.chat_state.scroll_offset = (fraction * max_scroll as f64)
                .round()
                .min(max_scroll as f64) as usize;
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
                    let fraction =
                        row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                    app.file_tree.scroll.offset = (fraction * max_scroll as f64)
                        .round()
                        .min(max_scroll as f64)
                        as usize;
                }
                LeftPanelMode::Search => {
                    let total = app.search_panel.results.len();
                    let viewport = track_height.saturating_sub(2);
                    if total <= viewport {
                        return;
                    }
                    let max_scroll = total.saturating_sub(viewport);
                    let row_in_track = row.saturating_sub(area.y) as usize;
                    let fraction =
                        row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                    app.search_panel.scroll.offset = (fraction * max_scroll as f64)
                        .round()
                        .min(max_scroll as f64)
                        as usize;
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
                        br.scroll_offset = (fraction * max_scroll as f64)
                            .round()
                            .min(max_scroll as f64)
                            as usize;
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
                        cs.scroll_offset = (fraction * max_scroll as f64)
                            .round()
                            .min(max_scroll as f64)
                            as usize;
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
        let (_, content_w) = editor_viewport(buf.line_count(), area);
        let visual_col = (col - area.x - gutter_w) as usize + buf.scroll.left_col;
        let click_row = (row - area.y) as usize + buf.scroll.top_line;
        if buf.word_wrap && content_w > 0 {
            let layout = buf.wrap_layout(content_w);
            if let Some(seg) = layout.segment_at(click_row) {
                buf.cursor.line = seg.logical_line;
                let base_visual = buf.char_col_to_visual(seg.logical_line, seg.start_col);
                let char_col = buf.visual_to_char_col(seg.logical_line, base_visual + visual_col);
                buf.cursor.col = char_col.min(buf.line_len(buf.cursor.line));
            }
        } else {
            let max_line = buf.line_count().saturating_sub(1);
            buf.cursor.line = click_row.min(max_line);
            let char_col = buf.visual_to_char_col(buf.cursor.line, visual_col);
            let line_len = buf.line_len(buf.cursor.line);
            buf.cursor.col = char_col.min(line_len);
        }
    }
}

pub(super) fn handle_paste(app: &mut App, text: &str) {
    if app.has_active_review() {
        return;
    }
    match app.focus {
        Focus::Editor => {
            if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                buf.paste_text(text);
                let area = app.layout.editor_area;
                let (vp_h, vp_w) = editor_viewport(buf.line_count(), area);
                buf.ensure_cursor_visible(vp_h, vp_w);
            }
        }
        Focus::SidePanel => {
            // Many terminals intercept Ctrl+V / Ctrl+Shift+V themselves and
            // forward an empty bracketed-paste payload when the clipboard
            // holds an image (no text representation). Prefer attaching the
            // image in that case so the paste still works.
            if text.is_empty() && super::side_panel::try_attach_clipboard_image(app) {
                return;
            }
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

/// Look up a pre-turn snapshot for an Option-B tool-agent edit.
fn pending_pre_turn_content(
    pending: &std::collections::HashMap<std::path::PathBuf, Option<String>>,
    path: &Path,
) -> Option<Option<String>> {
    if let Some(content) = pending.get(path) {
        return Some(content.clone());
    }
    pending.iter().find_map(|(p, content)| {
        Buffer::paths_refer_to_same_file(p, path)
            .then(|| content.clone())
    })
}

/// Resolve the "before" side of an external-change diff for a tool-agent edit.
/// The per-turn snapshot wins over the open buffer so a file-watcher reload
/// cannot erase the diff.
fn tool_agent_old_content(app: &App, path: &Path) -> String {
    if let Some(pre_turn) = pending_pre_turn_content(&app.pending_tool_agent_edits, path) {
        return pre_turn.unwrap_or_default();
    }
    app.buffers
        .iter()
        .find(|b| {
            b.path
                .as_deref()
                .is_some_and(|p| Buffer::paths_refer_to_same_file(p, path))
        })
        .map(|b| b.text.to_string())
        .unwrap_or_default()
}

/// Open external-change review for an in-process tool-agent edit (Option B).
pub(super) fn open_tool_agent_edit_review(app: &mut App, path: &Path) {
    if app.diff_review.is_some() {
        return;
    }

    let read_path = Buffer::resolve_editor_path(path);
    let new_content = match std::fs::read_to_string(&read_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let old_content = tool_agent_old_content(app, path);

    if old_content == new_content {
        return;
    }

    let proposal = gaviero_core::write_gate::WriteGatePipeline::build_proposal(
        0,
        "tool-agent",
        None,
        &read_path,
        &old_content,
        &new_content,
    );

    if let Some(buf_idx) = app.buffers.iter().position(|b| {
        b.path
            .as_deref()
            .is_some_and(|p| crate::editor::buffer::Buffer::paths_refer_to_same_file(p, path))
    }) {
        app.active_buffer = buf_idx;
    }
    app.focus = crate::app::Focus::Editor;
    app.diff_review = Some(crate::editor::diff_overlay::DiffReviewState::new(
        proposal,
        crate::editor::diff_overlay::DiffSource::External,
    ));
}

pub(super) fn handle_file_changed(app: &mut App, path: &Path) {
    if app.diff_review.is_some() {
        return;
    }

    let buf_idx = app.buffers.iter().position(|b| {
        b.path.as_deref().is_some_and(|buf_path| {
            Buffer::paths_refer_to_same_file(buf_path, path) && !b.modified
        })
    });
    let Some(buf_idx) = buf_idx else {
        return;
    };

    let read_path = match app.buffers[buf_idx].path.as_deref() {
        Some(p) => p.to_path_buf(),
        None => return,
    };

    let new_content = match std::fs::read_to_string(&read_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let buf = &mut app.buffers[buf_idx];
    if buf.should_suppress_post_open_watch() {
        // Watcher often replays stale metadata right after reopen; trust the
        // content we just loaded until the user edits or the grace window ends.
        buf.note_disk_sync(buf.text.to_string());
        return;
    }
    if buf.should_ignore_external_change(&new_content) {
        buf.note_disk_sync(&new_content);
        return;
    }

    let old_content = pending_pre_turn_content(&app.pending_tool_agent_edits, path)
        .map(|pre| pre.unwrap_or_default())
        .unwrap_or_else(|| buf.text.to_string());
    if old_content == new_content {
        buf.note_disk_sync(&new_content);
        return;
    }

    let source = if pending_pre_turn_content(&app.pending_tool_agent_edits, path).is_some() {
        "tool-agent"
    } else {
        "external"
    };
    let proposal = WriteGatePipeline::build_proposal(
        0,
        source,
        None,
        &read_path,
        &old_content,
        &new_content,
    );

    app.active_buffer = buf_idx;
    app.focus = Focus::Editor;
    app.diff_review = Some(DiffReviewState::new(proposal, DiffSource::External));
}

/// Open a read-only diff view of `path` as a regular editor tab. The buffer
/// holds the unified diff (each line tagged context/added/removed) and is
/// rendered with green/red row tints; syntax highlighting is preserved. If
/// a diff-view tab for the same path is already open, it is reused (and
/// recomputed against the latest contents). A separate writable buffer for
/// the same path can coexist as another tab.
pub(super) fn open_diff_view(
    app: &mut App,
    path: &Path,
    original: String,
    current: String,
) {
    // Reuse an existing diff-view tab for the same path if any.
    for (i, b) in app.buffers.iter().enumerate() {
        if b.diff_view.is_some() && b.path.as_deref() == Some(path) {
            app.active_buffer = i;
            app.needs_full_redraw = true;
            return;
        }
    }

    match Buffer::open_diff_view(path, original, current) {
        Ok(mut buf) => {
            use gaviero_core::workspace::settings;
            let lang = buf.lang_name.as_deref();
            let tab_size = if let Some(lang) = lang {
                app.workspace
                    .resolve_language_setting(settings::TAB_SIZE, lang, None)
            } else {
                app.workspace.resolve_setting(settings::TAB_SIZE, None)
            };
            buf.tab_width = tab_size.as_u64().unwrap_or(4) as u8;
            let word_wrap = app.workspace.resolve_setting(settings::WORD_WRAP, None);
            buf.word_wrap = word_wrap.as_bool().unwrap_or(false);

            if let (Some(lang_name), Some(language)) = (&buf.lang_name, &buf.language) {
                if !app.highlight_configs.contains_key(lang_name) {
                    if let Ok(config) = load_highlight_config(language.clone(), lang_name) {
                        app.highlight_configs.insert(lang_name.clone(), config);
                    }
                }
            }
            app.buffers.push(buf);
            app.active_buffer = app.buffers.len() - 1;
            app.needs_full_redraw = true;
        }
        Err(e) => {
            tracing::error!("Failed to open diff view for {}: {}", path.display(), e);
        }
    }
}

pub(super) fn sync_preview_mode_for_active_buffer(app: &mut App) {
    if !is_current_buffer_markdown(app) && app.preview_mode != MarkdownPreviewMode::Off {
        app.preview_mode = MarkdownPreviewMode::Off;
        app.preview_scroll = 0;
    }
}

pub(super) fn open_file(app: &mut App, path: &Path) {
    let path = Buffer::resolve_editor_path(path);
    for (i, buf) in app.buffers.iter().enumerate() {
        if buf
            .path
            .as_deref()
            .is_some_and(|existing| Buffer::paths_refer_to_same_file(existing, &path))
        {
            app.active_buffer = i;
            sync_preview_mode_for_active_buffer(app);
            app.needs_full_redraw = true;
            return;
        }
    }

    match Buffer::open(&path) {
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
            let word_wrap = app.workspace.resolve_setting(settings::WORD_WRAP, None);
            buf.word_wrap = word_wrap.as_bool().unwrap_or(false);

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
            let git_unmerged = app
                .workspace
                .roots()
                .first()
                .and_then(|root| {
                    path.strip_prefix(root).ok().and_then(|rel| {
                        gaviero_core::git::GitRepo::open(root)
                            .ok()
                            .and_then(|repo| repo.is_path_unmerged(rel.to_str()?).ok())
                    })
                })
                .unwrap_or(false);
            buf.refresh_conflict_metadata(git_unmerged);

            app.buffers.push(buf);
            app.active_buffer = app.buffers.len() - 1;
            sync_preview_mode_for_active_buffer(app);
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
    sync_preview_mode_for_active_buffer(app);
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
    let saved = app
        .buffers
        .get(app.active_buffer)
        .and_then(|b| b.path.clone());
    if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
        if let Err(e) = buf.save() {
            tracing::error!("Save failed: {}", e);
            app.status_message = Some((
                format!("Save failed: {e}"),
                std::time::Instant::now(),
            ));
            return;
        }
        buf.refresh_conflict_metadata(buf.git_unmerged);
    }
    if let Some(path) = saved {
        stage_file_if_conflict_resolved(app, &path);
        app.status_message = Some((
            format!("Saved {}", path.display()),
            std::time::Instant::now(),
        ));
    }
    app.refresh_git_changes();
}

fn stage_file_if_conflict_resolved(app: &mut App, path: &std::path::Path) {
    let root = match app.workspace.roots().first() {
        Some(r) => r.clone(),
        None => return,
    };
    let rel = match path.strip_prefix(&root).ok().and_then(|p| p.to_str()) {
        Some(r) => r,
        None => return,
    };
    let Ok(repo) = gaviero_core::git::GitRepo::open(&root) else {
        return;
    };
    let Ok(true) = repo.is_path_unmerged(rel) else {
        return;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    if gaviero_core::git_conflict::file_has_conflict_markers(&content) {
        return;
    }
    if repo.stage_file(rel).is_ok() {
        app.status_message = Some((
            format!("Staged resolved {} — run git commit to finish merge", rel),
            std::time::Instant::now(),
        ));
        if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
            buf.git_unmerged = false;
        }
    }
}

pub(super) fn is_current_buffer_markdown(app: &App) -> bool {
    app.buffers
        .get(app.active_buffer)
        .and_then(|b| b.lang_name.as_deref())
        == Some("markdown")
}
