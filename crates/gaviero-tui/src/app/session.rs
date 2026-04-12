use super::*;

pub(super) fn handle_first_run_key(app: &mut App, key: &crossterm::event::KeyEvent) {
    let step = match &app.first_run_dialog {
        Some(d) => d.step.clone(),
        None => return,
    };
    match step {
        FirstRunStep::AskSettings => match key.code {
            crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                if let Some(d) = &mut app.first_run_dialog {
                    d.create_settings = true;
                    d.step = FirstRunStep::AskMemory;
                }
            }
            crossterm::event::KeyCode::Char('n')
            | crossterm::event::KeyCode::Char('N')
            | crossterm::event::KeyCode::Esc => {
                if let Some(d) = &mut app.first_run_dialog {
                    d.create_settings = false;
                    d.step = FirstRunStep::AskMemory;
                }
            }
            _ => {}
        },
        FirstRunStep::AskMemory => match key.code {
            crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                app.apply_first_run(true);
            }
            crossterm::event::KeyCode::Char('n')
            | crossterm::event::KeyCode::Char('N')
            | crossterm::event::KeyCode::Esc => {
                app.apply_first_run(false);
            }
            _ => {}
        },
    }
}

pub(super) fn apply_first_run(app: &mut App, init_memory: bool) {
    let create_settings = app
        .first_run_dialog
        .as_ref()
        .map(|d| d.create_settings)
        .unwrap_or(false);
    app.first_run_dialog = None;

    if create_settings {
        app.workspace.ensure_settings();
        app.status_message = Some((
            "Created .gaviero/settings.json".to_string(),
            std::time::Instant::now(),
        ));
        app.refresh_file_tree();
    }

    if init_memory {
        if let Some(root) = app.workspace.roots().first().map(|r| r.to_path_buf()) {
            let tx = app.event_tx.clone();
            tokio::spawn(async move {
                match tokio::task::spawn_blocking(move || gaviero_core::memory::init_workspace(&root))
                    .await
                {
                    Ok(Ok(store)) => {
                        let _ = tx.send(Event::MemoryReady(store));
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Workspace memory init failed: {}", e);
                    }
                    Err(e) => {
                        tracing::warn!("Workspace memory init panicked: {}", e);
                    }
                }
            });
        }
    }

    // Warm up the code graph in the background so the first chat send doesn't pay build cost.
    warm_up_repo_map(app);
}

/// Build graph-backed context text for a chat send: repo outline + impact radius.
///
/// Returns empty string if `seeds` is empty, `budget_tokens == 0`, or the graph
/// can't be built. Rebuilds the cached `RepoMap` on demand if it was invalidated.
pub(crate) async fn build_graph_context(
    repo_map_cache: std::sync::Arc<tokio::sync::RwLock<Option<std::sync::Arc<gaviero_core::repo_map::RepoMap>>>>,
    workspace_root: std::path::PathBuf,
    seeds: Vec<String>,
    budget_tokens: usize,
) -> String {
    if seeds.is_empty() || budget_tokens == 0 {
        return String::new();
    }

    let repo_map = {
        let guard = repo_map_cache.read().await;
        guard.clone()
    };
    let repo_map = match repo_map {
        Some(rm) => Some(rm),
        None => {
            let root = workspace_root.clone();
            match tokio::task::spawn_blocking(move || gaviero_core::repo_map::RepoMap::build(&root)).await {
                Ok(Ok(map)) => {
                    let arc = std::sync::Arc::new(map);
                    let mut guard = repo_map_cache.write().await;
                    *guard = Some(arc.clone());
                    Some(arc)
                }
                _ => None,
            }
        }
    };

    let mut sections: Vec<String> = Vec::new();

    if let Some(rm) = &repo_map {
        let plan = rm.rank_for_agent(&seeds, budget_tokens);
        if !plan.repo_outline.is_empty() {
            sections.push(plan.repo_outline);
        }
    }

    // Impact radius lives in an on-disk SQLite store; rusqlite::Connection is !Send
    // so we build the store, query, and format entirely inside spawn_blocking.
    let seeds_clone = seeds.clone();
    let root_clone = workspace_root.clone();
    let impact_text = tokio::task::spawn_blocking(move || {
        let (store, _) = gaviero_core::repo_map::graph_builder::build_graph(&root_clone).ok()?;
        let seed_refs: Vec<&str> = seeds_clone.iter().map(|s| s.as_str()).collect();
        let impact = store.impact_radius(&seed_refs, 2).ok()?;
        if impact.affected_files.is_empty() {
            None
        } else {
            Some(gaviero_core::repo_map::store::GraphStore::format_impact_for_prompt(&impact))
        }
    })
    .await
    .ok()
    .flatten();

    if let Some(text) = impact_text {
        sections.push(text);
    }

    sections.join("\n\n")
}

/// Spawn a background task that (re)builds `RepoMap` and writes it into `app.repo_map`.
/// Safe to call multiple times — each invocation replaces the cached map.
pub(crate) fn warm_up_repo_map(app: &App) {
    let Some(root) = app.graph_workspace_root.clone() else {
        return;
    };
    let cache = app.repo_map.clone();
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || gaviero_core::repo_map::RepoMap::build(&root))
            .await
        {
            Ok(Ok(map)) => {
                let mut guard = cache.write().await;
                *guard = Some(std::sync::Arc::new(map));
                tracing::info!("repo_map warmed up");
            }
            Ok(Err(e)) => {
                tracing::debug!("repo_map build skipped: {}", e);
            }
            Err(e) => {
                tracing::warn!("repo_map build panicked: {}", e);
            }
        }
    });
}

pub(super) fn try_quit(app: &mut App) {
    use gaviero_core::swarm::models::AgentStatus;

    let unsaved: Vec<String> = app
        .buffers
        .iter()
        .filter(|b| b.modified)
        .map(|b| b.display_name().to_string())
        .collect();

    let streaming_agents = app
        .chat_state
        .conversations
        .iter()
        .filter(|c| c.is_streaming)
        .count();

    let running_swarm = app
        .swarm_dashboard
        .agents
        .iter()
        .filter(|a| matches!(a.status, AgentStatus::Running))
        .count();

    if unsaved.is_empty() && streaming_agents == 0 && running_swarm == 0 {
        app.should_quit = true;
    } else {
        app.quit_confirm = true;
    }
}

pub(super) fn workspace_key(app: &App) -> std::path::PathBuf {
    app.workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

pub(super) fn restore_session(app: &mut App) {
    let key = app.workspace_key();
    let state = session_state::load_session(&key);

    app.panel_visible.file_tree = state.panels.file_tree;
    app.panel_visible.side_panel = state.panels.side_panel;
    app.panel_visible.terminal = state.panels.terminal;

    app.file_tree.restore_expanded(&state.tree_expanded);
    if state.tree_selected < app.file_tree.entries.len() {
        app.file_tree.scroll.selected = state.tree_selected;
    }

    for tab in &state.tabs {
        let path = std::path::Path::new(&tab.path);
        if path.exists() {
            app.open_file(path);
            if let Some(buf) = app.buffers.last_mut() {
                let max_line = buf.text.len_lines().saturating_sub(1);
                buf.cursor.line = tab.cursor_line.min(max_line);
                buf.cursor.col = tab.cursor_col;
                buf.scroll.top_line = tab.scroll_top.min(max_line);
            }
        }
    }

    if state.active_tab < app.buffers.len() {
        app.active_buffer = state.active_tab;
    }

    if let Some(pct) = state.terminal_split_percent {
        app.terminal_split_percent = pct.clamp(10, 80);
    }

    if let Some(term_state) = &state.terminal_session {
        app.terminal_manager.restore_state(term_state);
    }

    if let Some(preset_idx) = state.active_preset {
        app.switch_layout(preset_idx as u8);
    }

    app.chat_state.load_conversations(&key);

    if !app.buffers.is_empty() {
        app.focus = Focus::Editor;
    } else if app.panel_visible.file_tree {
        app.focus = Focus::FileTree;
    }
}

pub(super) fn save_session(app: &App) {
    let key = app.workspace_key();

    let tabs: Vec<TabState> = app
        .buffers
        .iter()
        .filter_map(|buf| {
            buf.path.as_ref().map(|p| TabState {
                path: p.to_string_lossy().to_string(),
                cursor_line: buf.cursor.line,
                cursor_col: buf.cursor.col,
                scroll_top: buf.scroll.top_line,
            })
        })
        .collect();

    let state = SessionState {
        tabs,
        active_tab: app.active_buffer,
        panels: session_state::PanelState {
            file_tree: app.panel_visible.file_tree,
            side_panel: app.panel_visible.side_panel,
            terminal: app.panel_visible.terminal,
        },
        tree_expanded: app.file_tree.expanded_paths(),
        tree_selected: app.file_tree.scroll.selected,
        active_preset: app.active_preset,
        terminal_split_percent: Some(app.terminal_split_percent),
        terminal_session: Some(app.terminal_manager.save_state()),
    };

    if let Err(e) = session_state::save_session(&key, &state) {
        tracing::warn!("Failed to save session state: {}", e);
    }

    app.chat_state.save_conversations(&key);
}
