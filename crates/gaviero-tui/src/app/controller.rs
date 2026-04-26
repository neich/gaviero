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

                        // Intercept Ctrl+C when a terminal selection is active: copy instead of sending ^C.
                        let is_ctrl_c = key.code == crossterm::event::KeyCode::Char('c')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL);
                        if is_ctrl_c && app.terminal_selection.has_selection() {
                            let text =
                                if let Some(inst) = app.terminal_manager.active_instance_mut() {
                                    app.terminal_selection.extract_text(inst.screen_mut())
                                } else {
                                    None
                                };
                            if let Some(text) = text {
                                app.set_clipboard(&text);
                            }
                            app.terminal_selection.clear();
                            return;
                        }

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

            if app.codex_trust_dialog.is_some() {
                super::session::handle_codex_trust_key(app, &key);
                return;
            }

            if app.quit_confirm {
                use crossterm::event::KeyCode;
                let has_review = app.diff_review.is_some();
                match key.code {
                    KeyCode::Char('a') | KeyCode::Char('A') if has_review => {
                        super::review::finalize_current_review(app);
                        app.quit_confirm = false;
                        app.try_quit();
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') if has_review => {
                        app.diff_review = None;
                        app.quit_confirm = false;
                        app.try_quit();
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        app.diff_review = None;
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
                // A1: for assistant responses, parse + strip the
                // `<turn_annotations>` sidecar before the text reaches
                // the user. Parse failures never fail the turn — the
                // raw text flows on with annotations = None.
                let (visible_content, annotations_value): (String, Option<serde_json::Value>) =
                    if role == "assistant" {
                        let parsed = gaviero_core::memory::parse_and_strip(&content);
                        if let Some(err) = &parsed.parse_error {
                            tracing::warn!(
                                target: "memory_annotations",
                                conv_id = %conv_id,
                                error = %err,
                                "<turn_annotations> parse failed; stripping block, dropping annotations"
                            );
                        }
                        let value = parsed
                            .annotations
                            .as_ref()
                            .and_then(|a| serde_json::to_value(a).ok());
                        (parsed.stripped, value)
                    } else {
                        (content.clone(), None)
                    };

                app.chat_state
                    .finalize_message_to(&conv_id, &role, &visible_content);
                if role == "assistant" {
                    app.chat_state.collapse_file_blocks_in(&conv_id);
                    // S3 + A1: hand the turn transcript and (if parsed)
                    // the annotations sidecar to the extractor via the
                    // writer task. Fire-and-forget; the writer applies
                    // the short-turn cap, dedupe, and the safety-net
                    // extractor pass.
                    let workspace_root = app
                        .workspace
                        .roots()
                        .first()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let extractor_enabled = app
                        .workspace
                        .resolve_setting(
                            gaviero_core::workspace::settings::MEMORY_EXTRACTOR_ENABLED,
                            Some(&workspace_root),
                        )
                        .as_bool()
                        .unwrap_or(true);
                    if extractor_enabled && let Some(writer) = app.memory_writer.as_ref() {
                        if let Some(transcript) = super::chat_memory::build_turn_transcript(
                            app,
                            &conv_id,
                            &visible_content,
                        ) {
                            let (turn_id, module_path) = if let Some(idx) =
                                app.chat_state.find_conv_idx(&conv_id)
                            {
                                let conv = &mut app.chat_state.conversations[idx];
                                let turn_id = conv.pending_turn_id.take().unwrap_or_else(|| {
                                    format!(
                                        "{}-{}",
                                        conv_id,
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_millis())
                                            .unwrap_or(0)
                                    )
                                });
                                let module_path = conv.pending_module_path.take();
                                (turn_id, module_path)
                            } else {
                                (
                                    format!(
                                        "{}-{}",
                                        conv_id,
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_millis())
                                            .unwrap_or(0)
                                    ),
                                    None,
                                )
                            };
                            let repo_id = gaviero_core::memory::hash_path(&workspace_root);
                            let _ = writer.turn_complete(
                                conv_id.clone(),
                                turn_id.clone(),
                                repo_id,
                                module_path,
                                conv_id.clone(),
                                transcript,
                                annotations_value,
                            );
                            // Tier B / B6: post-turn retrieval-use telemetry.
                            // Fire-and-forget — never blocks the user.
                            // Skipped automatically inside the writer when
                            // the response is below `minResponseTokens`.
                            let telemetry_enabled = app
                                .workspace
                                .resolve_setting(
                                    gaviero_core::workspace::settings::MEMORY_TELEMETRY_ENABLED,
                                    Some(&workspace_root),
                                )
                                .as_bool()
                                .unwrap_or(true);
                            if telemetry_enabled {
                                let _ = writer.telemetry_classify(
                                    turn_id,
                                    conv_id.clone(),
                                    visible_content.clone(),
                                );
                            }
                        }
                    }
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
        Event::ClaudeSessionStarted {
            conv_id,
            session_id,
        } => {
            if let Some(conv) = app
                .chat_state
                .conversations
                .iter_mut()
                .find(|c| c.id == conv_id)
            {
                // V9 §11 M4 resume-failure detection: if we asked Claude
                // to `--resume <prior_id>` and it reported back a *different*
                // session_id, the prior handle was rejected (likely the
                // persisted conversation expired or was never saved). The
                // new id becomes the canonical one going forward, but we
                // also zero the ledger's turn_count so bootstrap fires on
                // this turn — turn-1 context (graph + memory) needs to be
                // re-injected since Claude has no server-side state.
                let was_resume_attempt = conv.claude_session_id.is_some();
                let resume_failed = was_resume_attempt
                    && conv.claude_session_id.as_deref() != Some(session_id.as_str());
                if resume_failed {
                    tracing::warn!(
                        target: "turn_metrics",
                        conv_id = %conv_id,
                        asked_id = %conv.claude_session_id.clone().unwrap_or_default(),
                        got_id = %session_id,
                        "claude resume rejected — forcing bootstrap on next turn"
                    );
                    if let Some(ref mut ledger) = conv.session_ledger {
                        ledger.record_resume_failure();
                    }
                }
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
                    if !resume_failed {
                        // M4: track successful resume timing for future
                        // invalidation heuristics (e.g., drop handles
                        // older than N days).
                        ledger.record_resume_success();
                    }
                    if ledger.is_first_turn() {
                        ledger.record_turn_dispatched();
                    }
                }
            }
        }
        Event::MemoryWriteEnqueued { kind: _ } => {
            // Count only; the committed callback drives the refresh.
            app.memory_panel.write_activity_counter =
                app.memory_panel.write_activity_counter.wrapping_add(1);
        }
        Event::MemoryWriteCommitted { kind: _ } => {
            // Debounce bootstrap + extractor-burst storms.
            let now = std::time::Instant::now();
            if let Some(prev) = app.memory_panel.last_recent_refresh {
                if now.duration_since(prev) < crate::panels::memory_panel::RECENT_REFRESH_DEBOUNCE {
                    return;
                }
            }
            app.memory_panel.last_recent_refresh = Some(now);
            // C1.5: Section 2 query honors the active kind tab so the
            // Recently Written list reflects whichever kind the user
            // selected (Records / History / Summaries).
            let active_kind = app.memory_panel.active_kind;
            // Fire a fresh query for Section 2 + 3.
            if let Some(mem) = app.memory.clone() {
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
                        let _ = tx.send(Event::MemorySearchResults { rows: panel_rows });
                    }
                    if let Ok(summary) = mem.workspace().scope_summary().await {
                        let rows: Vec<crate::panels::memory_panel::ScopeSummaryRow> = summary
                            .into_iter()
                            .map(|(level, count, last)| {
                                let label = match level {
                                    0 => "Global",
                                    1 => "Workspace",
                                    2 => "Repo",
                                    3 => "Module",
                                    4 => "Run",
                                    _ => "?",
                                };
                                crate::panels::memory_panel::ScopeSummaryRow {
                                    scope_label: label,
                                    count,
                                    last_write: last,
                                }
                            })
                            .collect();
                        let _ = tx.send(Event::MemoryScopeSummary { rows });
                    }
                });
            }
        }
        Event::MemoryWriteFailed { kind, error } => {
            app.memory_panel.last_error = Some((
                format!("write {kind} failed: {error}"),
                std::time::Instant::now(),
            ));
        }
        Event::MemoryManifestPersisted {
            turn_id,
            session_id: _,
        } => {
            // Re-fetch the full row by turn_id.
            if let Some(mem) = app.memory.clone() {
                let tx = app.event_tx.clone();
                tokio::spawn(async move {
                    if let Ok(rows) = mem.workspace().manifests_for_turn(&turn_id).await {
                        if let Some(row) = rows.into_iter().next() {
                            let _ = tx.send(Event::MemoryManifestReady { row });
                        }
                    }
                });
            }
        }
        Event::MemoryManifestReady { row } => {
            app.memory_panel.current_manifest = Some(row);
            app.memory_panel.manifest_selected_items.clear();
            super::side_panel::refresh_manifest_selected_items(app);
        }
        Event::McpToolCall {
            tool_name,
            duration_ms,
            error,
        } => {
            if let Some(error) = error {
                app.memory_panel.last_error = Some((
                    format!("MCP {tool_name} failed: {error}"),
                    std::time::Instant::now(),
                ));
            } else {
                app.status_message = Some((
                    format!("MCP {tool_name} ({duration_ms} ms)"),
                    std::time::Instant::now(),
                ));
            }
        }
        Event::MemorySearchResults { rows } => {
            if matches!(
                app.memory_panel.focused,
                crate::panels::memory_panel::PanelSection::Search
            ) && app.memory_panel.search_active
            {
                app.memory_panel.search_results = rows;
                app.memory_panel.search_cursor = 0;
            } else {
                // Reuse the same event for bootstrap into the Recent
                // Written section (controller::Event::MemoryWriteCommitted
                // piggy-backs on this shape).
                app.memory_panel.recent_rows = rows;
                app.memory_panel.recent_cursor = 0;
            }
        }
        Event::MemoryHistoryRows { rows } => {
            app.memory_panel.history_rows = rows;
            app.memory_panel.history_cursor = 0;
        }
        Event::MemorySelectedItems { rows } => {
            app.memory_panel.manifest_selected_items = rows;
            app.memory_panel.injected_cursor = 0;
        }
        Event::MemoryScopeSummary { rows } => {
            app.memory_panel.scope_summary = rows;
        }
        Event::ChatMemoryInjected {
            conv_id,
            items_injected,
            pool_size,
            tokens_used,
            token_budget,
        } => {
            tracing::info!(
                target: "memory_chat",
                conv_id = %conv_id,
                items = items_injected,
                pool = pool_size,
                tokens_used,
                token_budget,
                "chat memory injected"
            );
            if items_injected > 0 {
                app.status_message = Some((
                    format!(
                        "Memory: injected {items_injected} / {pool_size} items (~{tokens_used} tok)"
                    ),
                    std::time::Instant::now(),
                ));
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
            app.swarm_dashboard
                .set_tier_dispatch(&unit_id, tier, &backend);
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
            app.swarm_dashboard.status_message =
                format!("Plan saved: {} — review and /run it", rel);
            app.chat_state.add_system_message(&format!(
                "Plan saved to `{}`.\nReview it (it's open in the editor), then run it with:\n  /run {}",
                rel, rel,
            ));
            app.open_file(&plan_path);
            app.focus = Focus::Editor;
        }
        Event::MemoryReady(stores) => {
            let workspace_root = app
                .workspace
                .roots()
                .first()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let extractor_enabled = app
                .workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_EXTRACTOR_ENABLED,
                    Some(&workspace_root),
                )
                .as_bool()
                .unwrap_or(true);
            let extractor_model = app
                .workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_EXTRACTOR_MODEL,
                    Some(&workspace_root),
                )
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| app.chat_state.effective_model().to_string());
            let llm: Option<std::sync::Arc<dyn gaviero_core::memory::ConsolidationLlm>> =
                if extractor_enabled {
                    match gaviero_core::swarm::backend::shared::create_backend_for_model(
                        &extractor_model,
                        Some(&app.chat_state.agent_settings.ollama_base_url),
                    ) {
                        Ok(backend) => {
                            let backend: std::sync::Arc<
                                dyn gaviero_core::swarm::backend::AgentBackend,
                            > = backend.into();
                            Some(std::sync::Arc::new(
                                gaviero_core::memory::BackendConsolidationLlm::new(
                                    backend,
                                    workspace_root.clone(),
                                ),
                            ))
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "memory_extractor",
                                model = %extractor_model,
                                error = %e,
                                "extractor backend disabled"
                            );
                            None
                        }
                    }
                } else {
                    None
                };

            // A4: observers fan out to the memory panel so it can
            // refresh live on writes and manifest persists.
            let memory_observer: std::sync::Arc<dyn gaviero_core::memory::MemoryObserver> =
                std::sync::Arc::new(super::observers::TuiMemoryObserver {
                    tx: app.event_tx.clone(),
                });
            let manifest_observer: std::sync::Arc<
                dyn gaviero_core::memory::observer::ManifestObserver,
            > = std::sync::Arc::new(super::observers::TuiManifestObserver {
                tx: app.event_tx.clone(),
            });
            let writer =
                gaviero_core::memory::spawn_writer_task(gaviero_core::memory::WriterConfig {
                    stores: stores.clone(),
                    llm,
                    observer: Some(memory_observer),
                    manifest_observer: Some(manifest_observer),
                });

            // B4: apply recency-floor / decay-exempt-types overrides
            // from workspace settings to every store in the registry.
            // Settings are per-MemoryStore (not registry-level), so
            // fan out across global + workspace + every opened folder.
            let recency_floor = app
                .workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_SCORING_RECENCY_FLOOR,
                    Some(&workspace_root),
                )
                .as_f64()
                .map(|v| v as f32);
            let exempt_types = app
                .workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_SCORING_DECAY_EXEMPT_TYPES,
                    Some(&workspace_root),
                )
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(gaviero_core::memory::MemoryType::parse_str)
                        .collect::<Vec<_>>()
                });
            if recency_floor.is_some() || exempt_types.is_some() {
                let stores_for_cfg = stores.clone();
                let recency = recency_floor;
                let types = exempt_types;
                tokio::spawn(async move {
                    for store in stores_for_cfg.opened_stores().await {
                        if let Some(floor) = recency {
                            store.set_recency_floor(floor);
                        }
                        if let Some(ref t) = types {
                            store.set_decay_exempt_types(t.clone());
                        }
                    }
                });
            }

            // B2: spin up the cross-encoder reranker when settings ask
            // for it. Loading the ONNX model is blocking + downloads
            // the file on first run, so we offload to spawn_blocking.
            let rerank_enabled = app
                .workspace
                .resolve_setting(
                    gaviero_core::workspace::settings::MEMORY_RERANKER_ENABLED,
                    Some(&workspace_root),
                )
                .as_bool()
                .unwrap_or(false);
            if rerank_enabled {
                let model_name = app
                    .workspace
                    .resolve_setting(
                        gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
                        Some(&workspace_root),
                    )
                    .as_str()
                    .unwrap_or("none")
                    .to_string();
                let pool_size = app
                    .workspace
                    .resolve_setting(
                        gaviero_core::workspace::settings::MEMORY_RERANKER_POOL_SIZE,
                        Some(&workspace_root),
                    )
                    .as_u64()
                    .unwrap_or(50) as usize;
                let blend_weight = app
                    .workspace
                    .resolve_setting(
                        gaviero_core::workspace::settings::MEMORY_RERANKER_BLEND_WEIGHT,
                        Some(&workspace_root),
                    )
                    .as_f64()
                    .unwrap_or(0.6) as f32;
                let max_latency_ms = app
                    .workspace
                    .resolve_setting(
                        gaviero_core::workspace::settings::MEMORY_RERANKER_MAX_LATENCY_MS,
                        Some(&workspace_root),
                    )
                    .as_u64()
                    .unwrap_or(200);

                let cfg = gaviero_core::memory::RerankConfig {
                    enabled: true,
                    pool_size,
                    blend_weight,
                    max_latency_ms,
                };
                let model_for_load = model_name.clone();
                match tokio::task::block_in_place(|| {
                    gaviero_core::memory::build_reranker(&model_for_load)
                }) {
                    Ok(Some(rr)) => {
                        let arc: std::sync::Arc<dyn gaviero_core::memory::Reranker> = rr;
                        // B2: amortise the ~200ms first-load cost by
                        // running a single dummy pair through the
                        // reranker now, off the event-loop thread, so
                        // the first real query doesn't pay it.
                        let arc_for_warmup = arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = arc_for_warmup.warmup().await {
                                tracing::warn!(
                                    target: "memory_rerank",
                                    error = %e,
                                    "rerank warmup failed; first query will pay load cost"
                                );
                            } else {
                                tracing::debug!(
                                    target: "memory_rerank",
                                    "rerank warmup complete"
                                );
                            }
                        });
                        app.memory_reranker = Some(arc);
                        app.memory_rerank_cfg = Some(cfg);
                    }
                    Ok(None) => {
                        tracing::info!(
                            target: "memory_rerank",
                            model = %model_name,
                            "rerank model resolved to none — falling back to composite"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "memory_rerank",
                            model = %model_name,
                            error = %e,
                            "failed to load reranker model — falling back to composite"
                        );
                    }
                }
            }

            app.memory = Some(stores.clone());
            app.memory_writer = Some(writer);

            // B1: detect stale `_gaviero_meta.embedder_model` stamps
            // across every opened store and surface a chat hint per
            // mismatched DB. With the multi-DB registry, a user could
            // end up with workspace and folder DBs at different
            // embedder versions; we report each independently.
            {
                let stores_for_check = stores.clone();
                let tx = app.event_tx.clone();
                tokio::spawn(async move {
                    for mismatch in stores_for_check.detect_mismatches().await {
                        let db = mismatch
                            .db_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<in-memory>".to_string());
                        let _ = tx.send(Event::MessageComplete {
                            conv_id: String::new(),
                            role: "system".to_string(),
                            content: format!(
                                "Memory DB `{db}` uses embedder `{}` but `{}` is now configured. \
                                 Run `/reembed` to migrate (a `.bak-<ts>` is taken first; \
                                 rollback = restore the bak and revert the setting).",
                                mismatch.stored, mismatch.configured,
                            ),
                        });
                    }
                });
            }

            // A5: spawn the MCP server on the workspace's Unix socket.
            // Disabled if `mcp.gavieroServer.enabled = false`; failures
            // are logged but don't prevent the rest of Gaviero from
            // working (plan §A5: graceful degradation).
            let mcp_enabled = app
                .workspace
                .resolve_setting(gaviero_core::workspace::settings::MCP_GAVIERO_ENABLED, None)
                .as_bool()
                .unwrap_or(true);
            if mcp_enabled {
                let workspace_root_for_mcp = app
                    .workspace
                    .roots()
                    .first()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let socket_path = workspace_root_for_mcp.join(".gaviero/mcp.sock");
                let disabled_external_summary = {
                    let disable_external = app
                        .workspace
                        .resolve_setting(
                            gaviero_core::workspace::settings::MCP_GAVIERO_DISABLE_EXTERNAL,
                            None,
                        )
                        .as_bool()
                        .unwrap_or(true);
                    if disable_external {
                        let paths = gaviero_core::mcp::external_memory::candidate_config_paths(
                            &workspace_root_for_mcp,
                        );
                        match gaviero_core::mcp::external_memory::disable_external_memory_servers(
                            &paths,
                        ) {
                            Ok(hits) if !hits.is_empty() => Some(
                                hits.iter()
                                    .map(|h| h.source_tag)
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ),
                            Ok(_) => None,
                            Err(e) => {
                                tracing::warn!(
                                    target: "mcp_server",
                                    error = %e,
                                    "failed to disable external memory MCP server(s)"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    }
                };
                let mcp_retrieval_cfg = app
                    .workspace
                    .resolve_retrieval_config(Some(&workspace_root_for_mcp));
                let mcp_rerank_cfg = app
                    .workspace
                    .resolve_rerank_config(Some(&workspace_root_for_mcp));
                let mcp_specificity = app
                    .workspace
                    .resolve_specificity_config(Some(&workspace_root_for_mcp));
                let mcp_edge_weights = app
                    .workspace
                    .resolve_all_edge_weights(Some(&workspace_root_for_mcp));
                let server = gaviero_core::mcp::GavieroMcpServer::new(
                    stores.clone(),
                    workspace_root_for_mcp.clone(),
                    std::sync::Arc::new(super::observers::TuiMcpObserver {
                        tx: app.event_tx.clone(),
                    }),
                    mcp_retrieval_cfg,
                    mcp_rerank_cfg,
                    app.memory_reranker.clone(),
                )
                .with_specificity(mcp_specificity)
                .with_edge_weights(mcp_edge_weights);
                match gaviero_core::mcp::spawn_mcp_server(server, &socket_path) {
                    Ok(handle) => {
                        tracing::info!(
                            target: "mcp_server",
                            socket = %handle.socket_path.display(),
                            "mcp server listening"
                        );
                        app.mcp_server = Some(handle);
                        let shim_binary = app
                            .workspace
                            .resolve_setting(
                                gaviero_core::workspace::settings::MCP_GAVIERO_SHIM_BINARY,
                                Some(&workspace_root_for_mcp),
                            )
                            .as_str()
                            .unwrap_or("gaviero-mcp-shim")
                            .to_string();
                        let codex_trust = match app
                            .workspace
                            .resolve_setting(
                                gaviero_core::workspace::settings::MCP_GAVIERO_CODEX_TRUST,
                                Some(&workspace_root_for_mcp),
                            )
                            .as_str()
                            .unwrap_or("unknown")
                        {
                            "granted" | "trusted" => gaviero_core::mcp::TrustConsent::Granted,
                            "denied" | "untrusted" => gaviero_core::mcp::TrustConsent::Denied,
                            _ => gaviero_core::mcp::TrustConsent::Unknown,
                        };
                        let synth = gaviero_core::mcp::McpConfigSynth {
                            worktree: workspace_root_for_mcp.clone(),
                            socket_path: socket_path.clone(),
                            shim_binary,
                            codex_trust,
                            enabled: true,
                        };
                        if let Err(e) = gaviero_core::mcp::synthesize_for_worktree(&synth) {
                            tracing::warn!(
                                target: "mcp_server",
                                error = %e,
                                "failed to synthesize workspace MCP config"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "mcp_server",
                            error = %e,
                            "mcp server failed to start — falling back to prompt-time injection only"
                        );
                    }
                }
                if let Some(summary) = disabled_external_summary {
                    app.chat_state.add_system_message(&format!(
                        "External memory MCP server(s) disabled: {summary}. \
                         Backup config files were written next to the originals."
                    ));
                }
            }

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
                    inst.screen_mut()
                        .set_scrollback(current.saturating_sub(page));
                }
                return;
            }
            Action::SelectUp => {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    let (rows, cols) = inst.screen().size();
                    app.terminal_selection.select_kb((-1, 0), rows, cols);
                }
                return;
            }
            Action::SelectDown => {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    let (rows, cols) = inst.screen().size();
                    app.terminal_selection.select_kb((1, 0), rows, cols);
                }
                return;
            }
            Action::SelectLeft => {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    let (rows, cols) = inst.screen().size();
                    app.terminal_selection.select_kb((0, -1), rows, cols);
                }
                return;
            }
            Action::SelectRight => {
                if let Some(inst) = app.terminal_manager.active_instance() {
                    let (rows, cols) = inst.screen().size();
                    app.terminal_selection.select_kb((0, 1), rows, cols);
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
        Action::SetSideModeMemory => {
            app.panel_visible.side_panel = true;
            app.side_panel = SidePanelMode::MemoryPanel;
            app.focus = Focus::SidePanel;
            app.refresh_memory_panel();
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
            } else if app.focus == Focus::SidePanel && app.side_panel == SidePanelMode::AgentChat {
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
            SidePanelMode::MemoryPanel => app.handle_memory_panel_action(action),
        },
        _ => {}
    }
}
