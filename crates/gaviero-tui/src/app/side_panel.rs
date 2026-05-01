use super::*;

/// Build a unique display label per root. Starts from `display_name()`; if two
/// or more roots share that label, disambiguates by appending the parent dir
/// name, then by appending the absolute path. Empty label only if the root
/// has no usable name at all (single-folder fallback case).
fn unique_root_labels(roots: &[(String, std::path::PathBuf)]) -> Vec<String> {
    let mut labels: Vec<String> = roots.iter().map(|(n, _)| n.clone()).collect();
    loop {
        let mut counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for l in &labels {
            *counts.entry(l.as_str()).or_insert(0) += 1;
        }
        let dupes: std::collections::HashSet<String> = counts
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(l, _)| l.to_string())
            .collect();
        if dupes.is_empty() {
            break;
        }
        let mut changed = false;
        for (i, label) in labels.iter_mut().enumerate() {
            if !dupes.contains(label.as_str()) {
                continue;
            }
            let path = &roots[i].1;
            let parent = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str());
            if let Some(p) = parent.filter(|p| !p.is_empty()) {
                *label = format!("{}/{}", p, label);
                changed = true;
            } else {
                *label = path.to_string_lossy().to_string();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    labels
}

pub(super) fn handle_chat_action(app: &mut App, action: Action) {
    // Only clear the output text selection on non-selection keypresses.
    // SelectUp/SelectDown extend it; Copy reads it.
    let is_selection_action = matches!(
        action,
        Action::SelectUp
            | Action::SelectDown
            | Action::SelectLeft
            | Action::SelectRight
            | Action::SelectWordLeft
            | Action::SelectWordRight
            | Action::Copy
    );
    let had_output_selection = app.chat_state.has_text_selection();
    if !is_selection_action {
        app.chat_state.clear_text_selection();
    }

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
                } else if app
                    .chat_state
                    .text_input
                    .text
                    .trim()
                    .starts_with("/remember")
                {
                    app.handle_remember_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/attach") {
                    app.handle_attach_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/detach") {
                    app.handle_detach_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/run") {
                    app.handle_run_script_command();
                } else if app
                    .chat_state
                    .text_input
                    .text
                    .trim()
                    .starts_with("/reembed")
                {
                    app.handle_reembed_command();
                } else if app
                    .chat_state
                    .text_input
                    .text
                    .trim()
                    .starts_with("/consolidate-session")
                {
                    app.handle_consolidate_session_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/sleep") {
                    app.handle_sleep_command();
                } else if app.chat_state.text_input.text.trim().starts_with("/restore") {
                    app.handle_restore_command();
                } else if app
                    .chat_state
                    .text_input
                    .text
                    .trim()
                    .starts_with("/forget-history")
                {
                    app.handle_forget_history_command();
                } else if {
                    // Dispatch /forget* — `/forget-history` is captured
                    // by the prior arm so this one only sees the bulk
                    // soft-delete variants.
                    let t = app.chat_state.text_input.text.trim();
                    t.starts_with("/forget-scope")
                        || t.starts_with("/forget-type")
                        || t.starts_with("/forget-source")
                        || (t.starts_with("/forget") && !t.starts_with("/forget-history"))
                } {
                    app.handle_forget_command();
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
            } else if had_output_selection {
                // Copy the mouse/keyboard-selected text from the chat output
                if let Some(text) = app.chat_state.selected_chat_text() {
                    app.set_clipboard(&text);
                }
                app.chat_state.clear_text_selection();
            } else if app.chat_state.text_input.has_selection() {
                // Copy selected text from the input widget
                if let Some(text) = app.chat_state.text_input.selected_text() {
                    app.set_clipboard(&text.to_string());
                }
                app.chat_state.text_input.sel_anchor = None;
            } else {
                app.chat_state.enter_browse_mode();
            }
        }
        Action::SelectUp => {
            if !app.chat_state.active_conv_streaming() {
                app.chat_state.select_up_in_output();
            }
        }
        Action::SelectDown => {
            if !app.chat_state.active_conv_streaming() {
                app.chat_state.select_down_in_output();
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
                .map(|a| {
                    matches!(
                        a.status,
                        gaviero_core::swarm::models::AgentStatus::Completed
                    )
                })
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
                            let _ = tx
                                .send(Event::SwarmPhaseChanged(format!("revert panicked: {}", e)));
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
                dash.scroll.selected =
                    (dash.scroll.selected + 10).min(agent_count.saturating_sub(1));
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
                    if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                        if let Err(e) = entry.repo.checkout(&name) {
                            app.git_panel.error_message = Some(format!("{}", e));
                        }
                        app.git_panel.refresh(&entry.repo);
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
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    if let Err(e) = entry.repo.stage_file(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(&entry.repo);
                }
            }
        }
        Action::InsertChar('u') if app.git_panel.region != GitRegion::CommitInput => {
            if let Some(path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    if let Err(e) = entry.repo.unstage_file(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(&entry.repo);
                }
            }
        }
        Action::InsertChar('d') if app.git_panel.region != GitRegion::CommitInput => {
            if let Some(path) = app.git_panel.selected_path().map(|s| s.to_string()) {
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    if let Err(e) = entry.repo.discard_changes(&path) {
                        app.git_panel.error_message = Some(format!("{}", e));
                    }
                    app.git_panel.refresh(&entry.repo);
                    app.refresh_file_tree();
                }
            }
        }
        Action::InsertChar('c') if app.git_panel.region != GitRegion::CommitInput => {
            if !app.git_panel.commit_input.is_empty() {
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    match entry.repo.commit(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(&entry.repo);
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
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    match entry.repo.amend(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(&entry.repo);
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
                if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
                    match entry.repo.commit(&app.git_panel.commit_input.text) {
                        Ok(_) => {
                            app.git_panel.commit_input.clear();
                            app.git_panel.refresh(&entry.repo);
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
            open_selected_git_file(app);
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
    let entry = app.git_repos.get(app.git_panel.active_repo)?;
    entry.repo.head_file_content(rel_path).ok()
}

pub(super) fn refresh_git_panel(app: &mut App) {
    let names: Vec<String> = app.git_repos.iter().map(|e| e.name.clone()).collect();
    app.git_panel.set_repo_tabs(names);
    if let Some(entry) = app.git_repos.get(app.git_panel.active_repo) {
        app.git_panel.refresh(&entry.repo);
    }
}

/// Open a read-only diff-view tab for the file currently selected in the git
/// panel. If the file is unchanged vs HEAD, opens the file as a normal
/// editable buffer instead. Used by both the Enter key and mouse clicks.
pub(super) fn open_selected_git_file(app: &mut App) {
    let Some(rel_path) = app.git_panel.selected_path().map(|s| s.to_string()) else {
        return;
    };
    let Some(root) = app
        .git_repos
        .get(app.git_panel.active_repo)
        .and_then(|e| e.repo.workdir().map(|p| p.to_path_buf()))
    else {
        return;
    };
    let abs_path = root.join(&rel_path);
    if !abs_path.exists() {
        return;
    }
    let original = app.git_head_content(&rel_path).unwrap_or_default();
    let current = std::fs::read_to_string(&abs_path).unwrap_or_default();
    if original != current {
        crate::app::editing::open_diff_view(app, &abs_path, original, current);
    } else {
        app.open_file(&abs_path);
    }
    app.focus = Focus::Editor;
}

/// A4: handle a TUI action while the memory panel is focused.
///
/// Keybindings (plan §A4):
/// * `Tab` cycles section focus.
/// * `i` / `Esc` enter / leave inspect overlay (Injected Now).
/// * `h` / `Esc` enter / leave history overlay.
/// * `d`/`p`/`s`/`e` → delete / pin / scope-change / edit on Recently
///    Written (edit-text UX is kept minimal here: pop an inline text
///    input via `insert_char` in a follow-up change).
/// * `y`/`n` confirm / reject a pending delete.
/// * `/` activates search; printable chars type into the query; `Esc`
///    clears search.
pub(super) fn handle_memory_panel_action(app: &mut App, action: Action) {
    use crate::panels::memory_panel::{PanelPromptMode, PanelSection, ScopeChoice};
    use gaviero_core::memory::writer::PanelEditOp;

    // Inline `e` / `s` prompts hijack input until Enter / Esc.
    if !matches!(app.memory_panel.prompt_mode, PanelPromptMode::None) {
        match action {
            Action::Quit => {
                app.memory_panel.prompt_mode = PanelPromptMode::None;
            }
            Action::Enter => {
                commit_panel_prompt(app);
            }
            Action::InsertChar(ch) => match &mut app.memory_panel.prompt_mode {
                PanelPromptMode::EditText { buffer, .. } => buffer.push(ch),
                PanelPromptMode::SetScope { .. } => {}
                PanelPromptMode::None => {}
            },
            Action::Backspace => {
                if let PanelPromptMode::EditText { buffer, .. } = &mut app.memory_panel.prompt_mode
                {
                    buffer.pop();
                }
            }
            Action::CursorLeft => {
                if let PanelPromptMode::SetScope { selected, .. } =
                    &mut app.memory_panel.prompt_mode
                {
                    *selected = selected.prev();
                }
            }
            Action::CursorRight => {
                if let PanelPromptMode::SetScope { selected, .. } =
                    &mut app.memory_panel.prompt_mode
                {
                    *selected = selected.next();
                }
            }
            _ => {}
        }
        return;
    }

    // Inspect / history overlays consume Esc / arrow keys.
    if app.memory_panel.inspecting {
        match action {
            Action::Quit => app.memory_panel.inspecting = false,
            _ => {}
        }
        return;
    }
    if app.memory_panel.history_mode {
        match action {
            Action::Quit => app.memory_panel.history_mode = false,
            Action::CursorUp => {
                app.memory_panel.history_cursor = app.memory_panel.history_cursor.saturating_sub(1);
            }
            Action::CursorDown => {
                let max = app.memory_panel.history_rows.len().saturating_sub(1);
                app.memory_panel.history_cursor = (app.memory_panel.history_cursor + 1).min(max);
            }
            Action::Enter => {
                // Load the selected historical manifest as the
                // "current" one so the main view reflects that turn.
                if let Some(row) = app
                    .memory_panel
                    .history_rows
                    .get(app.memory_panel.history_cursor)
                    .cloned()
                {
                    app.memory_panel.current_manifest = Some(row);
                    app.memory_panel.manifest_selected_items.clear();
                    app.memory_panel.history_mode = false;
                    refresh_manifest_selected_items(app);
                }
            }
            _ => {}
        }
        return;
    }

    // Pending delete confirmation short-circuits.
    if let Some(memory_id) = app.memory_panel.confirm_delete_id {
        match action {
            Action::InsertChar('y') | Action::InsertChar('Y') => {
                app.memory_panel.confirm_delete_id = None;
                if let Some(writer) = app.memory_writer.as_ref().cloned() {
                    tokio::spawn(async move {
                        let _ = writer.enqueue(gaviero_core::memory::WriterMessage::PanelEdit {
                            op: PanelEditOp::Delete { memory_id },
                            ack: None,
                        });
                    });
                }
            }
            Action::InsertChar('n') | Action::InsertChar('N') | Action::Quit => {
                app.memory_panel.confirm_delete_id = None;
            }
            _ => {}
        }
        return;
    }

    // Live-search typing intercepts printable chars.
    if app.memory_panel.search_active {
        match action {
            Action::InsertChar(ch) => {
                app.memory_panel.search_query.push(ch);
                schedule_memory_panel_search(app);
            }
            Action::Backspace => {
                app.memory_panel.search_query.pop();
                schedule_memory_panel_search(app);
            }
            Action::Quit => {
                app.memory_panel.search_active = false;
                app.memory_panel.search_query.clear();
                app.memory_panel.search_results.clear();
            }
            Action::CursorDown => {
                let max = app.memory_panel.search_results.len().saturating_sub(1);
                app.memory_panel.search_cursor = (app.memory_panel.search_cursor + 1).min(max);
            }
            Action::CursorUp => {
                app.memory_panel.search_cursor = app.memory_panel.search_cursor.saturating_sub(1);
            }
            _ => {}
        }
        return;
    }

    // C1.5: destructive keys (`d` delete, `e` edit text, `p` pin, `s`
    // change scope) are vetoed when the History tab is active. The
    // writer task (C1.2) and the SQL trigger (C1.3) are the load-
    // bearing defenses; this guard is UX polish — surface "history is
    // read-only" before the user spends a confirm-keystroke.
    if app.memory_panel.history_tab_active() {
        let blocked = matches!(
            action,
            Action::InsertChar('d')
                | Action::InsertChar('e')
                | Action::InsertChar('p')
                | Action::InsertChar('s')
        );
        if blocked {
            app.memory_panel.last_error = Some((
                "history is read-only — use /forget-history to redact".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
    }

    // C2.6: Deletions tab is read-only too — d/e/p/s would target
    // already-deleted rows. Block with a hint.
    if app.memory_panel.viewing_deletions {
        let blocked = matches!(
            action,
            Action::InsertChar('d')
                | Action::InsertChar('e')
                | Action::InsertChar('p')
                | Action::InsertChar('s')
        );
        if blocked {
            app.memory_panel.last_error = Some((
                "deletions tab is read-only — use `u` to restore".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
    }

    match action {
        Action::Tab => app.memory_panel.focus_next(),
        Action::CycleTabForward => {
            let to_deletions = app.memory_panel.cycle_kind_tab(true);
            if to_deletions {
                refresh_deletions_rows(app);
            } else {
                refresh_recent_rows_for_kind(app);
            }
        }
        Action::CycleTabBack => {
            let to_deletions = app.memory_panel.cycle_kind_tab(false);
            if to_deletions {
                refresh_deletions_rows(app);
            } else {
                refresh_recent_rows_for_kind(app);
            }
        }
        Action::CursorDown => cursor_down_in_focus(app),
        Action::CursorUp => cursor_up_in_focus(app),
        // C1.5: kind-tab switching. Refresh of `recent_rows` is
        // triggered by the next observer event; for an immediate
        // refresh users can press `r` (existing reload) or wait for
        // the standard 100ms refresh debounce.
        Action::InsertChar('1') => {
            let left = app.memory_panel.leave_deletions_tab();
            if app
                .memory_panel
                .set_active_kind(gaviero_core::memory::MemoryKind::Record)
                || left
            {
                refresh_recent_rows_for_kind(app);
            }
        }
        Action::InsertChar('2') => {
            let left = app.memory_panel.leave_deletions_tab();
            if app
                .memory_panel
                .set_active_kind(gaviero_core::memory::MemoryKind::History)
                || left
            {
                refresh_recent_rows_for_kind(app);
            }
        }
        Action::InsertChar('3') => {
            let left = app.memory_panel.leave_deletions_tab();
            if app
                .memory_panel
                .set_active_kind(gaviero_core::memory::MemoryKind::Summary)
                || left
            {
                refresh_recent_rows_for_kind(app);
            }
        }
        // C2.6: Deletions tab. Loads the audit log and routes `u`
        // through WriterMessage::Restore. Redactions remain visible
        // but `u` is a no-op for them (with a hint).
        Action::InsertChar('4') => {
            if app.memory_panel.enter_deletions_tab() {
                refresh_deletions_rows(app);
            }
        }
        Action::InsertChar('u')
            if app.memory_panel.viewing_deletions
                && app.memory_panel.focused == PanelSection::RecentlyWritten =>
        {
            let Some(row) = app.memory_panel.deletion_under_cursor().cloned() else {
                return;
            };
            if !row.restorable {
                app.memory_panel.last_error = Some((
                    "redactions are permanent — `u` cannot restore user_redaction rows"
                        .to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
            let Some(writer) = app.memory_writer.clone() else {
                return;
            };
            let tx = app.event_tx.clone();
            tokio::spawn(async move {
                let body = match writer.restore_deletion(row.id).await {
                    Ok(outcome) => match outcome {
                        gaviero_core::memory::RestoreOutcome::Inserted {
                            new_memory_id, ..
                        } => format!("✓ restored audit {} as memory {}", row.id, new_memory_id),
                        gaviero_core::memory::RestoreOutcome::Deduplicated {
                            surviving_memory_id,
                            ..
                        } => format!(
                            "✓ audit {} merged into existing memory {}",
                            row.id, surviving_memory_id
                        ),
                        gaviero_core::memory::RestoreOutcome::AlreadyCovered { .. } => {
                            format!("✓ audit {} covered at a broader scope", row.id)
                        }
                        gaviero_core::memory::RestoreOutcome::Refused { reason, .. } => {
                            format!("✗ restore refused: {reason}")
                        }
                    },
                    Err(e) => format!("restore failed: {e}"),
                };
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: body,
                });
            });
        }
        Action::InsertChar('i') if app.memory_panel.focused == PanelSection::InjectedNow => {
            app.memory_panel.load_inspect_pool();
            app.memory_panel.inspecting = true;
        }
        Action::InsertChar('h') if app.memory_panel.focused == PanelSection::InjectedNow => {
            load_memory_panel_history(app);
            app.memory_panel.history_mode = true;
        }
        Action::InsertChar('d') if app.memory_panel.focused == PanelSection::RecentlyWritten => {
            if let Some(row) = app
                .memory_panel
                .recent_rows
                .get(app.memory_panel.recent_cursor)
            {
                app.memory_panel.confirm_delete_id = Some(row.id);
            }
        }
        Action::InsertChar('p') if app.memory_panel.focused == PanelSection::RecentlyWritten => {
            if let (Some(row), Some(writer)) = (
                app.memory_panel
                    .recent_rows
                    .get(app.memory_panel.recent_cursor)
                    .cloned(),
                app.memory_writer.clone(),
            ) {
                tokio::spawn(async move {
                    let _ = writer.enqueue(gaviero_core::memory::WriterMessage::PanelEdit {
                        op: PanelEditOp::Pin {
                            memory_id: row.id,
                            trust_score: 1.0,
                        },
                        ack: None,
                    });
                });
            }
        }
        Action::InsertChar('e') if app.memory_panel.focused == PanelSection::RecentlyWritten => {
            if let Some(row) = app
                .memory_panel
                .recent_rows
                .get(app.memory_panel.recent_cursor)
                .cloned()
            {
                app.memory_panel.prompt_mode = PanelPromptMode::EditText {
                    memory_id: row.id,
                    buffer: row.text,
                };
            }
        }
        Action::InsertChar('s') if app.memory_panel.focused == PanelSection::RecentlyWritten => {
            if let Some(row) = app
                .memory_panel
                .recent_rows
                .get(app.memory_panel.recent_cursor)
                .cloned()
            {
                let current = match row.scope_level {
                    0 => ScopeChoice::Global,
                    1 => ScopeChoice::Workspace,
                    2 => ScopeChoice::Repo,
                    3 => ScopeChoice::Module,
                    _ => ScopeChoice::Run,
                };
                app.memory_panel.prompt_mode = PanelPromptMode::SetScope {
                    memory_id: row.id,
                    selected: current,
                };
            }
        }
        Action::InsertChar('/') => {
            app.memory_panel.focused = PanelSection::Search;
            app.memory_panel.search_active = true;
            app.memory_panel.search_query.clear();
            app.memory_panel.search_results.clear();
        }
        _ => {}
    }
}

/// Commit the current `PanelPromptMode` — fires the appropriate
/// `PanelEditOp` through the writer and clears the mode. Invoked on
/// `Enter` from the inline prompt.
fn commit_panel_prompt(app: &mut App) {
    use crate::panels::memory_panel::{PanelPromptMode, ScopeChoice};
    use gaviero_core::memory::writer::PanelEditOp;
    use gaviero_core::memory::{WriteScope, hash_path};

    let mode = std::mem::replace(&mut app.memory_panel.prompt_mode, PanelPromptMode::None);
    let writer = match app.memory_writer.clone() {
        Some(w) => w,
        None => return,
    };

    match mode {
        PanelPromptMode::EditText { memory_id, buffer } => {
            let trimmed = buffer.trim().to_string();
            if trimmed.is_empty() {
                return;
            }
            tokio::spawn(async move {
                let _ = writer.enqueue(gaviero_core::memory::WriterMessage::PanelEdit {
                    op: PanelEditOp::UpdateText {
                        memory_id,
                        new_text: trimmed,
                    },
                    ack: None,
                });
            });
        }
        PanelPromptMode::SetScope {
            memory_id,
            selected,
        } => {
            let workspace_root = match app.workspace.roots().first().cloned() {
                Some(r) => r,
                None => return,
            };
            let repo_id = hash_path(&workspace_root);
            let new_scope = match selected {
                ScopeChoice::Global => WriteScope::Global,
                ScopeChoice::Workspace => WriteScope::Workspace,
                ScopeChoice::Repo => WriteScope::Repo { repo_id },
                ScopeChoice::Module => {
                    let module_path = app
                        .buffers
                        .get(app.active_buffer)
                        .and_then(|b| b.path.as_ref())
                        .and_then(|p| p.strip_prefix(&workspace_root).ok())
                        .and_then(|rel| rel.parent().map(|p| p.to_string_lossy().to_string()))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "".to_string());
                    if module_path.is_empty() {
                        WriteScope::Repo { repo_id }
                    } else {
                        WriteScope::Module {
                            repo_id,
                            module_path,
                        }
                    }
                }
                ScopeChoice::Run => {
                    let run_id = app.chat_state.active_conversation_id().to_string();
                    WriteScope::Run { repo_id, run_id }
                }
            };
            tokio::spawn(async move {
                let _ = writer.enqueue(gaviero_core::memory::WriterMessage::PanelEdit {
                    op: PanelEditOp::SetScope {
                        memory_id,
                        new_scope,
                    },
                    ack: None,
                });
            });
        }
        PanelPromptMode::None => {}
    }
}

fn cursor_down_in_focus(app: &mut App) {
    use crate::panels::memory_panel::PanelSection;
    match app.memory_panel.focused {
        PanelSection::InjectedNow => {
            let max = app
                .memory_panel
                .manifest_selected_items
                .len()
                .saturating_sub(1);
            app.memory_panel.injected_cursor = (app.memory_panel.injected_cursor + 1).min(max);
        }
        PanelSection::RecentlyWritten => {
            if app.memory_panel.viewing_deletions {
                let max = app.memory_panel.deletions_rows.len().saturating_sub(1);
                app.memory_panel.deletions_cursor =
                    (app.memory_panel.deletions_cursor + 1).min(max);
            } else {
                let max = app.memory_panel.recent_rows.len().saturating_sub(1);
                app.memory_panel.recent_cursor = (app.memory_panel.recent_cursor + 1).min(max);
            }
        }
        _ => {}
    }
}

fn cursor_up_in_focus(app: &mut App) {
    use crate::panels::memory_panel::PanelSection;
    match app.memory_panel.focused {
        PanelSection::InjectedNow => {
            app.memory_panel.injected_cursor = app.memory_panel.injected_cursor.saturating_sub(1);
        }
        PanelSection::RecentlyWritten => {
            if app.memory_panel.viewing_deletions {
                app.memory_panel.deletions_cursor =
                    app.memory_panel.deletions_cursor.saturating_sub(1);
            } else {
                app.memory_panel.recent_cursor = app.memory_panel.recent_cursor.saturating_sub(1);
            }
        }
        _ => {}
    }
}

/// Bootstrap fill the memory panel from `memory.db` when it first
/// opens (or after a TUI restart). Fires three async queries — recent
/// writes, scope summary, and the most recent manifest — and streams
/// results back via existing Event::Memory* variants.
pub(super) fn refresh_memory_panel(app: &mut App) {
    let Some(mem) = app.memory.clone() else {
        return;
    };
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        // Most recent manifest (across all sessions).
        if let Ok(rows) = mem.workspace().recent_manifests(1).await {
            if let Some(row) = rows.into_iter().next() {
                let _ = tx.send(crate::event::Event::MemoryManifestPersisted {
                    turn_id: row.turn_id,
                    session_id: row.session_id,
                });
            }
        }
        // Recently-written seed — piggy-back through MemoryWriteCommitted
        // so the controller runs the same refresh path.
        let _ = tx.send(crate::event::Event::MemoryWriteCommitted {
            kind: "PanelBootstrap".to_string(),
        });
    });
}

/// Fire a debounced live-search against `MemoryStore::search_scoped`.
fn schedule_memory_panel_search(app: &mut App) {
    use crate::panels::memory_panel::SEARCH_DEBOUNCE;
    let now = std::time::Instant::now();
    if let Some(prev) = app.memory_panel.search_last_run {
        if now.duration_since(prev) < SEARCH_DEBOUNCE {
            // Too soon — let the timer-driven re-invocation catch up.
            // The actual run happens below anyway; keeping a simple
            // eager path since typing cadence for humans is typically
            // < 1 event per 150ms.
        }
    }
    app.memory_panel.search_last_run = Some(now);

    let Some(mem) = app.memory.clone() else {
        return;
    };
    let query = app.memory_panel.search_query.clone();
    if query.trim().is_empty() {
        app.memory_panel.search_results.clear();
        return;
    }
    let workspace_root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let retrieval_cfg = app
        .workspace
        .resolve_retrieval_config(Some(&workspace_root));
    let rerank_cfg = app.workspace.resolve_rerank_config(Some(&workspace_root));
    let reranker = app.memory_reranker.clone();
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        // Memory panel search has no active-file context →
        // folder = None. Registry walks workspace + global only.
        let scope =
            gaviero_core::memory::MemoryScope::from_context(&workspace_root, None, None, None);
        let reranker_ref: Option<&dyn gaviero_core::memory::Reranker> = reranker.as_deref();
        let out = gaviero_core::memory::retrieve_ranked(
            &mem,
            &scope,
            &query,
            5,
            &retrieval_cfg,
            reranker_ref,
            Some(&rerank_cfg),
        )
        .await;
        if let Ok(out) = out {
            let rows: Vec<crate::panels::memory_panel::MemoryRow> = out
                .items
                .iter()
                .map(crate::panels::memory_panel::MemoryRow::from_scored)
                .collect();
            let _ = tx.send(crate::event::Event::MemorySearchResults { rows });
        }
    });
}

/// C1.5: trigger a fresh `recent_memories_by_kind` query when the
/// user switches kind tabs. Side-effecting: spawns a tokio task that
/// queries and posts a `MemorySearchResults` event back to the main
/// loop. The standard observer-driven refresh (in `controller.rs`)
/// also picks up the new active_kind on the next memory-write event;
/// this helper just makes the tab switch feel instant.
fn refresh_recent_rows_for_kind(app: &mut App) {
    let Some(mem) = app.memory.clone() else {
        return;
    };
    let active_kind = app.memory_panel.active_kind;
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        if let Ok(rows) = mem
            .workspace()
            .recent_memories_by_kind(active_kind, 24, 50)
            .await
        {
            let panel_rows: Vec<crate::panels::memory_panel::MemoryRow> = rows
                .iter()
                .map(crate::panels::memory_panel::MemoryRow::from_scored)
                .collect();
            let _ = tx.send(crate::event::Event::MemorySearchResults { rows: panel_rows });
        }
    });
}

/// C2.6: load the most recent N audit rows into the Deletions tab.
/// Spawned as a tokio task so the TUI stays responsive; the result
/// arrives via `Event::MemoryDeletionsLoaded`.
fn refresh_deletions_rows(app: &mut App) {
    let Some(mem) = app.memory.clone() else {
        return;
    };
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        if let Ok(rows) = mem.workspace().recent_deletions(50).await {
            let panel_rows: Vec<crate::panels::memory_panel::DeletionRow> = rows
                .into_iter()
                .map(|d| {
                    let restorable = d.is_restorable();
                    crate::panels::memory_panel::DeletionRow {
                        id: d.id,
                        memory_id: d.memory_id,
                        memory_kind: d.memory_kind,
                        memory_source: d.memory_source,
                        deleted_at: d.deleted_at,
                        deleted_by: d.deleted_by,
                        reason: d.reason,
                        restorable,
                    }
                })
                .collect();
            let _ = tx.send(crate::event::Event::MemoryDeletionsLoaded { rows: panel_rows });
        }
    });
}

/// Load the last 30 manifests (any session) for the history overlay.
fn load_memory_panel_history(app: &mut App) {
    let Some(mem) = app.memory.clone() else {
        return;
    };
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        if let Ok(rows) = mem.workspace().recent_manifests(30).await {
            let _ = tx.send(crate::event::Event::MemoryHistoryRows { rows });
        }
    });
}

/// Re-query the selected memory items of the current manifest — called
/// after a `MemoryManifestPersisted` event or when the user opens a
/// historical manifest from the history overlay.
pub(super) fn refresh_manifest_selected_items(app: &mut App) {
    let Some(manifest) = app.memory_panel.current_manifest.clone() else {
        return;
    };
    let Some(mem) = app.memory.clone() else {
        return;
    };
    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        // Parse `selected_ids` out of the payload.
        let ids: Vec<i64> = serde_json::from_str::<serde_json::Value>(&manifest.payload)
            .ok()
            .and_then(|v| {
                v.get("selected_ids")
                    .and_then(|a| a.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_i64()).collect())
            })
            .unwrap_or_default();

        // Resolve each id via a lightweight lookup. No search needed —
        // load via `recent_memories` and filter. This is coarser than
        // ideal but sidesteps adding a bulk-get-by-id API for MVP.
        if ids.is_empty() {
            let _ = tx.send(crate::event::Event::MemorySelectedItems { rows: Vec::new() });
            return;
        }
        let pool = mem
            .workspace()
            .recent_memories(24 * 7, 500)
            .await
            .unwrap_or_default();
        let rows: Vec<crate::panels::memory_panel::MemoryRow> = pool
            .iter()
            .filter(|m| ids.contains(&m.id))
            .map(crate::panels::memory_panel::MemoryRow::from_scored)
            .collect();
        let _ = tx.send(crate::event::Event::MemorySelectedItems { rows });
    });
}

pub(super) fn refresh_chat_autocomplete(app: &mut App) {
    if !app.chat_state.autocomplete.active {
        return;
    }

    let at_pos = app.chat_state.autocomplete.at_pos;
    let is_run_path_context = {
        let text = &app.chat_state.text_input.text;
        at_pos <= text.len() && text[..at_pos].trim() == "/run"
    };

    let folders = app.workspace.folders();
    let roots: Vec<(String, std::path::PathBuf)> = if folders.is_empty() {
        vec![(String::new(), std::path::PathBuf::from("."))]
    } else {
        folders
            .iter()
            .map(|f| (f.display_name().to_string(), f.path.clone()))
            .collect()
    };
    let multi_root = roots.len() > 1;
    let labels = unique_root_labels(&roots);

    const TOTAL_LIMIT: usize = 10_000;
    let per_root = TOTAL_LIMIT / roots.len().max(1);
    let mut seen = std::collections::HashSet::new();
    let mut files: Vec<String> = Vec::new();
    for ((_, root), label) in roots.iter().zip(labels.iter()) {
        let excludes = parse_exclude_patterns(&app.workspace, Some(root));
        for f in list_workspace_files(root, per_root, &excludes) {
            let display = if multi_root && !label.is_empty() {
                format!("{}/{}", label, f)
            } else {
                f
            };
            if seen.insert(display.clone()) {
                files.push(display);
            }
        }
    }

    let files: Vec<String> = files
        .into_iter()
        .filter(|f| !is_run_path_context || f.ends_with(".gaviero"))
        .collect();

    app.chat_state.update_autocomplete_matches(&files);
}

pub(super) fn send_chat_message(app: &mut App) {
    let conv_id = app.chat_state.active_conversation_id().to_string();
    let prompt = app.chat_state.take_input();
    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let turn_id = format!(
        "{}-{}",
        conv_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let pending_module_path = app
        .buffers
        .get(app.active_buffer)
        .and_then(|b| b.path.as_deref())
        .and_then(|p| p.strip_prefix(&root).ok())
        .and_then(|rel| rel.parent())
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| !p.is_empty());

    app.chat_state.add_user_message(&prompt);
    {
        let conv = app.chat_state.active_conversation_mut();
        conv.pending_turn_id = Some(turn_id.clone());
        conv.pending_module_path = pending_module_path;
        conv.is_streaming = true;
        conv.streaming_status = "Connecting...".to_string();
        conv.streaming_started_at = Some(std::time::Instant::now());
    }

    let tx = app.event_tx.clone();
    let wg = app.write_gate.clone();

    let named_roots: Vec<(String, std::path::PathBuf)> = {
        let folders = app.workspace.folders();
        if folders.is_empty() {
            vec![(String::new(), root.clone())]
        } else {
            folders
                .iter()
                .map(|f| (f.display_name().to_string(), f.path.clone()))
                .collect()
        }
    };
    let multi_root = named_roots.len() > 1;
    let labels = unique_root_labels(&named_roots);
    let refs = crate::panels::agent_chat::parse_file_references(&prompt);
    let mut file_refs: Vec<(String, String)> = Vec::new();
    for rel_path in &refs {
        // If multi-root and the ref starts with "<label>/", resolve it to that root only.
        // Labels can themselves contain '/', so we match the longest label first.
        let mut resolved = false;
        if multi_root {
            let mut idxs: Vec<usize> = (0..labels.len()).collect();
            idxs.sort_by_key(|&i| std::cmp::Reverse(labels[i].len()));
            for i in idxs {
                let label = &labels[i];
                if label.is_empty() {
                    continue;
                }
                if let Some(tail) = rel_path
                    .strip_prefix(label)
                    .and_then(|t| t.strip_prefix('/'))
                {
                    let abs_path = named_roots[i].1.join(tail);
                    if let Ok(content) = std::fs::read_to_string(&abs_path) {
                        file_refs.push((rel_path.clone(), content));
                        resolved = true;
                        break;
                    }
                }
            }
        }
        if !resolved {
            for (_, r) in &named_roots {
                let abs_path = r.join(rel_path);
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    file_refs.push((rel_path.clone(), content));
                    break;
                }
            }
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

    // Claude session reuse: on the first turn of a conversation,
    // `resume_session_id` is None and we send bootstrap context (graph +
    // memory + prior history). Claude replies with a `SystemInit` event
    // carrying a fresh session id, which the controller stores on the
    // `Conversation`. On every subsequent turn we pass that id back via
    // `--resume` and send only the new user message — Claude retains
    // history server-side, eliminating per-turn tempfile bloat.
    let model = app.chat_state.effective_model().to_string();

    // M1: lazy-init the per-conversation SessionLedger now that we know the
    // model (needed by the ProviderProfile factory). The ledger is the
    // canonical first-turn check (V9 §11 M1: "Ledger replaces ad-hoc
    // is_first_turn logic"). Today's `claude_session_id.is_none()` and
    // `ledger.is_first_turn()` are equivalent — the M4 invalidation logic
    // makes them diverge meaningfully when persisted state goes stale.
    let runtime = gaviero_core::context_planner::RuntimeConfig {
        ollama_base_url: Some(app.chat_state.agent_settings.ollama_base_url.clone()),
    };
    let provider_profile = gaviero_core::context_planner::build_provider_profile(
        &gaviero_core::context_planner::ModelSpec::parse(&model),
        &runtime,
    );
    {
        let conv = app.chat_state.active_conversation_mut();
        let current_fp =
            gaviero_core::context_planner::PlannerFingerprint::from_profile(&provider_profile);
        // Step 1: rehydrate or lazy-init the ledger.
        if conv.session_ledger.is_none() {
            // M4: if this is the first send after restore from disk,
            // rehydrate the ledger from persisted state. Otherwise start a
            // fresh ledger seeded from the current profile.
            let ledger = match conv.pending_persisted_ledger.take() {
                Some(persisted) => gaviero_core::context_planner::SessionLedger::from_persisted(
                    persisted,
                    &provider_profile,
                ),
                None => gaviero_core::context_planner::SessionLedger::new(
                    &provider_profile,
                    current_fp.clone(),
                ),
            };
            conv.session_ledger = Some(ledger);
        }
        // Step 2: run the fingerprint check on EVERY send, not only on
        // lazy-init. The model can change mid-conversation via `/model`,
        // which must invalidate the handle so haiku doesn't silently
        // resume a session Claude opened under sonnet. V9 §11 M4
        // acceptance: "Model change invalidates stored handle".
        if let Some(ref mut ledger) = conv.session_ledger {
            if ledger.invalidate_if_fingerprint_changed(&current_fp) {
                // Also drop the legacy `claude_session_id` mirror so the
                // next turn passes no `--resume` flag.
                conv.claude_session_id = None;
            }
        }
    }
    // Read the (possibly invalidated) legacy handle AFTER the lazy-init
    // block above — `invalidate_if_fingerprint_changed` may have cleared
    // it when the model changed since save. Reading earlier would leak
    // the stale id into `AgentOptions::resume_session_id` and Claude
    // would silently accept the old session, defeating the invalidation.
    let resume_session_id = app
        .chat_state
        .active_conversation()
        .claude_session_id
        .clone();
    let is_first_turn = app
        .chat_state
        .active_conversation()
        .session_ledger
        .as_ref()
        .map(|l| l.is_first_turn())
        .unwrap_or(true);

    // Conversation history is only inlined on the first turn AND when the
    // conversation's `transcript_inline_mode` allows it. On resumed turns
    // Claude already has the history and re-sending wastes tokens + risks
    // Claude's Read-tool size limits on the prompt tempfile. After
    // /reset (Suppress) the user explicitly asked to start fresh, so the
    // visible transcript stays in the panel but does not re-enter the prompt.
    let inline_mode = app
        .chat_state
        .active_conversation()
        .transcript_inline_mode;
    let inline_transcript = match inline_mode {
        crate::panels::agent_chat::TranscriptInlineMode::Auto => is_first_turn,
        crate::panels::agent_chat::TranscriptInlineMode::Suppress => false,
        crate::panels::agent_chat::TranscriptInlineMode::Force => true,
    };
    let context: Vec<(String, String)> = if inline_transcript {
        app.chat_state
            .context_messages()
            .into_iter()
            .rev()
            .skip(1)
            .rev()
            .map(|(r, c)| (r.to_string(), c.to_string()))
            .collect()
    } else {
        Vec::new()
    };

    let effort = app.chat_state.effective_effort().to_string();
    let max_tokens = app.chat_state.agent_settings.max_tokens;
    let auto_approve = app.chat_state.effective_auto_approve();
    app.chat_state.auto_approve_next = false;

    let (agent_available_tools, agent_approved_tools) =
        app.workspace.resolve_agent_tools(Some(&root));

    // M6: `resume_session_id` deprecated; ClaudeSession reads it from
    // `ContinuityHandle` instead. This construction site feeds
    // `LegacyAgentSession` (Ollama/Codex) and `ClaudeSession` both;
    // `ClaudeSession::new` reads the field to initialize its handle.
    // Allow stays until M10 removes the field.
    #[allow(deprecated)]
    let options = gaviero_core::acp::session::AgentOptions {
        effort,
        max_tokens,
        auto_approve,
        available_tools: Some(agent_available_tools),
        approved_tools: Some(agent_approved_tools),
        resume_session_id,
        ..gaviero_core::acp::session::AgentOptions::default()
    };

    let memory = app.memory.clone();
    let memory_writer = app.memory_writer.clone();
    // B2: clone the reranker handle + cfg so the spawned future can
    // pass them to `retrieve_for_chat_with_reranker` without holding
    // an `&App` reference across `.await`.
    let memory_reranker = app.memory_reranker.clone();
    let memory_rerank_cfg = app.memory_rerank_cfg.clone();
    let chat_injection_config = app.workspace.resolve_chat_injection_config(Some(&root));
    let retrieval_cfg = app.workspace.resolve_retrieval_config(Some(&root));
    let manifests_enabled = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_MANIFESTS_ENABLED,
            Some(&root),
        )
        .as_bool()
        .unwrap_or(true);
    let capture_candidate_pool = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_MANIFESTS_CAPTURE_POOL,
            Some(&root),
        )
        .as_bool()
        .unwrap_or(true);
    // B2: pull embedder + reranker names so the manifest reflects what
    // produced the candidate pool. Stale only when the user changes
    // settings mid-session — the manifest still records the *old* name
    // for that turn, which is correct.
    let embedder_name = app
        .memory
        .as_ref()
        .map(|m| m.embedder().name().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let reranker_name = if app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_RERANKER_ENABLED,
            Some(&root),
        )
        .as_bool()
        .unwrap_or(false)
    {
        Some(
            app.workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
                    Some(&root),
                )
                .as_str()
                .unwrap_or("none")
                .to_string(),
        )
    } else {
        None
    };
    let read_ns = app.chat_state.agent_settings.read_namespaces.clone();
    let ollama_base_url = app.chat_state.agent_settings.ollama_base_url.clone();
    let graph_budget_tokens = app.chat_state.agent_settings.graph_budget_tokens;
    let repo_map_cache = app.repo_map.clone();
    let graph_root = app
        .graph_workspace_root
        .clone()
        .unwrap_or_else(|| root.clone());

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

    // Snapshot the ledger for the spawn task. The planner only reads it in
    // M2 (no mutation), so a clone is safe. The canonical ledger lives on
    // the Conversation and is updated by the SystemInit event handler in
    // the controller — same lifecycle as `claude_session_id`.
    let ledger_snapshot = app
        .chat_state
        .active_conversation()
        .session_ledger
        .clone()
        .expect("session_ledger initialized above");
    let provider_profile_clone = provider_profile.clone();

    let conv_id_clone = conv_id.clone();
    let turn_id_clone = turn_id.clone();
    let task = tokio::spawn(async move {
        {
            let mut gate = wg.lock().await;
            tracing::info!(
                "Write gate mode before: {:?}, switching to Deferred",
                gate.mode()
            );
            gate.set_mode(WriteMode::Deferred);
        }

        // M2: Chat hands the planner the raw memory + repo_map references
        // and lets it own bootstrap policy. The TUI no longer decides what
        // to inject — it only acquires the resources and pre-computes
        // impact text (which lives in a `!Send` GraphStore, so it must be
        // built inside spawn_blocking; M3 wires `graph_store` into the
        // planner directly and removes this carrier).
        //
        // Follow-up turns: the planner's `is_first_turn()` check returns
        // false (the controller's SystemInit handler bumps `turn_count`
        // after Claude acknowledges turn 1), so memory + graph selections
        // come back empty and `render_chat_selections` emits just the
        // user message (V9 §11 M2 acceptance: "turn 2+ transmits only
        // new user message").
        let repo_map_arc = if is_first_turn {
            crate::app::session::get_or_build_repo_map_cached(repo_map_cache, graph_root.clone())
                .await
        } else {
            None
        };
        let impact_text = if is_first_turn {
            crate::app::session::compute_impact_text(graph_root.clone(), graph_seeds.clone()).await
        } else {
            None
        };

        let seed_paths_buf: Vec<std::path::PathBuf> = if is_first_turn {
            graph_seeds.iter().map(std::path::PathBuf::from).collect()
        } else {
            Vec::new()
        };
        let read_ns_for_planner: &[String] = if is_first_turn { &read_ns } else { &[] };
        let budget_for_planner: usize = if is_first_turn {
            graph_budget_tokens
        } else {
            0
        };

        let mut local_ledger = ledger_snapshot;
        let planner_input = gaviero_core::context_planner::PlannerInput {
            user_message: &prompt,
            explicit_refs: &[],
            seed_paths: &seed_paths_buf,
            provider_profile: &provider_profile_clone,
            read_namespaces: read_ns_for_planner,
            graph_budget_tokens: budget_for_planner,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: impact_text.as_deref(),
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
        };

        let selections = {
            let mut planner = gaviero_core::context_planner::ContextPlanner {
                memory: memory.as_ref(),
                repo_map: repo_map_arc.as_deref(),
                ledger: &mut local_ledger,
                workspace_root: &graph_root,
            };
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                planner.plan(&planner_input),
            )
            .await
            {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::error!("planner error: {}", e);
                    gaviero_core::context_planner::PlannerSelections::default()
                }
                Err(_) => {
                    tracing::warn!(
                        "Planner timed out after 5s, proceeding without bootstrap context"
                    );
                    gaviero_core::context_planner::PlannerSelections::default()
                }
            }
        };

        // Tier S / S1 + S4: chat memory auto-injection and manifest
        // persistence. Both are owned by `context_planner::chat_memory`
        // so CLI / headless callers reach the same path. The TUI
        // contributes only TUI-specific bits: emitting the
        // `ChatMemoryInjected` event for the panel and supplying
        // workspace-resolved configs.
        let chat_injection: Option<gaviero_core::memory::ChatInjection> =
            if let Some(mem) = memory.as_ref() {
                let reranker_ref: Option<&dyn gaviero_core::memory::Reranker> =
                    memory_reranker.as_deref();
                let outcome = gaviero_core::context_planner::perform_injection(
                    gaviero_core::context_planner::ChatMemoryRequest {
                        stores: mem,
                        writer: memory_writer.as_ref(),
                        workspace_root: &graph_root,
                        folder_root: None,
                        user_prompt: &prompt,
                        turn_id: &turn_id_clone,
                        session_id: &conv_id_clone,
                        injection_config: &chat_injection_config,
                        retrieval_config: &retrieval_cfg,
                        reranker: reranker_ref,
                        rerank_config: memory_rerank_cfg.as_ref(),
                        manifests_enabled,
                        capture_candidate_pool,
                        embedder_name: &embedder_name,
                        reranker_name: reranker_name.as_deref(),
                    },
                )
                .await;
                let _ = tx.send(Event::ChatMemoryInjected {
                    conv_id: conv_id_clone.clone(),
                    items_injected: outcome.summary.items_injected,
                    pool_size: outcome.summary.pool_size,
                    tokens_used: outcome.summary.tokens_used,
                    token_budget: outcome.summary.token_budget,
                });
                outcome.injection
            } else {
                None
            };

        // V9 §11 M5: lift `PlannerSelections` into a transport `Turn` and
        // dispatch through `AgentSession`. The registry hands back a
        // `LegacyAgentSession` shim today; M6 swaps Claude's entry for a
        // real `ClaudeSession` without touching this call site.
        //
        // File refs flow through `Turn::file_refs` as structured
        // `FileAttachment`s: (path, Some(content)) for @file text refs,
        // (path, None) for image attachments Claude routes via `--file`.
        // The shim splits them back into the legacy tuple shape.
        let mut turn_file_refs: Vec<gaviero_core::context_planner::FileAttachment> = Vec::new();
        for (path, content) in &file_refs {
            turn_file_refs.push(gaviero_core::context_planner::FileAttachment {
                path: std::path::PathBuf::from(path),
                content: Some(content.clone()),
            });
        }
        for p in &cli_file_attachments {
            turn_file_refs.push(gaviero_core::context_planner::FileAttachment {
                path: p.clone(),
                content: None,
            });
        }
        // Wrap existing chat history (`context`) into the planner's
        // ReplayPayload shape. The shim lifts it back for legacy backends.
        let replay_history = if !context.is_empty() {
            Some(gaviero_core::context_planner::ReplayPayload {
                entries: context
                    .iter()
                    .map(|(r, c)| {
                        let role = match r.as_str() {
                            "assistant" => gaviero_core::context_planner::Role::Assistant,
                            "system" => gaviero_core::context_planner::Role::System,
                            _ => gaviero_core::context_planner::Role::User,
                        };
                        (role, c.clone())
                    })
                    .collect(),
            })
        } else {
            None
        };
        // Fill `file_refs` / `replay_history` the planner left empty
        // (chat does its own @file parsing and history slicing). Preserves
        // M3 byte-identity through the shim.
        let mut selections = selections;
        selections.file_refs = turn_file_refs;
        selections.replay_history = replay_history;

        // S1: splice the chat-turn memory block as a pre-rendered
        // MemorySelection. Provider-agnostic; lives in core so CLI
        // callers do the same thing.
        gaviero_core::context_planner::splice_into_selections(chat_injection, &mut selections);

        let transport_ctx = gaviero_core::agent_session::TransportContext {
            user_message: prompt.clone(),
            effort: if options.effort.is_empty() || options.effort == "off" {
                None
            } else {
                Some(options.effort.clone())
            },
            auto_approve: options.auto_approve,
        };
        let turn = gaviero_core::agent_session::build_turn(selections, transport_ctx);

        let observer = TuiAcpObserver {
            tx: tx.clone(),
            conv_id: conv_id_clone.clone(),
        };
        let mut session = gaviero_core::agent_session::registry::create_session(
            gaviero_core::agent_session::registry::SessionConstruction {
                write_gate: wg.clone(),
                observer: Box::new(observer),
                model,
                ollama_base_url: Some(ollama_base_url),
                workspace_root: root,
                agent_id: "claude-chat".to_string(),
                options,
                profile: provider_profile_clone,
            },
        );
        if let Err(e) = session.send_turn(turn).await {
            tracing::error!("send_turn error: {}", e);
            let _ = tx.send(Event::MessageComplete {
                conv_id: conv_id.clone(),
                role: "system".to_string(),
                content: format!("Error: {}", e),
            });
        }
        session.close().await;

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
            tracing::info!(
                "Sending AcpTaskCompleted with {} proposals",
                proposals.len()
            );
            let _ = tx.send(Event::AcpTaskCompleted {
                conv_id: conv_id_clone,
                proposals,
            });
        } else {
            tracing::info!("No deferred proposals — skipping AcpTaskCompleted");
        }
    });
    app.acp_tasks.insert(
        app.chat_state.active_conversation_id().to_string(),
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
                        app.chat_state
                            .add_attachment(path, crate::panels::agent_chat::AttachmentKind::Image);
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
    let conv_id = app.chat_state.active_conversation_id().to_string();
    if let Some(task) = app.acp_tasks.remove(&conv_id) {
        task.abort();
        let conv = app.chat_state.active_conversation_mut();
        conv.is_streaming = false;
        conv.streaming_started_at = None;
        app.chat_state
            .finalize_message("system", "Cancelled by user.");
    }
}
