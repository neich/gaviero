use super::*;

/// Consume a keystroke while the Codex trust modal is open. Persists
/// the answer to `.gaviero/settings.json` and replays the pending
/// action regardless of grant/deny — denial just means Gaviero will
/// skip Codex config synthesis at that point.
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

    match dialog.pending {
        super::state::PendingAfterTrust::ChatSend => {
            // On grant, re-synthesize so the next codex-exec turn picks up the
            // gaviero MCP server (fresh subprocess reads .codex/config.toml at
            // spawn — no restart needed). On deny, codex runs without it. The
            // typed prompt is still in the chat input buffer, so replaying
            // `send_chat_message` dispatches it; trust is now persisted, so the
            // codex gate no longer fires (no dialog loop).
            if decision == "granted" {
                super::commands::resynthesize_mcp_configs(app);
            }
            app.chat_state
                .add_system_message(&format!("Codex MCP trust: {decision}. Sending…"));
            super::side_panel::send_chat_message(app);
        }
    }
}

/// Get the cached `RepoMap` or build it on demand.
///
/// M2 extracts this from the M1 `build_graph_context` so the chat path can
/// hand the planner a `&RepoMap` reference and let the planner own
/// rank_for_agent itself (V9 §11 M2: "Chat consumes PlannerSelections;
/// TUI no longer owns bootstrap policy").
/// Cached shallow directory topology for `<repo_topology>`.
pub(crate) async fn get_or_build_topology_cached(
    topology_cache: std::sync::Arc<
        tokio::sync::RwLock<std::collections::HashMap<std::path::PathBuf, String>>,
    >,
    workspace_root: std::path::PathBuf,
    excludes: Vec<String>,
    cfg: gaviero_core::repo_map::TopologyConfig,
) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    {
        let guard = topology_cache.read().await;
        if let Some(body) = guard.get(&workspace_root) {
            return Some(body.clone());
        }
    }
    let root = workspace_root.clone();
    match tokio::task::spawn_blocking(move || {
        gaviero_core::repo_map::build_folder_topology(&root, &excludes, &cfg)
    })
    .await
    {
        Ok(Ok(body)) if !body.is_empty() => {
            let mut guard = topology_cache.write().await;
            guard.insert(workspace_root, body.clone());
            Some(body)
        }
        _ => None,
    }
}

pub(crate) async fn get_or_build_repo_map_cached(
    repo_map_cache: std::sync::Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<
                std::path::PathBuf,
                std::sync::Arc<gaviero_core::repo_map::RepoMap>,
            >,
        >,
    >,
    workspace_root: std::path::PathBuf,
    excludes: Vec<String>,
) -> Option<std::sync::Arc<gaviero_core::repo_map::RepoMap>> {
    // Cache hit on the per-folder slot.
    {
        let guard = repo_map_cache.read().await;
        if let Some(rm) = guard.get(&workspace_root) {
            return Some(rm.clone());
        }
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
            guard.insert(workspace_root, arc.clone());
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

/// PUSH→PULL Phase 2: compute the thin impact *summary* for the strong-tier
/// chat first turn. Mirrors [`compute_impact_text`] but returns the ~150-token
/// count summary (naming `blast_radius(path)`) instead of the full ranked
/// render, so the model pulls the detail on demand. Returns `None` when seeds
/// are empty (empty buffer → inject nothing), the GraphStore can't be built,
/// or there is nothing to report.
pub(crate) async fn compute_impact_summary(
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
        let summary = gaviero_core::repo_map::store::GraphStore::format_impact_summary(&impact);
        if summary.is_empty() {
            None
        } else {
            Some(summary)
        }
    })
    .await
    .ok()
    .flatten()
}

/// Render chat prompt text from planner selections using the chat ordering:
/// user message first, then graph, then memory.
#[allow(dead_code)] // test-covered; chat-ordering renderer not yet wired into live dispatch
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
    if let Some(skills) =
        gaviero_core::swarm::backend::shared::render_skill_block(&selections.skill_selections)
    {
        parts.push(skills);
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: gaviero_core::repo_map::TopologyConfig::default(),
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: gaviero_core::context_planner::BootstrapArms::none(),
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

    /// An `App` over a throwaway workspace, with no session on disk yet.
    ///
    /// `App::new` seeds the widths from `.gaviero/settings.json`, so the empty
    /// object here means the built-in defaults (30 / 40 / 30) are in play and
    /// any other value in an assertion came from the session.
    fn session_app(dir: &std::path::Path) -> App {
        std::fs::create_dir_all(dir.join(".gaviero")).unwrap();
        std::fs::write(dir.join(".gaviero/settings.json"), "{}").unwrap();
        let workspace = Workspace::single_folder(dir.to_path_buf());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(workspace, tx)
    }

    /// Session state lands in the OS data dir, keyed by a hash of the workspace
    /// path, so a `tempfile` key cannot collide with a real workspace — but the
    /// directory still has to be removed or the test run litters the user's
    /// data dir. Mirrors the cleanup in `gaviero_core::session_state`'s tests.
    fn cleanup_session(dir: &std::path::Path) {
        if let Some(state_dir) = session_state::state_dir_for(dir) {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }

    #[test]
    fn panel_geometry_survives_a_restart() {
        // The point of the feature: widths and heights the user left behind
        // come back on the next run, rather than the settings defaults.
        let dir = tempfile::tempdir().unwrap();
        let mut app = session_app(dir.path());
        app.file_tree_width = 42;
        app.side_panel_width = 51;
        app.terminal_split_percent = 65;
        save_session(&app);

        let mut restarted = session_app(dir.path());
        assert_eq!(
            restarted.file_tree_width,
            theme::FILE_TREE_DEFAULT_WIDTH,
            "App::new uses the defaults"
        );

        restore_session(&mut restarted);
        assert_eq!(restarted.file_tree_width, 42);
        assert_eq!(restarted.side_panel_width, 51);
        assert_eq!(restarted.terminal_split_percent, 65);

        cleanup_session(dir.path());
    }

    #[test]
    fn restored_widths_are_clamped_to_the_configured_bounds() {
        // A `state.json` written on a 4K monitor, hand-edited, or left over
        // from an older build must not widen a panel past its maximum.
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path();
        session_state::save_session(
            key,
            &SessionState {
                file_tree_width: Some(9_000),
                side_panel_width: Some(0),
                ..SessionState::default()
            },
        )
        .unwrap();

        let mut app = session_app(key);
        restore_session(&mut app);
        assert_eq!(app.file_tree_width, theme::FILE_TREE_MAX_WIDTH);
        assert_eq!(app.side_panel_width, theme::SIDE_PANEL_MIN_WIDTH);

        cleanup_session(key);
    }

    #[test]
    fn a_preset_governed_layout_persists_no_absolute_width() {
        // While a preset is active the widths on screen are percentages, and
        // `app.file_tree_width` still holds the pre-preset columns. Storing
        // those would record a width that disagrees with what the user quit on.
        let dir = tempfile::tempdir().unwrap();
        let mut app = session_app(dir.path());
        app.switch_layout(1);
        app.file_tree_width = 42;
        app.side_panel_width = 51;
        save_session(&app);

        let state = session_state::load_session(&app.workspace_key());
        assert_eq!(state.active_preset, Some(1));
        assert_eq!(state.file_tree_width, None);
        assert_eq!(state.side_panel_width, None);
        // The terminal split is orthogonal to a preset, so it still persists.
        assert_eq!(state.terminal_split_percent, Some(30));

        cleanup_session(dir.path());
    }

    #[test]
    fn leaving_a_preset_makes_widths_authoritative_and_persisted_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = session_app(dir.path());
        app.switch_layout(1);
        assert!(app.active_preset.is_some());

        // Any manual resize materializes the preset's on-screen sizes into the
        // absolute widths and drops the preset.
        super::layout::resize_horizontal(&mut app, 5);
        assert!(app.active_preset.is_none());

        save_session(&app);
        let state = session_state::load_session(&app.workspace_key());
        assert_eq!(state.file_tree_width, Some(app.file_tree_width));
        assert_eq!(state.side_panel_width, Some(app.side_panel_width));

        cleanup_session(dir.path());
    }

    #[test]
    fn a_preset_restored_over_stored_widths_still_wins() {
        // Belt and braces for the pair above: presets decide the split after
        // restore, so a stale width in the session cannot survive one.
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path();
        session_state::save_session(
            key,
            &SessionState {
                active_preset: Some(2),
                file_tree_width: Some(42),
                side_panel_width: Some(51),
                ..SessionState::default()
            },
        )
        .unwrap();

        let mut app = session_app(key);
        restore_session(&mut app);
        assert_eq!(app.active_preset, Some(2));
        // Preset 3 is the explorer-less (0, 100, 0) default, and `switch_layout`
        // owns visibility, so the stored widths are not what is rendered.
        assert!(!app.panel_visible.file_tree);
        assert!(!app.panel_visible.side_panel);

        cleanup_session(key);
    }
}

/// Spawn a background task that (re)builds `RepoMap` and writes it into
/// `app.repo_map` under the primary workspace folder key. Safe to call
/// multiple times — each invocation replaces that folder's cached map.
/// Other folders are warmed up lazily on first use via
/// [`get_or_build_repo_map_cached`].
pub(crate) fn warm_up_repo_map(app: &App) {
    let Some(root) = app.graph_workspace_root.clone() else {
        return;
    };
    let cache = app.repo_map.clone();
    let excludes = super::parse_exclude_patterns(&app.workspace, Some(&root));
    let key = root.clone();
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || {
            gaviero_core::repo_map::RepoMap::build(&root, &excludes)
        })
        .await
        {
            Ok(Ok(map)) => {
                let mut guard = cache.write().await;
                guard.insert(key, std::sync::Arc::new(map));
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
        .filter(|c| c.is_streaming || c.has_running_background_agents())
        .count();

    let has_pending_review = app.diff_review.is_some();

    if unsaved.is_empty() && streaming_agents == 0 && !has_pending_review {
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
        app.file_tree
            .scroll
            .set_selected(state.tree_selected, count);
    }

    for tab in &state.tabs {
        let path =
            crate::editor::buffer::Buffer::resolve_editor_path(std::path::Path::new(&tab.path));
        if path.exists() {
            app.open_file(&path);
            if let Some(buf) = app.buffers.last_mut() {
                let max_line = buf.text.len_lines().saturating_sub(1);
                buf.cursor.line = tab.cursor_line.min(max_line);
                buf.cursor.col = tab.cursor_col;
                buf.scroll.top_line = tab.scroll_top.min(max_line);
                if buf.lang_name.as_deref() == Some("markdown") {
                    buf.preview_mode =
                        MarkdownPreviewMode::from_session_key(tab.preview_mode.as_deref());
                }
            }
        }
    }

    if state.active_tab < app.buffers.len() {
        app.active_buffer = state.active_tab;
    }

    // Live geometry is remembered per-session in `state.json` — the same file
    // that carries panel visibility — so a deleted session resets widths and
    // visibility together rather than leaving one of them behind. Both apply
    // before `switch_layout` below, which stays authoritative when a preset
    // was active at exit: it re-derives the horizontal split from percentages.
    //
    // Sanitised through the same helpers that clean the `panels.*` seed, so a
    // session written on a wider terminal (or hand-edited) lands in range
    // whether it arrives via `state.json` or via configuration.
    if let Some(width) = state.file_tree_width {
        app.file_tree_width = layout::clamp_file_tree_width(width);
    }
    if let Some(width) = state.side_panel_width {
        app.side_panel_width = layout::clamp_side_panel_width(width);
    }
    if let Some(pct) = state.terminal_split_percent {
        app.terminal_split_percent = layout::clamp_terminal_split_percent(pct);
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

/// The explorer width to remember across runs, or `None` while a layout preset
/// is active.
///
/// A preset decides the horizontal split as a percentage of the terminal, so
/// `app.file_tree_width` is *not* what is on screen while one is selected — it
/// still holds the pre-preset value. Persisting that would record a width that
/// disagrees with the layout the user actually quit on, reconciled on the next
/// run only by the preset re-applying over it. Storing `None` instead keeps a
/// value the preset owns out of the session entirely.
///
/// Leaving a preset is what makes widths authoritative again: every manual
/// resize runs `layout::materialize_preset_widths`, which copies the preset's
/// on-screen sizes into the absolute widths and clears `active_preset`. So the
/// moment widths start being stored is the moment they start being right.
fn stored_file_tree_width(app: &App) -> Option<u16> {
    app.active_preset.is_none().then_some(app.file_tree_width)
}

/// Side-panel sibling of [`stored_file_tree_width`], under the same rule.
fn stored_side_panel_width(app: &App) -> Option<u16> {
    app.active_preset.is_none().then_some(app.side_panel_width)
}

pub(super) fn save_session(app: &App) {
    let key = app.workspace_key();

    let tabs: Vec<TabState> = app
        .buffers
        .iter()
        .filter_map(|buf| {
            buf.path.as_ref().map(|p| TabState {
                path: crate::editor::buffer::Buffer::resolve_editor_path(p)
                    .to_string_lossy()
                    .to_string(),
                cursor_line: buf.cursor.line,
                cursor_col: buf.cursor.col,
                scroll_top: buf.scroll.top_line,
                preview_mode: buf.preview_mode.session_key().map(str::to_owned),
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
        file_tree_width: stored_file_tree_width(app),
        side_panel_width: stored_side_panel_width(app),
        terminal_split_percent: Some(app.terminal_split_percent),
        terminal_session: Some(app.terminal_manager.save_state()),
    };

    if let Err(e) = session_state::save_session(&key, &state) {
        tracing::warn!("Failed to save session state: {}", e);
    }

    app.chat_state.save_conversations(&key);
}
