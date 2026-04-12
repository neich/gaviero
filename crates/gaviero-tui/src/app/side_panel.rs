use super::*;

pub(super) fn handle_chat_action(app: &mut App, action: Action) {
    app.chat_state.clear_text_selection();

    if app.chat_state.renaming {
        match action {
            Action::Enter => app.chat_state.confirm_rename(),
            Action::Quit => app.chat_state.cancel_rename(),
            Action::InsertChar(ch) => app.chat_state.insert_char(ch),
            Action::Backspace => app.chat_state.backspace(),
            Action::Delete => app.chat_state.text_input.delete(),
            Action::CursorLeft => app.chat_state.text_input.move_left(),
            Action::CursorRight => app.chat_state.text_input.move_right(),
            Action::Home => app.chat_state.text_input.move_home(),
            Action::End => app.chat_state.text_input.move_end(),
            _ => {}
        }
        return;
    }

    if app.chat_state.active_conv_pending_permission() {
        match action {
            Action::InsertChar('y') | Action::InsertChar('Y') => {
                app.chat_state.respond_active_permission(true);
            }
            Action::InsertChar('n') | Action::InsertChar('N') | Action::Quit => {
                app.chat_state.respond_active_permission(false);
            }
            _ => {}
        }
        return;
    }

    if app.chat_state.browse_mode {
        match action {
            Action::CursorUp => app.chat_state.browse_up(),
            Action::CursorDown => app.chat_state.browse_down(),
            Action::Copy => {
                if let Some(text) = app.chat_state.browsed_message_content() {
                    app.set_clipboard(&text);
                }
                app.chat_state.exit_browse_mode();
            }
            Action::Quit | Action::Enter => {
                app.chat_state.exit_browse_mode();
            }
            _ => {
                app.chat_state.exit_browse_mode();
                app.handle_chat_action(action);
                return;
            }
        }
        return;
    }

    let ac_active =
        app.chat_state.autocomplete.active && !app.chat_state.autocomplete.matches.is_empty();

    match action {
        Action::Rename => {
            app.chat_state.start_rename();
            return;
        }
        Action::Tab if ac_active => {
            app.chat_state.accept_autocomplete();
            return;
        }
        Action::Enter if ac_active => {
            app.chat_state.accept_autocomplete();
            return;
        }
        Action::CursorUp if ac_active => {
            app.chat_state.autocomplete_up();
            return;
        }
        Action::CursorDown if ac_active => {
            app.chat_state.autocomplete_down();
            return;
        }
        Action::Quit if ac_active => {
            app.chat_state.autocomplete.reset();
            return;
        }
        Action::Enter => {
            if !app.chat_state.text_input.text.is_empty() && !app.chat_state.active_conv_streaming()
            {
                if app.batch_review.is_some() {
                    app.status_message = Some((
                        "Exit review first (f: apply, Esc: discard)".to_string(),
                        std::time::Instant::now(),
                    ));
                    return;
                }

                app.chat_state.scroll_pinned_to_bottom = true;

                if app.chat_state.text_input.text.trim().starts_with("/cswarm") {
                    app.handle_coordinated_swarm_command();
                } else if app.chat_state.text_input.text.trim() == "/undo-swarm" {
                    app.handle_undo_swarm_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/swarm") {
                    app.handle_swarm_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/remember") {
                    app.handle_remember_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/attach") {
                    app.handle_attach_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/detach") {
                    app.handle_detach_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/run") {
                    app.handle_run_script_command();
                } else if !app.chat_state.process_slash_command() {
                    app.send_chat_message();
                }
            }
        }
        Action::AltEnter => {
            app.chat_state.insert_char('\n');
        }
        Action::ToggleAutoApprove => {
            app.chat_state.toggle_auto_approve();
            let state = if app.chat_state.effective_auto_approve() {
                "ON"
            } else {
                "OFF"
            };
            app.status_message = Some((
                format!("Auto-approve: {} (next message only)", state),
                std::time::Instant::now(),
            ));
        }
        Action::InsertChar(ch) => {
            if !app.chat_state.active_conv_streaming() {
                app.chat_state.insert_char(ch);
                app.refresh_chat_autocomplete();
            }
        }
        Action::Backspace => {
            app.chat_state.backspace();
            app.refresh_chat_autocomplete();
        }
        Action::Delete => app.chat_state.text_input.delete(),
        Action::Undo => app.chat_state.text_input.undo(),
        Action::Redo => app.chat_state.text_input.redo(),
        Action::SelectAll => app.chat_state.text_input.select_all(),
        Action::DeleteWordBack => {
            app.chat_state.delete_word_back();
            app.refresh_chat_autocomplete();
        }
        Action::CursorLeft => app.chat_state.text_input.move_left(),
        Action::CursorRight => app.chat_state.text_input.move_right(),
        Action::WordLeft => app.chat_state.text_input.move_word_left(),
        Action::WordRight => app.chat_state.text_input.move_word_right(),
        Action::SelectLeft => app.chat_state.text_input.select_left(),
        Action::SelectRight => app.chat_state.text_input.select_right(),
        Action::SelectWordLeft => app.chat_state.text_input.select_word_left(),
        Action::SelectWordRight => app.chat_state.text_input.select_word_right(),
        Action::Home => app.chat_state.text_input.move_home(),
        Action::End => app.chat_state.text_input.move_end(),
        Action::CursorUp => {
            let streaming = app.chat_state.active_conv_streaming();
            if streaming {
                app.chat_state.scroll_offset = app.chat_state.scroll_offset.saturating_sub(1);
                app.chat_state.user_scrolled_during_stream = true;
            } else {
                let prompt_len = 2;
                let panel_w = app
                    .layout
                    .side_panel_area
                    .map(|a| a.width)
                    .unwrap_or(40)
                    .saturating_sub(2) as usize;
                let first_w = panel_w.saturating_sub(prompt_len);
                let has_visual_lines = !app.chat_state.text_input.text.is_empty()
                    && (app.chat_state.input_is_multiline()
                        || app.chat_state.input_wraps_visually(first_w, panel_w));

                if has_visual_lines {
                    if !app.chat_state.move_up_visual(first_w, panel_w) {
                        if app.chat_state.history_index.is_some()
                            || app.chat_state.text_input.text.is_empty()
                        {
                            app.chat_state.history_up();
                        } else {
                            app.chat_state.scroll_offset =
                                app.chat_state.scroll_offset.saturating_sub(1);
                        }
                    }
                } else if app.chat_state.history_index.is_some()
                    || app.chat_state.text_input.text.is_empty()
                {
                    app.chat_state.history_up();
                } else {
                    app.chat_state.scroll_offset = app.chat_state.scroll_offset.saturating_sub(1);
                }
            }
        }
        Action::CursorDown => {
            let streaming = app.chat_state.active_conv_streaming();
            if streaming {
                app.chat_state.scroll_offset += 1;
            } else {
                let prompt_len = 2;
                let panel_w = app
                    .layout
                    .side_panel_area
                    .map(|a| a.width)
                    .unwrap_or(40)
                    .saturating_sub(2) as usize;
                let first_w = panel_w.saturating_sub(prompt_len);
                let has_visual_lines = !app.chat_state.text_input.text.is_empty()
                    && (app.chat_state.input_is_multiline()
                        || app.chat_state.input_wraps_visually(first_w, panel_w));

                if has_visual_lines {
                    if !app.chat_state.move_down_visual(first_w, panel_w) {
                        if app.chat_state.history_index.is_some() {
                            app.chat_state.history_down();
                        } else {
                            app.chat_state.scroll_offset += 1;
                        }
                    }
                } else if app.chat_state.history_index.is_some() {
                    app.chat_state.history_down();
                } else {
                    app.chat_state.scroll_offset += 1;
                }
            }
        }
        Action::PageUp => {
            app.chat_state.scroll_offset = app.chat_state.scroll_offset.saturating_sub(20);
            if app.chat_state.active_conv_streaming() {
                app.chat_state.user_scrolled_during_stream = true;
            }
        }
        Action::PageDown => {
            app.chat_state.scroll_offset = app.chat_state.scroll_offset.saturating_add(20);
        }
        Action::Quit => {
            if !app.chat_state.text_input.text.is_empty() {
                app.chat_state.text_input.clear();
                app.chat_state.autocomplete.reset();
            } else {
                app.focus = Focus::Editor;
            }
        }
        Action::Paste => {
            if !app.chat_state.active_conv_streaming() {
                app.chat_paste_from_clipboard();
            }
        }
        Action::Copy => {
            if app.chat_state.active_conv_streaming() {
                app.cancel_agent();
            } else {
                app.chat_state.enter_browse_mode();
            }
        }
        _ => {}
    }
}

pub(super) fn handle_swarm_dashboard_action(app: &mut App, action: Action) {
    use crate::panels::swarm_dashboard::DashboardFocus;

    match action {
        Action::Enter => {
            if app.swarm_dashboard.diff_agent.is_some() {
                app.swarm_dashboard.close_diff();
                return;
            }
            let agent = app
                .swarm_dashboard
                .agents
                .get(app.swarm_dashboard.scroll.selected);
            let branch = agent.and_then(|a| a.branch.clone());
            let agent_id = agent.map(|a| a.id.clone());
            let is_completed = agent
                .map(|a| matches!(a.status, gaviero_core::swarm::models::AgentStatus::Completed))
                .unwrap_or(false);

            if !is_completed {
                return;
            }
            match (branch, agent_id) {
                (Some(branch), Some(agent_id)) => {
                    let root = app
                        .workspace
                        .roots()
                        .first()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let pre_sha = app
                        .swarm_dashboard
                        .result
                        .as_ref()
                        .map(|r| r.pre_swarm_sha.clone())
                        .unwrap_or_default();
                    let diff_text = gaviero_core::git::diff_branch_vs_sha(&root, &pre_sha, &branch)
                        .unwrap_or_default();
                    app.swarm_dashboard.show_diff(agent_id, diff_text);
                }
                (_, Some(id)) => {
                    app.swarm_dashboard.status_message = format!(
                        "No diff available for '{}' (agent ran without worktree isolation)",
                        id
                    );
                }
                _ => {}
            }
            return;
        }
        Action::InsertChar('u') => {
            let can_undo = app
                .swarm_dashboard
                .result
                .as_ref()
                .map(|r| !r.pre_swarm_sha.is_empty())
                .unwrap_or(false);
            if !can_undo {
                return;
            }

            if app.swarm_dashboard.pending_undo_confirm {
                app.swarm_dashboard.pending_undo_confirm = false;
                let result = match app.swarm_dashboard.result.clone() {
                    Some(r) => r,
                    None => return,
                };
                let root = app
                    .workspace
                    .roots()
                    .first()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let tx = app.event_tx.clone();
                tokio::task::spawn(async move {
                    let revert_result = tokio::task::spawn_blocking(move || {
                        gaviero_core::swarm::pipeline::revert_swarm(&root, &result)
                    })
                    .await;
                    match revert_result {
                        Ok(Ok(())) => {
                            let _ = tx.send(Event::SwarmPhaseChanged("reverted".to_string()));
                        }
                        Ok(Err(e)) => {
                            let _ =
                                tx.send(Event::SwarmPhaseChanged(format!("revert failed: {}", e)));
                        }
                        Err(e) => {
                            let _ =
                                tx.send(Event::SwarmPhaseChanged(format!("revert panicked: {}", e)));
                        }
                    }
                });
            } else {
                app.swarm_dashboard.pending_undo_confirm = true;
            }
            return;
        }
        _ => {}
    }

    let dash = &mut app.swarm_dashboard;
    let agent_count = dash.agents.len();

    macro_rules! reset_detail {
        ($dash:expr) => {
            $dash.detail_scroll = 0;
            $dash.detail_auto_scroll = true;
        };
    }

    match action {
        Action::InsertChar('\t') => {
            dash.cycle_focus();
        }
        Action::CursorUp | Action::InsertChar('k') => {
            if let Some(ref mut diff) = dash.diff_agent {
                diff.scroll = diff.scroll.saturating_sub(1);
            } else {
                match dash.focus {
                    DashboardFocus::Table => {
                        let prev = dash.scroll.selected;
                        dash.scroll.move_up();
                        if dash.scroll.selected != prev {
                            reset_detail!(dash);
                        }
                    }
                    DashboardFocus::Detail => {
                        dash.detail_auto_scroll = false;
                        dash.detail_scroll = dash.detail_scroll.saturating_sub(1);
                    }
                }
            }
        }
        Action::CursorDown | Action::InsertChar('j') => {
            if let Some(ref mut diff) = dash.diff_agent {
                diff.scroll = diff.scroll.saturating_add(1);
            } else {
                match dash.focus {
                    DashboardFocus::Table => {
                        let prev = dash.scroll.selected;
                        dash.scroll.move_down(agent_count);
                        if dash.scroll.selected != prev {
                            reset_detail!(dash);
                        }
                    }
                    DashboardFocus::Detail => {
                        if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                            let w = dash.detail_rect.width.saturating_sub(1) as usize;
                            let total = crate::panels::swarm_dashboard::count_display_lines(
                                &agent.activity,
                                w,
                            );
                            dash.detail_scroll =
                                (dash.detail_scroll + 1).min(total.saturating_sub(1));
                        }
                    }
                }
            }
        }
        Action::PageUp => match dash.focus {
            DashboardFocus::Table => {
                dash.scroll.selected = dash.scroll.selected.saturating_sub(10);
                dash.scroll.ensure_visible();
                reset_detail!(dash);
            }
            DashboardFocus::Detail => {
                dash.detail_auto_scroll = false;
                dash.detail_scroll = dash.detail_scroll.saturating_sub(10);
            }
        },
        Action::PageDown => match dash.focus {
            DashboardFocus::Table => {
                dash.scroll.selected = (dash.scroll.selected + 10).min(agent_count.saturating_sub(1));
                dash.scroll.ensure_visible();
                reset_detail!(dash);
            }
            DashboardFocus::Detail => {
                if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                    let w = dash.detail_rect.width.saturating_sub(1) as usize;
                    let total =
                        crate::panels::swarm_dashboard::count_display_lines(&agent.activity, w);
                    dash.detail_scroll = (dash.detail_scroll + 10).min(total.saturating_sub(1));
                }
            }
        },
        Action::Home => match dash.focus {
            DashboardFocus::Table => {
                dash.scroll.reset();
            }
            DashboardFocus::Detail => {
                dash.detail_scroll = 0;
                dash.detail_auto_scroll = false;
            }
        },
        Action::End => match dash.focus {
            DashboardFocus::Table => {
                dash.scroll.selected = agent_count.saturating_sub(1);
                dash.scroll.ensure_visible();
            }
            DashboardFocus::Detail => {
                dash.detail_auto_scroll = true;
            }
        },
        Action::InsertChar('f') => {
            dash.detail_auto_scroll = !dash.detail_auto_scroll;
        }
        _ => {}
    }
}

pub(super) fn handle_git_panel_action(app: &mut App, action: Action) {
    use crate::panels::git_panel::GitRegion;

    if app.git_panel.branch_picker_open {
        match action {
            Action::CursorUp | Action::InsertChar('k') => app.git_panel.branch_picker_up(),
            Action::CursorDown | Action::InsertChar('j') => app.git_panel.branch_picker_down(),
            Action::Enter => {
                if let Some(name) = app.git_panel.selected_branch_name() {
                    if let Some(repo) = &app.git_repo {
                        if let Err(e) = repo.checkout(&name) {
                            app.git_panel.error_message = Some(format!("{}", e));
                        }
                        app.git_panel.refresh(repo);
                    }
                }
                app.git_panel.close_branch_picker();
            }
            Action::Quit => app.git_panel.close_branch_picker(),
            Action::Backspace => app.git_panel.branch_picker_backspace(),
            Action::InsertChar(ch) => app.git_panel.branch_picker_insert(ch),
            _ => {}
        }
        return;
    }

    match action {
        Action::CursorUp | Action::InsertChar('k') => app.git_panel.move_up(),
        Action::CursorDown | Action::InsertChar('j') => app.git_panel.move_down(),
        Action::Tab => app.git_panel.cycle_region(),
        Action::InsertChar('s') if app.git_panel.region != GitRegion::CommitInput => {
            if let Some(path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                if let Some(repo) = &app.git_repo {
                    if let Err(e) = repo.stage_file(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(repo);
                }
            }
        }
        Action::InsertChar('u') if app.git_panel.region != GitRegion::CommitInput => {
            if let Some(path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                if let Some(repo) = &app.git_repo {
                    if let Err(e) = repo.unstage_file(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(repo);
                }
            }
        }
        Action::InsertChar('d') if app.git_panel.region != GitRegion::CommitInput => {
            if let Some(path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                if let Some(repo) = &app.git_repo {
                    if let Err(e) = repo.discard_changes(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(repo);
                    app.refresh_file_tree();
                }
            }
        }
        Action::InsertChar('c') if app.git_panel.region != GitRegion::CommitInput => {
            if !app.git_panel.commit_input.is_empty() {
                if let Some(repo) = &app.git_repo {
                    match repo.commit(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(repo);
                            app.refresh_file_tree();
                        }
                        Err(e) => {
                            app.git_panel.error_message = Some(format!("{}", e));
                        }
                    }
                }
            } else {
                app.git_panel.region = GitRegion::CommitInput;
            }
        }
        Action::InsertChar('a') if app.git_panel.region != GitRegion::CommitInput => {
            if !app.git_panel.commit_input.is_empty() {
                if let Some(repo) = &app.git_repo {
                    match repo.amend(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(repo);
                        }
                        Err(e) => {
                            app.git_panel.error_message = Some(format!("{}", e));
                        }
                    }
                }
            }
        }
        Action::InsertChar(ch) if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.insert_char(ch);
        }
        Action::Backspace if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.backspace();
        }
        Action::CursorLeft if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.move_left();
        }
        Action::CursorRight if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.move_right();
        }
        Action::WordLeft if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.move_word_left();
        }
        Action::WordRight if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.move_word_right();
        }
        Action::SelectLeft if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.select_left();
        }
        Action::SelectRight if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.select_right();
        }
        Action::SelectWordLeft if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.select_word_left();
        }
        Action::SelectWordRight if app.git_panel.region == GitRegion::CommitInput => {
            app.git_panel.commit_input.select_word_right();
        }
        Action::Enter if app.git_panel.region == GitRegion::CommitInput => {
            if !app.git_panel.commit_input.is_empty() {
                if let Some(repo) = &app.git_repo {
                    match repo.commit(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(repo);
                            app.refresh_file_tree();
                        }
                        Err(e) => {
                            app.git_panel.error_message = Some(format!("{}", e));
                        }
                    }
                }
            }
        }
        Action::Enter => {
            if let Some(rel_path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                let root = app.workspace.roots().first().map(|p| p.to_path_buf());
                if let Some(root) = root {
                    let abs_path = root.join(&rel_path);
                    if abs_path.exists() {
                        let original = app.git_head_content(&rel_path).unwrap_or_default();
                        let current = std::fs::read_to_string(&abs_path).unwrap_or_default();

                        if original != current {
                            let proposal = WriteGatePipeline::build_proposal(
                                0,
                                "git-diff",
                                &abs_path,
                                &original,
                                &current,
                            );
                            app.open_file(&abs_path);
                            app.focus = Focus::Editor;
                            app.diff_review =
                                Some(DiffReviewState::new(proposal, DiffSource::Acp));
                        } else {
                            app.open_file(&abs_path);
                            app.focus = Focus::Editor;
                        }
                    }
                }
            }
        }
        Action::InsertChar('b') if app.git_panel.region != GitRegion::CommitInput => {
            app.git_panel.toggle_branch_picker();
        }
        Action::Quit => {
            if app.git_panel.branch_picker_open {
                app.git_panel.close_branch_picker();
            } else if app.git_panel.region == GitRegion::CommitInput
                && !app.git_panel.commit_input.is_empty()
            {
                app.git_panel.commit_input.clear();
            } else {
                app.focus = Focus::Editor;
            }
        }
        _ => {}
    }
}

pub(super) fn git_head_content(app: &App, rel_path: &str) -> Option<String> {
    let repo = app.git_repo.as_ref()?;
    let head = repo.head_file_content(rel_path).ok()?;
    Some(head)
}

pub(super) fn refresh_git_panel(app: &mut App) {
    if let Some(repo) = &app.git_repo {
        app.git_panel.refresh(repo);
    }
}

pub(super) fn refresh_chat_autocomplete(app: &mut App) {
    if !app.chat_state.autocomplete.active {
        return;
    }
    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let at_pos = app.chat_state.autocomplete.at_pos;
    let is_run_path_context = {
        let text = &app.chat_state.text_input.text;
        at_pos <= text.len() && text[..at_pos].trim() == "/run"
    };

    let files: Vec<String> = app
        .file_tree
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter_map(|e| {
            e.path
                .strip_prefix(&root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .filter(|f| !is_run_path_context || f.ends_with(".gaviero"))
        .collect();

    app.chat_state.update_autocomplete_matches(&files);
}

pub(super) fn send_chat_message(app: &mut App) {
    let conv_id = app.chat_state.conversations[app.chat_state.active_conv]
        .id
        .clone();
    let prompt = app.chat_state.take_input();
    app.chat_state.add_user_message(&prompt);
    app.chat_state.conversations[app.chat_state.active_conv].is_streaming = true;
    app.chat_state.conversations[app.chat_state.active_conv].streaming_status =
        "Connecting...".to_string();
    app.chat_state.conversations[app.chat_state.active_conv].streaming_started_at =
        Some(std::time::Instant::now());

    let tx = app.event_tx.clone();
    let wg = app.write_gate.clone();
    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let refs = crate::panels::agent_chat::parse_file_references(&prompt);
    let mut file_refs: Vec<(String, String)> = Vec::new();
    for rel_path in &refs {
        let abs_path = root.join(rel_path);
        if let Ok(content) = std::fs::read_to_string(&abs_path) {
            file_refs.push((rel_path.clone(), content));
        }
    }

    let attachments = app.chat_state.take_attachments();
    let mut cli_file_attachments: Vec<std::path::PathBuf> = Vec::new();
    for attach in &attachments {
        match attach.kind {
            crate::panels::agent_chat::AttachmentKind::Text => {
                if let Ok(content) = std::fs::read_to_string(&attach.path) {
                    file_refs.push((attach.display_name.clone(), content));
                }
            }
            crate::panels::agent_chat::AttachmentKind::Image => {
                cli_file_attachments.push(attach.path.clone());
            }
        }
    }

    let context: Vec<(String, String)> = app
        .chat_state
        .context_messages()
        .into_iter()
        .rev()
        .skip(1)
        .rev()
        .map(|(r, c)| (r.to_string(), c.to_string()))
        .collect();

    let model = app.chat_state.effective_model().to_string();
    let effort = app.chat_state.effective_effort().to_string();
    let max_tokens = app.chat_state.agent_settings.max_tokens;
    let auto_approve = app.chat_state.effective_auto_approve();
    app.chat_state.auto_approve_next = false;

    let options = gaviero_core::acp::session::AgentOptions {
        effort,
        max_tokens,
        auto_approve,
    };

    let memory = app.memory.clone();
    let read_ns = app.chat_state.agent_settings.read_namespaces.clone();
    let ollama_base_url = app.chat_state.agent_settings.ollama_base_url.clone();
    let graph_budget_tokens = app.chat_state.agent_settings.graph_budget_tokens;
    let repo_map_cache = app.repo_map.clone();
    let graph_root = app.graph_workspace_root.clone().unwrap_or_else(|| root.clone());

    // Seed paths for graph ranking: explicit @file refs + active buffer (if any), made relative to workspace root.
    let mut graph_seeds: Vec<String> = refs.clone();
    if let Some(buf) = app.buffers.get(app.active_buffer) {
        if let Some(p) = buf.path.as_deref() {
            if let Ok(rel) = p.strip_prefix(&graph_root) {
                graph_seeds.push(rel.to_string_lossy().to_string());
            }
        }
    }
    graph_seeds.sort();
    graph_seeds.dedup();

    let conv_id_clone = conv_id.clone();
    let task = tokio::spawn(async move {
        {
            let mut gate = wg.lock().await;
            tracing::info!("Write gate mode before: {:?}, switching to Deferred", gate.mode());
            gate.set_mode(WriteMode::Deferred);
        }

        // Graph-backed source context: ranked outline + impact radius for the seeds.
        // This narrows what the LLM needs to pull via Read/Grep and is prepended to memory context.
        let graph_ctx = crate::app::session::build_graph_context(
            repo_map_cache,
            graph_root,
            graph_seeds,
            graph_budget_tokens,
        )
        .await;

        let mem_ctx = if let Some(ref mem) = memory {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                mem.search_context(&read_ns, &prompt, 5),
            )
            .await
            {
                Ok(ctx) => ctx,
                Err(_) => {
                    tracing::warn!(
                        "Memory search timed out after 5s, proceeding without context"
                    );
                    String::new()
                }
            }
        } else {
            String::new()
        };

        let mut parts: Vec<String> = Vec::new();
        if !graph_ctx.is_empty() {
            parts.push(graph_ctx);
        }
        if !mem_ctx.is_empty() {
            parts.push(mem_ctx);
        }
        parts.push(prompt.clone());
        let enriched_prompt = parts.join("\n\n");

        let observer = TuiAcpObserver {
            tx: tx.clone(),
            conv_id: conv_id_clone.clone(),
        };
        let pipeline = AcpPipeline::new(
            wg.clone(),
            Box::new(observer),
            model,
            Some(ollama_base_url),
            root,
            "claude-chat",
            options,
        );
        if let Err(e) = pipeline
            .send_prompt(&enriched_prompt, &file_refs, &context, &cli_file_attachments)
            .await
        {
            tracing::error!("send_prompt error: {}", e);
            let _ = tx.send(Event::MessageComplete {
                conv_id: conv_id.clone(),
                role: "system".to_string(),
                content: format!("Error: {}", e),
            });
        }

        let proposals = {
            let mut gate = wg.lock().await;
            let proposals = gate.take_pending_proposals();
            tracing::info!(
                "Draining deferred proposals: count={}, switching back to Interactive",
                proposals.len()
            );
            gate.set_mode(WriteMode::Interactive);
            proposals
        };
        if !proposals.is_empty() {
            tracing::info!("Sending AcpTaskCompleted with {} proposals", proposals.len());
            let _ = tx.send(Event::AcpTaskCompleted {
                conv_id: conv_id_clone,
                proposals,
            });
        } else {
            tracing::info!("No deferred proposals — skipping AcpTaskCompleted");
        }
    });
    app.acp_tasks.insert(
        app.chat_state.conversations[app.chat_state.active_conv]
            .id
            .clone(),
        task,
    );
}

pub(super) fn chat_paste_from_clipboard(app: &mut App) {
    if let Some(cb) = &mut app.clipboard {
        if let Ok(img) = cb.get_image() {
            if img.width > 0 && img.height > 0 {
                match save_clipboard_image_as_png(&img) {
                    Ok(path) => {
                        let display_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "clipboard.png".to_string());
                        app.chat_state.add_attachment(
                            path,
                            crate::panels::agent_chat::AttachmentKind::Image,
                        );
                        app.chat_state.add_system_message(&format!(
                            "Pasted clipboard image: {} ({}x{})",
                            display_name, img.width, img.height
                        ));
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to save clipboard image: {}", e);
                    }
                }
            }
        }
    }

    let text = app.get_clipboard();
    if !text.is_empty() {
        app.chat_state.insert_str(&text);
        app.refresh_chat_autocomplete();
    }
}

pub(super) fn cancel_agent(app: &mut App) {
    let conv_id = app.chat_state.conversations[app.chat_state.active_conv]
        .id
        .clone();
    if let Some(task) = app.acp_tasks.remove(&conv_id) {
        task.abort();
        app.chat_state.conversations[app.chat_state.active_conv].is_streaming = false;
        app.chat_state.conversations[app.chat_state.active_conv].streaming_started_at = None;
        app.chat_state.finalize_message("system", "Cancelled by user.");
    }
}
