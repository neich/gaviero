use super::*;

/// Mark the cached `RepoMap` as stale; the next chat send will rebuild it.
///
/// Cheap: acquires the lock briefly and writes `None`. Avoids rebuilding
/// eagerly on every file save — rebuild cost is paid only when next needed.
fn invalidate_repo_map(app: &App) {
    let cache = app.repo_map.clone();
    tokio::spawn(async move {
        let mut guard = cache.write().await;
        *guard = None;
    });
}

pub(super) fn handle_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) => {
            if app.focus == Focus::Terminal {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    if inst.spawned {
                        use crate::panels::terminal::{is_terminal_escape_key, key_event_to_bytes};
                        if is_terminal_escape_key(&key) {
                            let action = Keymap::resolve(&key);
                            app.handle_action(action);
                        } else {
                            let bytes = key_event_to_bytes(&key);
                            if !bytes.is_empty() {
                                app.terminal_selection.clear();
                                let inst = app.terminal_manager.active_instance_mut().unwrap();
                                inst.screen_mut().set_scrollback(0);
                                inst.write_input(&bytes);
                            }
                        }
                        return;
                    }
                }
            }

            if app.first_run_dialog.is_some() {
                app.handle_first_run_key(&key);
                return;
            }

            if app.quit_confirm {
                match key.code {
                    crossterm::event::KeyCode::Char('y')
                    | crossterm::event::KeyCode::Char('Y') => {
                        app.should_quit = true;
                    }
                    _ => {
                        app.quit_confirm = false;
                    }
                }
                return;
            }

            if app.tree_dialog.is_some() {
                app.handle_dialog_key(&key);
                return;
            }

            if app.move_state.is_some()
                && app.focus == Focus::FileTree
                && app.left_panel == LeftPanelMode::FileTree
            {
                app.handle_move_key(&key);
                return;
            }

            if app.find_bar_active && key.code == crossterm::event::KeyCode::Esc {
                app.find_bar_active = false;
                if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    buf.set_search_highlight(None);
                }
                return;
            }

            let action = Keymap::resolve(&key);
            app.handle_action(action);
        }
        Event::Paste(text) => {
            if app.focus == Focus::Terminal {
                if let Some(inst) = app.terminal_manager.active_instance_mut() {
                    if inst.spawned {
                        inst.write_input(text.as_bytes());
                        return;
                    }
                }
            }
            app.handle_paste(&text);
        }
        Event::Mouse(mouse) => app.handle_mouse(mouse),
        Event::Resize(_w, _h) => {
            app.needs_full_redraw = true;
        }
        Event::FileChanged(path) => {
            invalidate_repo_map(app);
            app.handle_file_changed(&path);
        }
        Event::FileTreeChanged => {
            invalidate_repo_map(app);
            app.refresh_file_tree();
            app.refresh_git_panel();
        }
        Event::ProposalCreated(proposal) => {
            app.enter_review_mode(*proposal, DiffSource::Acp);
        }
        Event::ProposalUpdated(_id) => {}
        Event::ProposalFinalized(path_str) => {
            let path = std::path::PathBuf::from(&path_str);
            if path.exists() {
                app.open_file(&path);
            }
            app.refresh_file_tree();
        }
        Event::StreamChunk { conv_id, text } => {
            if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                app.swarm_dashboard.append_stream_chunk(agent_id, &text);
            } else {
                app.chat_state.append_stream_chunk_to(&conv_id, &text);
            }
        }
        Event::ToolCallStarted { conv_id, tool_name } => {
            if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                app.swarm_dashboard.add_tool_call(agent_id, &tool_name);
            } else {
                app.chat_state.add_tool_call_to(&conv_id, &tool_name);
            }
        }
        Event::StreamingStatus { conv_id, status } => {
            if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                app.swarm_dashboard.set_streaming_status(agent_id, &status);
            } else if let Some(idx) = app.chat_state.find_conv_idx(&conv_id) {
                app.chat_state.conversations[idx].streaming_status = status;
            }
        }
        Event::MessageComplete {
            conv_id,
            role,
            content,
        } => {
            if conv_id.is_empty() {
                app.chat_state.add_system_message(&content);
                if app.swarm_dashboard.phase == "failed" {
                    app.swarm_dashboard.status_message = content.clone();
                }
                tracing::warn!("Swarm message: {}", content);
            } else {
                app.chat_state.finalize_message_to(&conv_id, &role, &content);
                if role == "assistant" {
                    app.chat_state.collapse_file_blocks_in(&conv_id);
                    super::chat_memory::store_chat_turn(app, &conv_id, &content);
                }
            }
        }
        Event::FileProposalDeferred {
            conv_id,
            path,
            additions,
            deletions,
        } => {
            tracing::debug!(
                "Deferred proposal: {} (+{} -{})",
                path.display(),
                additions,
                deletions
            );
            if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                app.swarm_dashboard.add_file_change(
                    agent_id,
                    &path.to_string_lossy(),
                    additions,
                    deletions,
                );
            } else {
                app.chat_state
                    .append_deferred_summary(&conv_id, &path, additions, deletions);
            }
        }
        Event::ClaudeSessionStarted { conv_id, session_id } => {
            if let Some(conv) = app
                .chat_state
                .conversations
                .iter_mut()
                .find(|c| c.id == conv_id)
            {
                if conv.claude_session_id.as_deref() != Some(session_id.as_str()) {
                    tracing::info!(
                        "Captured Claude session id for conv {}: {}",
                        conv_id,
                        session_id
                    );
                    conv.claude_session_id = Some(session_id.clone());
                }
                // M1: keep the planner ledger in sync. `record_continuity_handle`
                // mirrors `claude_session_id`; `record_turn_dispatched` flips
                // `is_first_turn()` for subsequent sends — equivalent to
                // today's `claude_session_id.is_none()` check.
                if let Some(ref mut ledger) = conv.session_ledger {
                    ledger.record_continuity_handle(
                        gaviero_core::context_planner::ContinuityHandle::ClaudeSessionId(
                            session_id,
                        ),
                    );
                    if ledger.is_first_turn() {
                        ledger.record_turn_dispatched();
                    }
                }
            }
        }
        Event::AcpTaskCompleted { conv_id, proposals } => {
            tracing::info!(
                "ACP task completed for conv {} with {} proposals",
                conv_id,
                proposals.len()
            );
            if proposals.is_empty() {
                app.status_message = Some((
                    "Agent finished — no file changes".to_string(),
                    std::time::Instant::now(),
                ));
            } else {
                app.enter_batch_review(proposals);
            }
        }
        Event::PermissionRequest {
            conv_id,
            tool_name,
            description,
            respond,
        } => {
            if let Some(idx) = app.chat_state.find_conv_idx(&conv_id) {
                app.chat_state.conversations[idx].streaming_status =
                    format!("Waiting for permission: {}", tool_name);
                app.chat_state.set_pending_permission(
                    &conv_id,
                    crate::panels::agent_chat::PendingPermission {
                        tool_name,
                        description,
                        respond,
                    },
                );
                app.panel_visible.side_panel = true;
                if app.side_panel != SidePanelMode::AgentChat {
                    app.side_panel = SidePanelMode::AgentChat;
                }
                app.chat_state.active_conv = idx;
                app.focus = Focus::SidePanel;
            } else {
                let _ = respond.send(false);
            }
        }
        Event::SwarmPhaseChanged(phase) => {
            app.swarm_dashboard.set_phase(&phase);
            if phase == "running" || phase == "merging" || phase == "verifying" {
                app.swarm_dashboard.status_message.clear();
            } else if phase == "validating" {
                app.swarm_dashboard.status_message = "Validating scopes and dependencies...".into();
            } else if phase == "failed" {
                if app.swarm_dashboard.status_message.is_empty() {
                    app.swarm_dashboard.status_message =
                        "Run failed — waiting for error details...".into();
                }
            } else if phase == "reverted" {
                app.swarm_dashboard.result = None;
                app.swarm_dashboard.diff_agent = None;
                app.swarm_dashboard.status_message = "Swarm reverted to pre-run state.".into();
            } else if phase.starts_with("revert failed") || phase.starts_with("revert panicked") {
                app.swarm_dashboard.status_message =
                    format!("Undo failed: {}", &phase["revert ".len()..]);
            }
        }
        Event::SwarmAgentStateChanged { id, status, detail } => {
            app.swarm_dashboard.update_agent(&id, &status, &detail);
        }
        Event::SwarmTierStarted { current, total } => {
            app.swarm_dashboard.set_tier(current, total);
        }
        Event::SwarmCompleted(result) => {
            app.swarm_dashboard.set_phase("completed");
            app.swarm_dashboard.status_message.clear();
            app.swarm_dashboard.set_result(*result);
        }
        Event::SwarmMergeConflict { branch, files } => {
            app.status_message = Some((
                format!("Merge conflict in {}: {}", branch, files.join(", ")),
                std::time::Instant::now(),
            ));
        }
        Event::SwarmCoordinationStarted(_prompt) => {
            app.swarm_dashboard.set_phase("coordinating");
            app.swarm_dashboard.status_message = "Opus is decomposing the task...".into();
        }
        Event::SwarmCoordinationComplete {
            unit_count,
            summary: _,
        } => {
            app.swarm_dashboard
                .set_phase(&format!("planned ({} agents)", unit_count));
            app.swarm_dashboard.status_message =
                format!("Plan ready: {} agents, starting execution...", unit_count);
        }
        Event::SwarmTierDispatch {
            unit_id,
            tier,
            backend,
        } => {
            app.swarm_dashboard.set_tier_dispatch(&unit_id, tier, &backend);
        }
        Event::SwarmCostUpdate(estimate) => {
            app.swarm_dashboard.set_cost(estimate.estimated_usd);
        }
        Event::SwarmDslPlanReady(plan_path) => {
            app.swarm_dashboard.set_phase("plan ready");
            let workspace_root = app
                .workspace
                .roots()
                .first()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let rel = plan_path
                .strip_prefix(&workspace_root)
                .unwrap_or(&plan_path)
                .display()
                .to_string();
            app.swarm_dashboard.status_message = format!("Plan saved: {} — review and /run it", rel);
            app.chat_state.add_system_message(&format!(
                "Plan saved to `{}`.\nReview it (it's open in the editor), then run it with:\n  /run {}",
                rel, rel,
            ));
            app.open_file(&plan_path);
            app.focus = Focus::Editor;
        }
        Event::MemoryReady(store) => {
            app.memory = Some(store);
            app.status_message = Some(("Memory ready".to_string(), std::time::Instant::now()));
            app.refresh_file_tree();
        }
        Event::Terminal(term_event) => {
            if matches!(
                &term_event,
                gaviero_core::terminal::TerminalEvent::PtyOutput { .. }
            ) {
                app.terminal_selection.clear();
            }
            app.terminal_manager.process_event(term_event);
        }
        Event::Tick => {
            app.terminal_manager.tick();
            if app.chat_state.active_conv_streaming() {
                app.chat_state.tick_count = app.chat_state.tick_count.wrapping_add(1);
            }
        }
    }
}

pub(super) fn handle_action(app: &mut App, action: Action) {
    if app.diff_review.is_some() && app.handle_review_action(&action) {
        return;
    }

    if app.batch_review.is_some() && app.handle_batch_review_action(&action) {
        return;
    }

    if app.panel_visible.terminal {
        match action {
            Action::MoveLineUp if app.focus == Focus::Terminal => {
                app.terminal_split_percent = (app.terminal_split_percent
                    + theme::TERMINAL_RESIZE_STEP)
                    .min(theme::TERMINAL_MAX_PERCENT);
                return;
            }
            Action::MoveLineDown if app.focus == Focus::Terminal => {
                app.terminal_split_percent = app
                    .terminal_split_percent
                    .saturating_sub(theme::TERMINAL_RESIZE_STEP)
                    .max(theme::TERMINAL_MIN_PERCENT);
                return;
            }
            _ => {}
        }
    }

    if app.focus == Focus::Terminal {
        match action {
            Action::PageUp => {
                if let Some(inst) = app.terminal_manager.active_instance_mut() {
                    let current = inst.screen().scrollback();
                    let page = inst.screen().size().0 as usize;
                    inst.screen_mut().set_scrollback(current + page);
                }
                return;
            }
            Action::PageDown => {
                if let Some(inst) = app.terminal_manager.active_instance_mut() {
                    let current = inst.screen().scrollback();
                    let page = inst.screen().size().0 as usize;
                    inst.screen_mut().set_scrollback(current.saturating_sub(page));
                }
                return;
            }
            _ => {}
        }
    }

    if app.focus == Focus::Terminal {
        if let Action::Paste = action {
            let text = app.get_clipboard();
            if !text.is_empty() {
                if let Some(inst) = app.terminal_manager.active_instance_mut() {
                    if inst.spawned {
                        let mut payload = b"\x1b[200~".to_vec();
                        payload.extend_from_slice(text.as_bytes());
                        payload.extend_from_slice(b"\x1b[201~");
                        inst.write_input(&payload);
                    }
                }
            }
            return;
        }
    }

    if app.focus == Focus::SidePanel {
        match action {
            Action::MoveLineUp => {
                let current = app.chat_state.input_area_rows.max(3);
                app.chat_state.input_area_rows = (current + 1).min(30);
                return;
            }
            Action::MoveLineDown => {
                let current = app.chat_state.input_area_rows;
                if current <= 3 {
                    app.chat_state.input_area_rows = 0;
                } else {
                    app.chat_state.input_area_rows = current - 1;
                }
                return;
            }
            _ => {}
        }
    }

    match action {
        Action::Quit => {
            if app.focus == Focus::SidePanel
                && matches!(app.side_panel, SidePanelMode::SwarmDashboard)
                && app.swarm_dashboard.diff_agent.is_some()
            {
                app.swarm_dashboard.close_diff();
            } else if app.focus == Focus::SidePanel
                && matches!(app.side_panel, SidePanelMode::SwarmDashboard)
                && app.swarm_dashboard.pending_undo_confirm
            {
                app.swarm_dashboard.pending_undo_confirm = false;
            } else if app.diff_review.is_some() {
                app.diff_review = None;
            } else {
                app.try_quit();
            }
        }
        Action::ToggleFileTree => {
            app.panel_visible.file_tree = !app.panel_visible.file_tree;
            if !app.panel_visible.file_tree && app.focus == Focus::FileTree {
                app.focus = Focus::Editor;
            }
        }
        Action::ToggleSidePanel => {
            app.panel_visible.side_panel = !app.panel_visible.side_panel;
            if !app.panel_visible.side_panel && app.focus == Focus::SidePanel {
                app.focus = Focus::Editor;
            }
        }
        Action::ToggleSwarmDashboard => {
            if !app.panel_visible.side_panel {
                app.panel_visible.side_panel = true;
            }
            app.side_panel = SidePanelMode::SwarmDashboard;
            app.focus = Focus::SidePanel;
        }
        Action::SetSideModeChat => {
            app.panel_visible.side_panel = true;
            app.side_panel = SidePanelMode::AgentChat;
            app.focus = Focus::SidePanel;
        }
        Action::SetSideModeSwarm => {
            app.panel_visible.side_panel = true;
            app.side_panel = SidePanelMode::SwarmDashboard;
            app.focus = Focus::SidePanel;
        }
        Action::SetSideModeGit => {
            app.panel_visible.side_panel = true;
            app.side_panel = SidePanelMode::GitPanel;
            app.focus = Focus::SidePanel;
            app.refresh_git_panel();
        }
        Action::ToggleTerminal => {
            if app.focus == Focus::SidePanel && matches!(app.side_panel, SidePanelMode::AgentChat) {
                if !app.chat_state.active_conv_streaming() {
                    app.chat_state.insert_char('\n');
                }
                return;
            }
            app.panel_visible.terminal = !app.panel_visible.terminal;
            if app.panel_visible.terminal {
                app.spawn_active_terminal();
                app.focus = Focus::Terminal;
            } else if app.focus == Focus::Terminal {
                app.focus = Focus::Editor;
            }
        }
        Action::NewTerminal => {
            let root = app
                .workspace
                .roots()
                .first()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            match app.terminal_manager.create_tab(&root) {
                Ok(id) => {
                    app.terminal_manager.switch_tab(id);
                    app.panel_visible.terminal = true;
                    app.focus = Focus::Terminal;
                }
                Err(e) => {
                    app.status_message =
                        Some((format!("Terminal: {}", e), std::time::Instant::now()));
                }
            }
        }
        Action::CloseTerminal => {
            if app.focus == Focus::Terminal {
                if let Some(id) = app.terminal_manager.active_tab() {
                    if app.terminal_manager.tab_count() > 1 {
                        app.terminal_manager.close_tab(id);
                    } else {
                        app.terminal_manager.close_tab(id);
                        app.panel_visible.terminal = false;
                        app.focus = Focus::Editor;
                    }
                }
            }
        }
        Action::NewTab => {
            if app.focus == Focus::SidePanel {
                app.chat_state.new_conversation();
            } else {
                app.buffers.push(Buffer::empty());
                app.active_buffer = app.buffers.len() - 1;
                app.focus = Focus::Editor;
            }
        }
        Action::FocusLeftPanel => {
            if !app.panel_visible.file_tree {
                app.panel_visible.file_tree = true;
            }
            app.focus = Focus::FileTree;
        }
        Action::FocusEditor => {
            app.focus = Focus::Editor;
        }
        Action::FocusSidePanel => {
            if !app.panel_visible.side_panel {
                app.panel_visible.side_panel = true;
            }
            app.focus = Focus::SidePanel;
        }
        Action::FocusTerminal => {
            if !app.panel_visible.terminal {
                app.panel_visible.terminal = true;
                app.spawn_active_terminal();
            }
            app.focus = Focus::Terminal;
        }
        Action::CycleTabForward => {
            if app.focus == Focus::SidePanel {
                app.chat_state.next_conversation();
            } else {
                app.cycle_tab(1);
            }
        }
        Action::CycleTabBack => {
            if app.focus == Focus::SidePanel {
                app.chat_state.prev_conversation();
            } else {
                app.cycle_tab(-1);
            }
        }
        Action::SetLeftModeExplorer => {
            if !app.panel_visible.file_tree {
                app.panel_visible.file_tree = true;
            }
            app.left_panel = LeftPanelMode::FileTree;
            app.focus = Focus::FileTree;
        }
        Action::SetLeftModeFind => {
            if !app.panel_visible.file_tree {
                app.panel_visible.file_tree = true;
            }
            app.left_panel = LeftPanelMode::Search;
            app.focus = Focus::FileTree;
            app.search_panel.focus_input();
        }
        Action::SetLeftModeChanges => {
            if !app.panel_visible.file_tree {
                app.panel_visible.file_tree = true;
            }
            app.left_panel = LeftPanelMode::Changes;
            app.focus = Focus::FileTree;
            app.refresh_git_changes();
        }
        Action::CloseTab => {
            if app.focus == Focus::Terminal {
                app.handle_action(Action::CloseTerminal);
            } else if app.focus == Focus::SidePanel
                && app.side_panel == SidePanelMode::AgentChat
            {
                let closing_conv_id = app
                    .chat_state
                    .conversations
                    .get(app.chat_state.active_conv)
                    .map(|c| c.id.clone());
                if let Some(ref id) = closing_conv_id {
                    super::chat_memory::consolidate_conversation(app, id);
                }
                app.chat_state.close_conversation();
            } else {
                app.close_tab();
            }
        }
        Action::Save => app.save_current_buffer(),
        Action::TogglePreview => {
            app.preview_visible = !app.preview_visible;
            app.preview_scroll = 0;
        }
        Action::ToggleFullscreen => app.toggle_fullscreen(),
        Action::SwitchLayout(n) => app.switch_layout(n),
        Action::FindInBuffer => {
            app.find_bar_active = true;
            app.find_input.select_all();
            app.focus = Focus::Editor;
            return;
        }
        Action::SearchInWorkspace => {
            if app.find_bar_active {
                if let Some(buf) = app.buffers.get_mut(app.active_buffer) {
                    buf.find_next_match();
                }
                app.ensure_editor_cursor_visible();
                return;
            }
            app.search_selected_in_workspace();
            return;
        }
        _ if app.focus == Focus::FileTree => match app.left_panel {
            LeftPanelMode::FileTree => app.handle_file_tree_action(action),
            LeftPanelMode::Search => app.handle_search_action(action),
            LeftPanelMode::Review => {}
            LeftPanelMode::Changes => {
                app.handle_changes_action(&action);
            }
        },
        _ if app.focus == Focus::Editor && app.find_bar_active => {
            app.handle_find_bar_action(action);
        }
        _ if app.focus == Focus::Editor => {
            if app.diff_review.is_none() {
                app.handle_editor_action(action);
            }
        }
        _ if app.focus == Focus::SidePanel => match app.side_panel {
            SidePanelMode::AgentChat => app.handle_chat_action(action),
            SidePanelMode::GitPanel => app.handle_git_panel_action(action),
            SidePanelMode::SwarmDashboard => app.handle_swarm_dashboard_action(action),
        },
        _ => {}
    }
}
