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

/// Consume a keystroke while the Codex trust modal is open. Persists
/// the answer to `.gaviero/settings.json` and replays the pending
/// `/swarm` regardless of grant/deny — denial just means Gaviero will
/// skip Codex config synthesis at swarm time.
pub(super) fn handle_codex_trust_key(app: &mut App, key: &crossterm::event::KeyEvent) {
    use gaviero_core::workspace::settings as S;

    let dialog = match app.codex_trust_dialog.take() {
        Some(d) => d,
        None => return,
    };

    let decision = match key.code {
        crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
            Some("granted")
        }
        crossterm::event::KeyCode::Char('n')
        | crossterm::event::KeyCode::Char('N')
        | crossterm::event::KeyCode::Esc => Some("denied"),
        _ => None,
    };

    let Some(decision) = decision else {
        // Unrecognized key — keep the dialog open.
        app.codex_trust_dialog = Some(dialog);
        return;
    };

    if let Some(root) = app.workspace.roots().first().map(|p| p.to_path_buf()) {
        if let Err(e) = app.workspace.save_folder_setting(
            &root,
            S::MCP_GAVIERO_CODEX_TRUST,
            serde_json::json!(decision),
        ) {
            tracing::warn!("persisting codexTrust failed: {e}");
        }
    }

    app.chat_state
        .add_system_message(&format!("Codex MCP trust: {decision}. Resuming /swarm…"));
    super::commands::run_swarm(app, dialog.pending_task);
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
            let ws = app.workspace.clone();
            tokio::spawn(async move {
                match tokio::task::spawn_blocking(move || {
                    gaviero_core::memory::init_workspace_stores(&root, &ws)
                })
                .await
                {
                    Ok(Ok(stores)) => {
                        let _ = tx.send(Event::MemoryReady(stores));
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

/// Get the cached `RepoMap` or build it on demand.
///
/// M2 extracts this from the M1 `build_graph_context` so the chat path can
/// hand the planner a `&RepoMap` reference and let the planner own
/// rank_for_agent itself (V9 §11 M2: "Chat consumes PlannerSelections;
/// TUI no longer owns bootstrap policy").
pub(crate) async fn get_or_build_repo_map_cached(
    repo_map_cache: std::sync::Arc<
        tokio::sync::RwLock<Option<std::sync::Arc<gaviero_core::repo_map::RepoMap>>>,
    >,
    workspace_root: std::path::PathBuf,
    excludes: Vec<String>,
) -> Option<std::sync::Arc<gaviero_core::repo_map::RepoMap>> {
    let cached = {
        let guard = repo_map_cache.read().await;
        guard.clone()
    };
    if let Some(rm) = cached {
        return Some(rm);
    }
    let root = workspace_root.clone();
    match tokio::task::spawn_blocking(move || {
        gaviero_core::repo_map::RepoMap::build(&root, &excludes)
    })
    .await
    {
        Ok(Ok(map)) => {
            let arc = std::sync::Arc::new(map);
            let mut guard = repo_map_cache.write().await;
            *guard = Some(arc.clone());
            Some(arc)
        }
        _ => None,
    }
}

/// Compute the prompt-formatted impact-radius text for the given seeds.
///
/// Returns `None` when seeds are empty, the GraphStore can't be built, or
/// no affected files were found. M2 carries this through the planner via
/// `PlannerInput::pre_fetched_impact_text` because `GraphStore` is `!Send`
/// and lives in `spawn_blocking`; M3 will wire `graph_store` into the
/// planner directly per V9 §4 and remove this carrier.
pub(crate) async fn compute_impact_text(
    workspace_root: std::path::PathBuf,
    seeds: Vec<String>,
    excludes: Vec<String>,
) -> Option<String> {
    if seeds.is_empty() {
        return None;
    }
    tokio::task::spawn_blocking(move || {
        let (store, _) =
            gaviero_core::repo_map::graph_builder::build_graph(&workspace_root, &excludes).ok()?;
        let seed_refs: Vec<&str> = seeds.iter().map(|s| s.as_str()).collect();
        let impact = store.impact_radius(&seed_refs, 2).ok()?;
        if impact.affected_files.is_empty() {
            return None;
        }
        // C3: rank the affected set with HippoRAG specificity and the
        // default `mode=All` weights, then render with `[sp 0.92]`
        // badges so the chat injection visibly carries the score.
        let mut to_rank: Vec<String> = impact.changed_files.clone();
        for f in &impact.affected_files {
            if !to_rank.contains(f) {
                to_rank.push(f.clone());
            }
        }
        let ranks = gaviero_core::repo_map::rank_files_with_mode(
            &store,
            &seed_refs,
            &to_rank,
            gaviero_core::repo_map::store::BlastRadiusMode::All,
            gaviero_core::repo_map::SpecificityConfig::default(),
        )
        .unwrap_or_default();
        Some(
            gaviero_core::repo_map::store::GraphStore::format_impact_for_prompt_ranked(
                &impact, &ranks,
            ),
        )
    })
    .await
    .ok()
    .flatten()
}

/// Render chat prompt text from planner selections using the chat ordering:
/// user message first, then graph, then memory.
pub(crate) fn render_chat_selections(
    selections: &gaviero_core::context_planner::PlannerSelections,
    user_message: &str,
) -> String {
    let mut parts: Vec<String> = vec![user_message.to_string()];

    if let Some(graph) =
        gaviero_core::swarm::backend::shared::render_graph_block(&selections.graph_selections)
    {
        parts.push(graph);
    }
    if let Some(memory) =
        gaviero_core::swarm::backend::shared::render_memory_block(&selections.memory_selections)
    {
        parts.push(memory);
    }

    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::context_planner::{
        ContextPlanner, ModelSpec, PlannerFingerprint, PlannerInput, PlannerSelections,
        RuntimeConfig, SessionLedger, build_provider_profile,
    };

    #[tokio::test]
    async fn m2_follow_up_turn_renders_only_user_message() {
        // V9 §11 M2 acceptance: "turn 2+ transmits only new user message;
        // turn-1 detail still in model context".
        //
        // Verify that after the ledger records turn 1 dispatched, the
        // planner emits empty memory + graph selections, and the
        // renderer therefore produces just the user prompt string.
        let profile = build_provider_profile(
            &ModelSpec::parse("claude:sonnet"),
            &RuntimeConfig::default(),
        );
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        ledger.record_turn_dispatched(); // turn 1 acknowledged → follow-up regime
        let workspace = std::path::PathBuf::from(".");

        // Provide memory + repo_map as None to keep the test hermetic.
        // The planner's follow-up branch must not query them anyway.
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };

        let user_message = "follow up question";
        let seeds = vec![std::path::PathBuf::from("src/lib.rs")];
        let read_ns = vec!["workspace".to_string()];
        let input = PlannerInput {
            user_message,
            explicit_refs: &[],
            seed_paths: &seeds,
            provider_profile: &profile,
            read_namespaces: &read_ns,
            graph_budget_tokens: 8000,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            // Even with carriers populated, the planner must skip
            // bootstrap on follow-up turns.
            pre_fetched_impact_text: Some("[Impact] should be ignored"),
            pre_fetched_graph_context: Some("[Graph] should be ignored"),
            pre_fetched_memory_context: Some("[Memory] should be ignored"),
        };
        let selections = planner.plan(&input).await.unwrap();
        assert!(
            selections.memory_selections.is_empty(),
            "follow-up turn must emit no memory selections"
        );
        assert!(
            selections.graph_selections.is_empty(),
            "follow-up turn must emit no graph selections"
        );

        let rendered = render_chat_selections(&selections, user_message);
        assert_eq!(
            rendered, user_message,
            "follow-up turn enriched prompt must equal the user message verbatim"
        );
    }

    #[test]
    fn render_chat_selections_first_turn_puts_prompt_before_graph_then_memory() {
        // Pins the post-fix chat-adapter ordering: user prompt FIRST, then
        // graph block, then memory block — joined by "\n\n". Putting the
        // prompt at the top keeps it inside Claude's default 2000-line Read
        // window when this blob is later spilled to a tempfile on
        // bootstrap-heavy first turns.
        let mut sel = PlannerSelections::default();
        sel.graph_selections
            .push(gaviero_core::context_planner::GraphSelection {
                path: None,
                kind: gaviero_core::context_planner::GraphSelectionKind::OutlineOnly,
                token_estimate: 0,
                content: "[Graph] outline".to_string(),
                rank_score: None,
                confidence: None,
                symbols: Vec::new(),
                content_digest: None,
            });
        sel.memory_selections
            .push(gaviero_core::context_planner::MemorySelection {
                id: None,
                namespace: None,
                scope_label: None,
                score: None,
                trust: None,
                content: "[Memory] context".to_string(),
                source_hash: None,
                updated_at: None,
            });
        let out = render_chat_selections(&sel, "do the thing");
        assert_eq!(out, "do the thing\n\n[Graph] outline\n\n[Memory] context");
    }
}

/// Spawn a background task that (re)builds `RepoMap` and writes it into `app.repo_map`.
/// Safe to call multiple times — each invocation replaces the cached map.
pub(crate) fn warm_up_repo_map(app: &App) {
    let Some(root) = app.graph_workspace_root.clone() else {
        return;
    };
    let cache = app.repo_map.clone();
    let excludes = super::parse_exclude_patterns(&app.workspace, Some(&root));
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || {
            gaviero_core::repo_map::RepoMap::build(&root, &excludes)
        })
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

    let has_pending_review = app.diff_review.is_some();

    if unsaved.is_empty() && streaming_agents == 0 && running_swarm == 0 && !has_pending_review {
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
        let count = app.file_tree.entries.len();
        app.file_tree.scroll.set_selected(state.tree_selected, count);
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
