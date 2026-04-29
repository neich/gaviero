use super::*;

pub(super) fn handle_swarm_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let task_desc = input
        .trim()
        .strip_prefix("/swarm")
        .unwrap_or("")
        .trim()
        .to_string();

    if task_desc.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /swarm <task description>\n\
             Plans and executes a multi-agent task with git worktree isolation.\n\
             Example: /swarm Refactor the auth module and update all tests",
        );
        return;
    }

    // First-swarm-run Codex trust prompt. If the user has never
    // answered, stash the task and open the modal — the dialog handler
    // replays `run_swarm` after the answer persists.
    if should_prompt_codex_trust(app) {
        app.codex_trust_dialog = Some(super::state::CodexTrustDialog {
            pending_task: task_desc,
        });
        return;
    }

    run_swarm(app, task_desc);
}

/// Returns `true` when the Codex trust consent has not yet been given
/// or denied for this workspace — i.e. the value resolves to
/// `"unknown"` (the hardcoded default).
fn should_prompt_codex_trust(app: &App) -> bool {
    use gaviero_core::workspace::settings as S;
    let root = app.workspace.roots().first().map(|p| p.to_path_buf());
    let value = app
        .workspace
        .resolve_setting(S::MCP_GAVIERO_CODEX_TRUST, root.as_deref());
    matches!(value.as_str(), Some("unknown"))
}

fn mcp_config_for_workspace(
    app: &App,
    root: &std::path::Path,
) -> gaviero_core::mcp::McpConfigSynth {
    use gaviero_core::workspace::settings as S;

    let enabled = app
        .workspace
        .resolve_setting(S::MCP_GAVIERO_ENABLED, Some(root))
        .as_bool()
        .unwrap_or(true);
    let shim_binary = app
        .workspace
        .resolve_setting(S::MCP_GAVIERO_SHIM_BINARY, Some(root))
        .as_str()
        .unwrap_or("gaviero-mcp-shim")
        .to_string();
    let codex_trust = match app
        .workspace
        .resolve_setting(S::MCP_GAVIERO_CODEX_TRUST, Some(root))
        .as_str()
        .unwrap_or("unknown")
    {
        "granted" | "trusted" => gaviero_core::mcp::TrustConsent::Granted,
        "denied" | "untrusted" => gaviero_core::mcp::TrustConsent::Denied,
        _ => gaviero_core::mcp::TrustConsent::Unknown,
    };

    gaviero_core::mcp::McpConfigSynth {
        worktree: root.to_path_buf(),
        socket_path: root.join(".gaviero/mcp.sock"),
        shim_binary,
        codex_trust,
        enabled,
    }
}

pub(crate) fn run_swarm(app: &mut App, task_desc: String) {
    app.chat_state
        .add_user_message(&format!("/swarm {}", task_desc));
    app.chat_state.add_system_message(&format!(
        "Planning swarm task: {}\nSwitch to SWARM panel (Ctrl+Shift+P) to monitor progress.",
        task_desc
    ));

    app.side_panel = SidePanelMode::SwarmDashboard;
    app.panel_visible.side_panel = true;
    app.swarm_dashboard.reset("planning");
    app.swarm_dashboard.status_message = format!(
        "Planning task: {}...",
        task_desc.chars().take(60).collect::<String>()
    );

    let tx = app.event_tx.clone();
    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let model = app.chat_state.effective_model().to_string();
    let ollama_base_url = app.chat_state.agent_settings.ollama_base_url.clone();
    let write_ns = app.chat_state.agent_settings.write_namespace.clone();
    let read_ns = app.chat_state.agent_settings.read_namespaces.clone();
    let memory = app.memory.clone();
    let memory_writer = app.memory_writer.clone();
    let mcp_config = mcp_config_for_workspace(app, &root);
    let excludes = parse_exclude_patterns(&app.workspace, Some(&root));
    let specificity = app.workspace.resolve_specificity_config(Some(&root));
    let (swarm_extra_tools, _) = app.workspace.resolve_agent_tools(Some(&root));

    tokio::spawn(async move {
        use gaviero_core::swarm::{pipeline, planner};

        let memory_ctx = if let Some(ref mem) = memory {
            mem.workspace()
                .search_context(&read_ns, &task_desc, 5)
                .await
        } else {
            String::new()
        };

        let file_list = list_workspace_files(&root, 200, &excludes);
        let work_units = match planner::plan_task(
            &task_desc,
            &root,
            &model,
            Some(&ollama_base_url),
            &file_list,
            &memory_ctx,
        )
        .await
        {
            Ok(units) => units,
            Err(e) => {
                let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("Swarm planning failed: {}", e),
                });
                return;
            }
        };

        let unit_count = work_units.len();
        let _ = tx.send(Event::SwarmPhaseChanged(format!(
            "planned ({} agents)",
            unit_count
        )));

        let plan = gaviero_core::swarm::plan::CompiledPlan::from_work_units(work_units, None);
        let config = pipeline::SwarmConfig {
            max_parallel: unit_count.min(4),
            workspace_root: root,
            model: model.clone(),
            ollama_base_url: Some(ollama_base_url),
            use_worktrees: unit_count > 1,
            read_namespaces: read_ns,
            write_namespace: write_ns,
            context_files: vec![],
            excludes: vec![],
            memory_writer,
            mcp_config: Some(mcp_config),
            specificity,
            swarm_extra_tools,
        };

        let observer = TuiSwarmObserver { tx: tx.clone() };
        let tx2 = tx.clone();
        let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
            Box::new(TuiAcpObserver {
                tx: tx2.clone(),
                conv_id: format!("swarm-{}", agent_id),
            })
        };

        match pipeline::execute(&plan, &config, None, memory, &observer, make_obs).await {
            Ok(result) => {
                let _ = tx.send(Event::SwarmCompleted(Box::new(result)));
            }
            Err(e) => {
                let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("Swarm execution failed: {}", e),
                });
            }
        }
    });
}

pub(super) fn handle_run_script_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let rest = input.trim().strip_prefix("/run").unwrap_or("").trim();

    let (raw_path_token, runtime_prompt) = match rest.find(|c: char| c.is_ascii_whitespace()) {
        Some(idx) => {
            let path_tok = &rest[..idx];
            let remainder = rest[idx..].trim();
            let prompt = if remainder.is_empty() {
                None
            } else {
                Some(remainder.to_string())
            };
            (path_tok, prompt)
        }
        None => (rest, None),
    };

    let script_path = raw_path_token
        .strip_prefix('@')
        .unwrap_or(raw_path_token)
        .to_string();

    if script_path.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /run <path.gaviero> [prompt]\n\
             Compiles and executes a .gaviero DSL script.\n\
             Use {{PROMPT}} in agent prompts for runtime substitution.\n\
             Example: /run workflows/security_audit.gaviero\n\
             Example: /run @workflows/tdd.gaviero implement OAuth login",
        );
        return;
    }

    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let resolved = if std::path::Path::new(&script_path).is_absolute() {
        std::path::PathBuf::from(&script_path)
    } else {
        root.join(&script_path)
    };

    let raw = match std::fs::read_to_string(&resolved) {
        Ok(s) => s,
        Err(e) => {
            app.chat_state.add_system_message(&format!(
                "Cannot read {}: {}",
                resolved.display(),
                e
            ));
            return;
        }
    };

    let source = extract_gaviero_block(&raw);
    let filename = resolved.display().to_string();
    let compiled = match gaviero_dsl::compile(&source, &filename, None, runtime_prompt.as_deref()) {
        Ok(c) => c,
        Err(report) => {
            app.chat_state
                .add_system_message(&format!("DSL compilation failed:\n{}", report));
            return;
        }
    };

    let unit_count = compiled.graph.node_count();
    let display_cmd = match &runtime_prompt {
        Some(rp) => format!("/run {} {}", script_path, rp),
        None => format!("/run {}", script_path),
    };
    app.chat_state.add_user_message(&display_cmd);
    app.chat_state.add_system_message(&format!(
        "Compiled {} -> {} agent(s). Executing...\n\
         Switch to SWARM panel (Ctrl+Shift+P) to monitor progress.",
        script_path, unit_count
    ));

    app.side_panel = SidePanelMode::SwarmDashboard;
    app.panel_visible.side_panel = true;
    app.swarm_dashboard.reset("compiled");
    app.swarm_dashboard.status_message = format!("Script: {} ({} agents)", script_path, unit_count);

    let tx = app.event_tx.clone();
    let model = app.chat_state.effective_model().to_string();
    let ollama_base_url = app.chat_state.agent_settings.ollama_base_url.clone();
    let write_ns = app.chat_state.agent_settings.write_namespace.clone();
    let read_ns = app.chat_state.agent_settings.read_namespaces.clone();
    let memory = app.memory.clone();
    let memory_writer = app.memory_writer.clone();
    let mcp_config = mcp_config_for_workspace(app, &root);
    let specificity = app.workspace.resolve_specificity_config(Some(&root));
    let (swarm_extra_tools, _) = app.workspace.resolve_agent_tools(Some(&root));

    tokio::spawn(async move {
        use gaviero_core::swarm::pipeline;

        let effective_max_parallel = compiled.max_parallel.unwrap_or_else(|| unit_count.min(4));

        let config = pipeline::SwarmConfig {
            max_parallel: effective_max_parallel,
            workspace_root: root,
            model,
            ollama_base_url: Some(ollama_base_url),
            use_worktrees: effective_max_parallel > 1,
            read_namespaces: read_ns,
            write_namespace: write_ns,
            context_files: vec![],
            excludes: vec![],
            memory_writer,
            mcp_config: Some(mcp_config),
            specificity,
            swarm_extra_tools,
        };

        let observer = TuiSwarmObserver { tx: tx.clone() };
        let tx2 = tx.clone();
        let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
            Box::new(TuiAcpObserver {
                tx: tx2.clone(),
                conv_id: format!("swarm-{}", agent_id),
            })
        };

        match pipeline::execute(&compiled, &config, None, memory, &observer, make_obs).await {
            Ok(result) => {
                let _ = tx.send(Event::SwarmCompleted(Box::new(result)));
            }
            Err(e) => {
                let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("Script execution failed: {}", e),
                });
            }
        }
    });
}

pub(super) fn handle_coordinated_swarm_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let task_desc = input
        .trim()
        .strip_prefix("/cswarm")
        .unwrap_or("")
        .trim()
        .to_string();

    if task_desc.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /cswarm <task description>\n\
             Coordinated tier-routed swarm with provider-aware planning and execution.\n\
             Example: /cswarm Refactor the auth module to use the strategy pattern",
        );
        return;
    }

    app.chat_state
        .add_user_message(&format!("/cswarm {}", task_desc));
    app.chat_state.add_system_message(&format!(
        "Coordinated swarm: {}\nThe coordinator will produce a .gaviero plan file for review.\n\
         Switch to SWARM panel (Ctrl+Shift+P) to monitor.",
        task_desc
    ));

    app.side_panel = SidePanelMode::SwarmDashboard;
    app.panel_visible.side_panel = true;
    app.swarm_dashboard.reset("coordinating");
    app.swarm_dashboard.status_message = format!(
        "Coordinator planning (DSL): {}...",
        task_desc.chars().take(60).collect::<String>()
    );

    let tx = app.event_tx.clone();
    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let (task_desc, context_files) = {
        use crate::panels::agent_chat::parse_file_references;
        let refs = parse_file_references(&task_desc);
        let mut enriched = task_desc.clone();
        let mut ctx_files: Vec<(String, String)> = Vec::new();
        let all_roots: Vec<std::path::PathBuf> = {
            let r: Vec<_> = app.workspace.roots().iter().map(|p| p.to_path_buf()).collect();
            if r.is_empty() { vec![root.clone()] } else { r }
        };
        for rel_path in &refs {
            let found = all_roots.iter().find_map(|r| {
                std::fs::read_to_string(r.join(rel_path)).ok()
            });
            if let Some(content) = found {
                let tag = format!("@{}", rel_path);
                let replacement = format!(
                    "\n[File: {}]\n{}\n[End of file: {}]",
                    rel_path, content, rel_path
                );
                enriched = enriched.replace(&tag, &replacement);
                ctx_files.push((rel_path.clone(), content));
                tracing::debug!(
                    "Inlined @{} into cswarm prompt ({} bytes)",
                    rel_path,
                    ctx_files.last().unwrap().1.len()
                );
            } else {
                tracing::warn!("Could not read @{} for cswarm prompt", rel_path);
            }
        }
        (enriched, ctx_files)
    };
    let write_ns = app.chat_state.agent_settings.write_namespace.clone();
    let read_ns = app.chat_state.agent_settings.read_namespaces.clone();
    let ollama_base_url = app.chat_state.agent_settings.ollama_base_url.clone();
    // Coordinator model resolution:
    //   1. explicit agent.coordinator.model setting
    //   2. chat's effective model (honors per-conv override + agent.model workspace setting)
    // This lets users set `agent.model = codex:gpt-5-codex` and have swarm coordination
    // route through Codex too, without a separate coordinator setting.
    let coordinator_model = app
        .workspace
        .resolve_setting(gaviero_core::workspace::settings::COORDINATOR_MODEL, None)
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| app.chat_state.effective_model().to_string());
    let memory = app.memory.clone();
    let memory_writer = app.memory_writer.clone();
    let mcp_config = mcp_config_for_workspace(app, &root);
    let specificity = app.workspace.resolve_specificity_config(Some(&root));
    let (swarm_extra_tools, _) = app.workspace.resolve_agent_tools(Some(&root));

    tokio::spawn(async move {
        use gaviero_core::swarm::{coordinator, pipeline};

        let config = pipeline::SwarmConfig {
            max_parallel: 4,
            workspace_root: root.clone(),
            model: coordinator_model.clone(),
            ollama_base_url: Some(ollama_base_url.clone()),
            use_worktrees: true,
            read_namespaces: read_ns,
            write_namespace: write_ns,
            context_files,
            excludes: vec![],
            memory_writer,
            mcp_config: Some(mcp_config),
            specificity,
            swarm_extra_tools,
        };

        let coord_config = coordinator::CoordinatorConfig {
            model: coordinator_model,
            ollama_base_url: Some(ollama_base_url),
            ..Default::default()
        };

        let observer = TuiSwarmObserver { tx: tx.clone() };
        let tx2 = tx.clone();
        let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
            Box::new(TuiAcpObserver {
                tx: tx2.clone(),
                conv_id: format!("swarm-{}", agent_id),
            })
        };

        match pipeline::plan_coordinated(
            &task_desc,
            &config,
            coord_config,
            memory,
            &observer,
            make_obs,
        )
        .await
        {
            Ok(dsl_text) => {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let plan_filename = format!("gaviero_plan_{}.gaviero", timestamp);
                let plan_path = root.join("tmp").join(&plan_filename);
                if let Err(e) = std::fs::create_dir_all(plan_path.parent().unwrap()) {
                    let _ = tx.send(Event::MessageComplete {
                        conv_id: String::new(),
                        role: "system".to_string(),
                        content: format!("Failed to create tmp/ directory: {}", e),
                    });
                    return;
                }
                match std::fs::write(&plan_path, &dsl_text) {
                    Ok(()) => {
                        let _ = tx.send(Event::SwarmDslPlanReady(plan_path));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::MessageComplete {
                            conv_id: String::new(),
                            role: "system".to_string(),
                            content: format!("Failed to write plan file: {}", e),
                        });
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("Coordinated swarm planning failed: {}", e),
                });
            }
        }
    });
}

pub(super) fn handle_undo_swarm_command(app: &mut App) {
    app.chat_state.take_input();

    let has_result = app
        .swarm_dashboard
        .result
        .as_ref()
        .map(|r| !r.pre_swarm_sha.is_empty())
        .unwrap_or(false);

    if !has_result {
        app.chat_state
            .add_system_message("No undoable swarm result found. Run /cswarm first.");
        return;
    }

    app.side_panel = SidePanelMode::SwarmDashboard;
    app.panel_visible.side_panel = true;
    app.focus = Focus::SidePanel;
    app.swarm_dashboard.pending_undo_confirm = true;
    app.chat_state
        .add_system_message("Swarm dashboard: press u to confirm undo all changes, Esc to cancel.");
}

/// A2: resolve the scope a `/remember` invocation targets.
///
/// * `/remember` → user's configured default (`memory.remember.defaultScope`)
/// * `/remember-here` → Run (session-local)
/// * `/remember-module` → Module (requires active file; else Repo with a note)
/// * `/remember-workspace` → Workspace
/// * `/remember-global` → Global
///
/// Returns `Ok((scope, variant_label, module_fallback_note))`. The
/// caller emits `note` if non-empty to teach the user about the
/// fallback.
fn resolve_remember_scope(
    app: &App,
    variant: &str,
) -> Result<
    (
        gaviero_core::memory::WriteScope,
        &'static str,
        Option<String>,
    ),
    String,
> {
    use gaviero_core::memory::{WriteScope, hash_path};
    use gaviero_core::workspace::settings as S;

    let workspace_root = app
        .workspace
        .roots()
        .first()
        .cloned()
        .ok_or_else(|| "no workspace root".to_string())?;
    let repo_id = hash_path(&workspace_root);

    match variant {
        "here" => {
            let run_id = app.chat_state.conversations[app.chat_state.active_conv]
                .id
                .clone();
            Ok((WriteScope::Run { repo_id, run_id }, "Run", None))
        }
        "module" => {
            let module_path = app
                .buffers
                .get(app.active_buffer)
                .and_then(|b| b.path.as_ref())
                .and_then(|p| p.strip_prefix(&workspace_root).ok())
                .and_then(|rel| rel.parent().map(|p| p.to_string_lossy().to_string()))
                .filter(|s| !s.is_empty());
            match module_path {
                Some(m) => Ok((
                    WriteScope::Module {
                        repo_id,
                        module_path: m,
                    },
                    "Module",
                    None,
                )),
                None => Ok((
                    WriteScope::Repo { repo_id },
                    "Repo",
                    Some(
                        "ℹ Module scope requires a focused file; stored at [Repo] instead."
                            .to_string(),
                    ),
                )),
            }
        }
        "workspace" => Ok((WriteScope::Workspace, "Workspace", None)),
        "global" => Ok((WriteScope::Global, "Global", None)),
        "" => {
            // Default variant — read from settings.
            let default = app
                .workspace
                .resolve_setting(S::MEMORY_REMEMBER_DEFAULT_SCOPE, Some(&workspace_root))
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "repo".to_string());
            match default.to_ascii_lowercase().as_str() {
                "run" => {
                    let run_id = app.chat_state.conversations[app.chat_state.active_conv]
                        .id
                        .clone();
                    Ok((WriteScope::Run { repo_id, run_id }, "Run", None))
                }
                "module" => resolve_remember_scope(app, "module"),
                "workspace" => Ok((WriteScope::Workspace, "Workspace", None)),
                "global" => Ok((WriteScope::Global, "Global", None)),
                _ => Ok((WriteScope::Repo { repo_id }, "Repo", None)),
            }
        }
        other => Err(format!("unknown /remember variant: {other}")),
    }
}

/// Tier B / B5: `/consolidate-session` — manual end-of-session
/// consolidator trigger. Pulls the last N turns of transcript out of
/// the active conversation, stamps the session's repo + run ids, and
/// hands the lot to the writer task. Result lands as a system message.
pub(super) fn handle_consolidate_session_command(app: &mut App) {
    let _ = app.chat_state.take_input();
    app.chat_state.add_user_message("/consolidate-session");

    let Some(writer) = app.memory_writer.clone() else {
        app.chat_state
            .add_system_message("Memory writer not initialised; cannot consolidate.");
        return;
    };

    let conv_id = app.chat_state.conversations[app.chat_state.active_conv]
        .id
        .clone();
    let workspace_root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let repo_id = gaviero_core::memory::hash_path(&workspace_root);

    // Concat the conversation's visible turn texts as a coarse
    // transcript. Good enough for an explicit /consolidate-session
    // trigger; the idle-trigger path (Tier B5 follow-up) uses the same
    // helper.
    let transcript: String = app.chat_state.conversations[app.chat_state.active_conv]
        .messages
        .iter()
        .map(|m| format!("{:?}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        let res = writer
            .session_consolidate(conv_id.clone(), repo_id, None, conv_id.clone(), transcript)
            .await;
        let msg = match res {
            Ok(_) => "Session consolidator: queued for processing.".to_string(),
            Err(e) => format!("Session consolidator failed: {e}"),
        };
        let _ = tx.send(Event::MessageComplete {
            conv_id: String::new(),
            role: "system".to_string(),
            content: msg,
        });
    });
}

/// Tier B / B5: `/sleep [--dry-run]` — kick off the sleeptime pass.
/// Fire-and-forget; the writer task handles audit + observer events.
pub(super) fn handle_sleep_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let trimmed = input.trim();
    let dry_run = trimmed.contains("--dry-run");
    app.chat_state.add_user_message(trimmed);

    let Some(writer) = app.memory_writer.clone() else {
        app.chat_state
            .add_system_message("Memory writer not initialised; cannot run sleeptime.");
        return;
    };
    let payload = serde_json::json!({ "dry_run": dry_run });
    if let Err(e) = writer.sleeptime(payload) {
        app.chat_state
            .add_system_message(&format!("Sleeptime enqueue failed: {e}"));
    } else {
        app.chat_state.add_system_message(if dry_run {
            "Sleeptime queued (dry-run; no writes will land)."
        } else {
            "Sleeptime queued."
        });
    }
}

/// Tier C / C2.3: `/forget`, `/forget-scope`, `/forget-type`,
/// `/forget-source` — bulk soft-delete with mandatory two-shot
/// confirmation (`--yes` flag) and `--dry-run` preview. Always tagged
/// `DeletedBy::UserCommand`; the audit row carries the optional
/// `--reason` text. Never matches history rows.
pub(super) fn handle_forget_command(app: &mut App) {
    use gaviero_core::memory::ForgetFilter;
    use gaviero_core::memory::scope::MemoryType;
    use gaviero_core::memory::trust_defaults::MemorySource;

    let input = app.chat_state.take_input();
    let trimmed = input.trim();
    app.chat_state.add_user_message(trimmed);

    let (variant, rest) = if let Some(r) = trimmed.strip_prefix("/forget-scope") {
        ("scope", r)
    } else if let Some(r) = trimmed.strip_prefix("/forget-type") {
        ("type", r)
    } else if let Some(r) = trimmed.strip_prefix("/forget-source") {
        ("source", r)
    } else if let Some(r) = trimmed.strip_prefix("/forget") {
        ("query", r)
    } else {
        return;
    };

    let (flags, args) = parse_forget_flags(rest);
    let confirmed = flags.contains("--yes");
    let dry_run = flags.contains("--dry-run") || !confirmed;

    if args.trim().is_empty() {
        app.chat_state.add_system_message(
            "Usage:\n\
             /forget <query>            — fuzzy match (records and summaries; never history)\n\
             /forget-scope <scope_path> — every row at that scope (e.g. workspace, repo:<id>)\n\
             /forget-type <type>        — factual|procedural|decision|pattern|gotcha|...\n\
             /forget-source <source>    — user_remember|llm_extracted|llm_consolidated|...\n\
             Append --dry-run to preview, --yes to confirm. \
             Optional --reason \"<text>\" attaches to the audit rows.",
        );
        return;
    }

    let (reason, body) = extract_reason_flag(&args);
    let body = body.trim().to_string();

    let filter = match variant {
        "query" => ForgetFilter::ByQuery(body.clone()),
        "scope" => ForgetFilter::ByScope {
            scope_level: scope_level_for_path(&body),
            scope_path: body.clone(),
        },
        "type" => ForgetFilter::ByType(MemoryType::parse_str(body.trim().to_lowercase().as_str())),
        "source" => {
            ForgetFilter::BySource(MemorySource::parse_str(body.trim().to_lowercase().as_str()))
        }
        _ => unreachable!(),
    };

    let Some(writer) = app.memory_writer.clone() else {
        app.chat_state
            .add_system_message("Memory writer not initialised; cannot run /forget.");
        return;
    };

    let tx = app.event_tx.clone();
    let header = if dry_run {
        format!("/forget {variant} (dry-run): \"{body}\"")
    } else {
        format!("/forget {variant} (live): \"{body}\"")
    };
    tokio::spawn(async move {
        let body_msg = match writer.bulk_forget(filter, dry_run, reason).await {
            Ok(report) => format_forget_report(&header, &report, dry_run),
            Err(e) => format!("/forget failed: {e}"),
        };
        let _ = tx.send(Event::MessageComplete {
            conv_id: String::new(),
            role: "system".to_string(),
            content: body_msg,
        });
    });
}

/// Pull recognized flag tokens (`--dry-run`, `--yes`) off the front of
/// `rest`, returning the residual argument text. Multiple flags allowed
/// in any order; unknown `--*` tokens are kept in the body so the
/// caller can complain if they want.
fn parse_forget_flags(rest: &str) -> (std::collections::HashSet<&'static str>, String) {
    let mut flags = std::collections::HashSet::new();
    let mut residual: Vec<&str> = Vec::new();
    for tok in rest.split_whitespace() {
        match tok {
            "--dry-run" => {
                flags.insert("--dry-run");
            }
            "--yes" => {
                flags.insert("--yes");
            }
            other => residual.push(other),
        }
    }
    (flags, residual.join(" "))
}

/// Pull a `--reason "<text>"` argument off `args`. Reason quoting is
/// shell-style: `--reason X` for one-word reasons, `--reason "X Y Z"`
/// for multi-word ones. Returns `(reason, body_without_reason)`.
fn extract_reason_flag(args: &str) -> (Option<String>, String) {
    let s = args.trim();
    let Some(idx) = s.find("--reason") else {
        return (None, s.to_string());
    };
    let before = s[..idx].trim();
    let after = s[idx + "--reason".len()..].trim_start();
    if let Some(rest_q) = after.strip_prefix('"') {
        if let Some(end) = rest_q.find('"') {
            let reason = &rest_q[..end];
            let tail = rest_q[end + 1..].trim();
            let body = if tail.is_empty() {
                before.to_string()
            } else {
                format!("{before} {tail}")
            };
            return (Some(reason.to_string()), body);
        }
    }
    let mut iter = after.split_whitespace();
    let first = iter.next().unwrap_or("").to_string();
    let tail: Vec<&str> = iter.collect();
    let body = if tail.is_empty() {
        before.to_string()
    } else {
        format!("{before} {}", tail.join(" "))
    };
    (Some(first), body)
}

fn scope_level_for_path(path: &str) -> i32 {
    if path == "global" {
        gaviero_core::memory::scope::SCOPE_GLOBAL
    } else if path == "workspace" {
        gaviero_core::memory::scope::SCOPE_WORKSPACE
    } else if path.contains("/run:") {
        gaviero_core::memory::scope::SCOPE_RUN
    } else if path.contains("/module:") {
        gaviero_core::memory::scope::SCOPE_MODULE
    } else {
        gaviero_core::memory::scope::SCOPE_REPO
    }
}

fn format_forget_report(
    header: &str,
    report: &gaviero_core::memory::BulkForgetReport,
    dry_run: bool,
) -> String {
    if report.candidates.is_empty() {
        return format!("{header}\nNothing matched. (Records and summaries only — history is excluded by design.)");
    }
    let kinds: Vec<String> = report
        .kind_breakdown
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect();
    let scopes: Vec<String> = report
        .scope_breakdown
        .iter()
        .map(|(s, n)| format!("{s}={n}"))
        .collect();
    if dry_run {
        format!(
            "{header}\n  matches: {}\n  kinds:   {}\n  scopes:  {}\n  Re-run with --yes to confirm. \
             /restore <audit-id> can undo each row within the retention window.",
            report.candidates.len(),
            kinds.join(", "),
            scopes.join(", "),
        )
    } else {
        format!(
            "{header}\n  deleted: {} (audit ids written; use /restore <id> to undo)\n  kinds:   {}\n  scopes:  {}",
            report.deleted,
            kinds.join(", "),
            scopes.join(", "),
        )
    }
}

/// Tier C / C2.4: `/forget-history` — two-step-confirmed redaction
/// of a single history row. Three chances to back out:
///   1. `/forget-history <memory_id>` (or `--turn <turn_id>`) — prints
///      the row, asks for the id again with `--confirm <id>`.
///   2. `/forget-history --confirm <id>` — asks for the literal word
///      `REDACT` plus a reason, via `--confirm <id> REDACT <reason>`.
///   3. Sends the [`WriterMessage::RedactHistory`] message and
///      reports the audit id.
///
/// Settings gate: `memory.forget.allowHistoryRedaction` (default
/// `true`) — workspaces under strict audit can disable the path
/// entirely.
pub(super) fn handle_forget_history_command(app: &mut App) {
    use gaviero_core::workspace::settings as S;

    let input = app.chat_state.take_input();
    let trimmed = input.trim();
    app.chat_state.add_user_message(trimmed);

    let workspace_root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let allow = app
        .workspace
        .resolve_setting(S::MEMORY_FORGET_ALLOW_HISTORY_REDACTION, Some(&workspace_root))
        .as_bool()
        .unwrap_or(true);
    if !allow {
        app.chat_state.add_system_message(
            "/forget-history is disabled in this workspace \
             (memory.forget.allowHistoryRedaction = false). \
             History rows are immutable.",
        );
        return;
    }

    let rest = trimmed
        .strip_prefix("/forget-history")
        .unwrap_or("")
        .trim()
        .to_string();

    if rest.is_empty() {
        app.chat_state.add_system_message(
            "Usage:\n\
             /forget-history <memory_id>                 — preview the row\n\
             /forget-history --confirm <memory_id>       — second-step prompt\n\
             /forget-history --confirm <memory_id> REDACT <reason>\n\
                                                          — execute the redaction\n\
             Redaction is one-way: the transcript is replaced with a tombstone \
             (sha + timestamp + reason). The row continues to exist; only its \
             content is wiped. Use /forget on derived records if you want \
             reversible deletion instead.",
        );
        return;
    }

    let Some(writer) = app.memory_writer.clone() else {
        app.chat_state
            .add_system_message("Memory writer not initialised; cannot run /forget-history.");
        return;
    };
    let Some(memory) = app.memory.clone() else {
        app.chat_state
            .add_system_message("Memory store not initialised; cannot run /forget-history.");
        return;
    };

    let tx = app.event_tx.clone();
    let words: Vec<&str> = rest.split_whitespace().collect();
    let confirm_idx = words.iter().position(|w| *w == "--confirm");

    // Step 1: preview-only.
    if confirm_idx.is_none() {
        let id_str = words.first().copied().unwrap_or("");
        let Ok(id) = id_str.parse::<i64>() else {
            app.chat_state.add_system_message(
                "/forget-history: expected a numeric memory id (`/forget-history <id>`).",
            );
            return;
        };
        tokio::spawn(async move {
            let body = match memory.workspace().read_history_content(id).await {
                Ok(Some(content)) => format!(
                    "/forget-history (preview) row {id}:\n  {}\n\
                     Type `/forget-history --confirm {id}` to proceed to the second-step prompt. \
                     This redaction CANNOT be undone.",
                    content.lines().take(5).collect::<Vec<_>>().join("\n  ")
                ),
                Ok(None) => format!(
                    "/forget-history: no history row at id {id} (already redacted? wrong id?)"
                ),
                Err(e) => format!("/forget-history preview failed: {e}"),
            };
            let _ = tx.send(Event::MessageComplete {
                conv_id: String::new(),
                role: "system".to_string(),
                content: body,
            });
        });
        return;
    }

    // Step 2/3: confirm form: `--confirm <id> [REDACT <reason...>]`.
    let confirm_idx = confirm_idx.unwrap();
    let id_word = words.get(confirm_idx + 1).copied().unwrap_or("");
    let Ok(id) = id_word.parse::<i64>() else {
        app.chat_state
            .add_system_message("/forget-history --confirm: missing or non-numeric memory id.");
        return;
    };
    let redact_idx = words.iter().position(|w| *w == "REDACT");
    let Some(redact_idx) = redact_idx else {
        // Step 2: print the literal-REDACT prompt and wait for the user.
        app.chat_state.add_system_message(&format!(
            "/forget-history (step 2): about to redact memory id {id}. \
             Type `/forget-history --confirm {id} REDACT <reason>` to proceed. \
             This redaction CANNOT be undone."
        ));
        return;
    };
    let reason = words[redact_idx + 1..].join(" ").trim().to_string();
    if reason.is_empty() {
        app.chat_state.add_system_message(
            "/forget-history --confirm: a non-empty reason is required after REDACT.",
        );
        return;
    }

    tokio::spawn(async move {
        let body = match writer.redact_history(id, reason).await {
            Ok(audit_id) => format!(
                "✓ /forget-history: row {id} redacted; audit {audit_id} written. \
                 The transcript has been permanently replaced with a tombstone."
            ),
            Err(e) => format!("/forget-history failed: {e}"),
        };
        let _ = tx.send(Event::MessageComplete {
            conv_id: String::new(),
            role: "system".to_string(),
            content: body,
        });
    });
}

/// Tier C / C2.2: `/restore <id>` and `/restore --since <duration>`.
/// Replays soft-deleted rows through the dedup pipeline. Refused for
/// `user_redaction` audit rows (see C2.4). `--since` accepts SQLite
/// relative-datetime fragments (`2 hours`, `7 days`, `30 minutes`).
pub(super) fn handle_restore_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let trimmed = input.trim();
    app.chat_state.add_user_message(trimmed);

    let rest = trimmed
        .strip_prefix("/restore")
        .unwrap_or("")
        .trim()
        .to_string();

    if rest.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /restore <deletion-id>\n       /restore --since <N hours|N days|N minutes>",
        );
        return;
    }

    let Some(writer) = app.memory_writer.clone() else {
        app.chat_state
            .add_system_message("Memory writer not initialised; cannot restore.");
        return;
    };

    let tx = app.event_tx.clone();

    if let Some(window) = rest.strip_prefix("--since") {
        let since_offset = match parse_restore_since_window(window.trim()) {
            Ok(s) => s,
            Err(e) => {
                app.chat_state
                    .add_system_message(&format!("/restore --since: {e}"));
                return;
            }
        };
        tokio::spawn(async move {
            let body = match writer.restore_deletions_since(&since_offset).await {
                Ok(outcomes) if outcomes.is_empty() => {
                    "/restore: no soft-deleted rows in that window.".to_string()
                }
                Ok(outcomes) => format_restore_summary(&outcomes),
                Err(e) => format!("/restore --since failed: {e}"),
            };
            let _ = tx.send(Event::MessageComplete {
                conv_id: String::new(),
                role: "system".to_string(),
                content: body,
            });
        });
        return;
    }

    let id: i64 = match rest.parse() {
        Ok(v) => v,
        Err(_) => {
            app.chat_state.add_system_message(
                "/restore: expected a numeric audit id or `--since <duration>`.",
            );
            return;
        }
    };

    tokio::spawn(async move {
        let body = match writer.restore_deletion(id).await {
            Ok(outcome) => format_restore_outcome(&outcome),
            Err(e) => format!("/restore failed: {e}"),
        };
        let _ = tx.send(Event::MessageComplete {
            conv_id: String::new(),
            role: "system".to_string(),
            content: body,
        });
    });
}

/// Parse `/restore --since <N> <unit>` into the SQLite relative-
/// datetime fragment that the store API expects (e.g. `"-2 hours"`).
/// Accepts singular and plural unit names, leading minus is added by
/// us so the user spec stays positive.
fn parse_restore_since_window(spec: &str) -> Result<String, String> {
    let s = spec.trim();
    if s.is_empty() {
        return Err("missing duration (e.g. `2 hours`, `7 days`)".into());
    }
    let mut it = s.split_whitespace();
    let n: u32 = it
        .next()
        .ok_or_else(|| "missing count".to_string())?
        .parse()
        .map_err(|_| "count must be a positive integer".to_string())?;
    let unit_raw = it.next().ok_or_else(|| "missing unit".to_string())?;
    let unit = match unit_raw.trim_end_matches('s') {
        "minute" | "min" => "minutes",
        "hour" | "hr" => "hours",
        "day" => "days",
        other => return Err(format!("unsupported unit `{other}` (use minutes / hours / days)")),
    };
    Ok(format!("-{n} {unit}"))
}

fn format_restore_outcome(o: &gaviero_core::memory::RestoreOutcome) -> String {
    use gaviero_core::memory::RestoreOutcome::*;
    match o {
        Inserted {
            deletion_id,
            new_memory_id,
        } => format!(
            "✓ /restore: audit {deletion_id} reinstated as new memory id {new_memory_id}."
        ),
        Deduplicated {
            deletion_id,
            surviving_memory_id,
        } => format!(
            "✓ /restore: audit {deletion_id} merged into existing memory {surviving_memory_id} (dedup hit)."
        ),
        AlreadyCovered { deletion_id } => format!(
            "✓ /restore: audit {deletion_id} already covered at a broader scope; nothing new written."
        ),
        Refused {
            deletion_id,
            reason,
        } => format!("✗ /restore refused for audit {deletion_id}: {reason}"),
    }
}

fn format_restore_summary(outcomes: &[gaviero_core::memory::RestoreOutcome]) -> String {
    use gaviero_core::memory::RestoreOutcome::*;
    let mut inserted = 0u32;
    let mut deduped = 0u32;
    let mut covered = 0u32;
    let mut refused = 0u32;
    for o in outcomes {
        match o {
            Inserted { .. } => inserted += 1,
            Deduplicated { .. } => deduped += 1,
            AlreadyCovered { .. } => covered += 1,
            Refused { .. } => refused += 1,
        }
    }
    format!(
        "/restore --since: {} processed (inserted {inserted}, deduped {deduped}, covered {covered}, refused {refused}).",
        outcomes.len()
    )
}

/// Tier B / B1: `/reembed` — re-embed every memory under the currently
/// configured embedder. Runs on a tokio task so the TUI stays
/// responsive; takes a `.bak-<ts>` of `memory.db` before mutating
/// (mandatory rollback path), then streams progress as system messages.
///
/// The configured embedder comes from `memory.embedder.model`; if it
/// matches what's already stamped in `_gaviero_meta.embedder_model`,
/// the run still goes through but every row hits the
/// `current_model == new_model_id` skip path inside `reembed_all`.
pub(super) fn handle_reembed_command(app: &mut App) {
    use gaviero_core::workspace::settings as S;
    let _ = app.chat_state.take_input();
    app.chat_state.add_user_message("/reembed");

    let Some(store) = app.memory.clone() else {
        app.chat_state
            .add_system_message("Memory not initialized; cannot run /reembed.");
        return;
    };

    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let new_model = app
        .workspace
        .resolve_setting(S::MEMORY_EMBEDDER_MODEL, Some(&root))
        .as_str()
        .unwrap_or("nomic")
        .to_string();
    let batch_size = app
        .workspace
        .resolve_setting(S::MEMORY_EMBEDDER_REEMBED_BATCH_SIZE, Some(&root))
        .as_u64()
        .unwrap_or(32) as usize;

    let tx = app.event_tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(Event::MessageComplete {
            conv_id: String::new(),
            role: "system".to_string(),
            content: format!(
                "Re-embedding memories with `{new_model}` (batch {batch_size}). \
                 A `.bak-<ts>` of memory.db is taken first."
            ),
        });

        // Load the new embedder OUTSIDE the spawn-blocking — ONNX init
        // is CPU-heavy, so push it off the runtime.
        let new_model_for_load = new_model.clone();
        let new_embedder = match tokio::task::spawn_blocking(move || {
            gaviero_core::memory::build_embedder_by_name(&new_model_for_load)
        })
        .await
        {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("/reembed: failed to load `{new_model}`: {e}"),
                });
                return;
            }
            Err(e) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("/reembed: embedder load panicked: {e}"),
                });
                return;
            }
        };

        // Reembed currently runs against the workspace store only.
        // Per-folder reembed is a follow-up: extend reembed_all to
        // accept &MemoryStores and iterate opened_stores().
        let workspace_store = store.workspace().clone();
        match gaviero_core::memory::reembed_migration::reembed_all(
            &workspace_store,
            new_embedder,
            batch_size,
            None,
        )
        .await
        {
            Ok(report) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!(
                        "/reembed done: {} re-embedded, {} skipped, {} failed (total {}). Backup: {}",
                        report.re_embedded,
                        report.skipped,
                        report.failed,
                        report.total,
                        report
                            .backup_path
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "(in-memory store; no backup)".into()),
                    ),
                });
            }
            Err(e) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id: String::new(),
                    role: "system".to_string(),
                    content: format!("/reembed failed: {e}"),
                });
            }
        }
    });
}

pub(super) fn handle_remember_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let trimmed = input.trim();

    // Parse variant from the head word.
    let (variant, rest) = if let Some(r) = trimmed.strip_prefix("/remember-here") {
        ("here", r)
    } else if let Some(r) = trimmed.strip_prefix("/remember-module") {
        ("module", r)
    } else if let Some(r) = trimmed.strip_prefix("/remember-workspace") {
        ("workspace", r)
    } else if let Some(r) = trimmed.strip_prefix("/remember-global") {
        ("global", r)
    } else if let Some(r) = trimmed.strip_prefix("/remember") {
        ("", r)
    } else {
        return;
    };
    let text = rest.trim();

    if text.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /remember <text> (default scope)\n\
             /remember-here <text>       — Run (dies with session)\n\
             /remember-module <text>     — Module (current file's dir)\n\
             /remember-workspace <text>  — Workspace\n\
             /remember-global <text>     — Global\n\
             Default scope is configurable via memory.remember.defaultScope.",
        );
        return;
    }

    app.chat_state.add_user_message(&input);

    let Some(ref writer) = app.memory_writer else {
        app.chat_state.add_system_message(
            "Memory is not available (initialization may still be in progress).",
        );
        return;
    };

    let (scope, scope_label, fallback_note) = match resolve_remember_scope(app, variant) {
        Ok(v) => v,
        Err(e) => {
            app.chat_state
                .add_system_message(&format!("/remember: {e}"));
            return;
        }
    };
    if let Some(note) = &fallback_note {
        app.chat_state.add_system_message(note);
    }

    // A2: before writing, check for a near-duplicate at a broader scope
    // so we can print the "reinforced" confirmation instead of a plain
    // insertion badge. The check uses `search_scoped` with the target
    // scope's MemoryScope chain.
    let writer = writer.clone();
    let content = text.to_string();
    let tx = app.event_tx.clone();
    let conv_id = app.chat_state.conversations[app.chat_state.active_conv]
        .id
        .clone();
    let memory = app.memory.clone();
    let show_badge = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_REMEMBER_SHOW_SCOPE_BADGE,
            None,
        )
        .as_bool()
        .unwrap_or(true);
    let show_reinforce = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_REMEMBER_SHOW_SIMILARITY_ON_REINFORCE,
            None,
        )
        .as_bool()
        .unwrap_or(true);
    let workspace_root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    tokio::spawn(async move {
        // Pre-check: do we have a very similar memory at a broader scope?
        // Prefix the scope chain from the configured workspace down to
        // Module so cascading search covers everything wider than Run.
        let reinforce_hit: Option<(String, f32)> = match &memory {
            Some(mem) => {
                let mscope = gaviero_core::memory::MemoryScope::from_context(
                    &workspace_root,
                    Some(&workspace_root),
                    None,
                    None,
                );
                let cfg =
                    gaviero_core::memory::SearchConfig::new(&content, mscope).with_max_results(1);
                match mem.workspace().search_scoped(&cfg).await {
                    Ok(ref results) if !results.is_empty() && results[0].raw_similarity >= 0.90 => {
                        let m = &results[0];
                        let label = match m.scope_level {
                            0 => "Global",
                            1 => "Workspace",
                            2 => "Repo",
                            3 => "Module",
                            _ => "Run",
                        };
                        Some((label.to_string(), m.raw_similarity))
                    }
                    _ => None,
                }
            }
            None => None,
        };

        match writer.user_remember_scoped(scope, content.clone()).await {
            Ok(result) => {
                let badge = if show_badge {
                    format!("[{scope_label}] ")
                } else {
                    String::new()
                };
                let body = match (&result, &reinforce_hit) {
                    (_, Some((other_label, sim))) if show_reinforce => format!(
                        "✓ Reinforced existing [{other_label}] memory (similarity {sim:.2}): \
                         \"{content}\""
                    ),
                    (gaviero_core::memory::WriteResult::Deduplicated(_), _) => {
                        format!("✓ {badge}Already known: \"{content}\"")
                    }
                    (gaviero_core::memory::WriteResult::AlreadyCovered, _) => {
                        format!("✓ {badge}Covered by a broader scope; no new row: \"{content}\"")
                    }
                    _ => format!("✓ {badge}Remembered: \"{content}\""),
                };
                let _ = tx.send(Event::MessageComplete {
                    conv_id,
                    role: "system".to_string(),
                    content: body,
                });
            }
            Err(e) => {
                let msg = if e.to_string().contains("timeout") {
                    format!("⧖ Queued [{scope_label}] \"{content}\" (writer busy)")
                } else {
                    format!("Failed to store memory: {e}")
                };
                let _ = tx.send(Event::MessageComplete {
                    conv_id,
                    role: "system".to_string(),
                    content: msg,
                });
            }
        }
    });
}

pub(super) fn handle_attach_command(app: &mut App) {
    use crate::panels::agent_chat::AttachmentKind;

    let input = app.chat_state.take_input();
    let arg = input
        .trim()
        .strip_prefix("/attach")
        .unwrap_or("")
        .trim()
        .to_string();

    app.chat_state.add_user_message(&input);

    if arg.is_empty() {
        if app.chat_state.attachments.is_empty() {
            app.chat_state.add_system_message(
                "No attachments.\n\
                 Usage: /attach <path>  — attach a file\n\
                 Ctrl+V pastes clipboard images.\n\
                 /detach <name>         — remove an attachment\n\
                 /detach all            — remove all attachments",
            );
        } else {
            let list: Vec<String> = app
                .chat_state
                .attachments
                .iter()
                .map(|a| {
                    let kind = if a.kind == AttachmentKind::Image {
                        "image"
                    } else {
                        "text"
                    };
                    format!("  {} ({})", a.display_name, kind)
                })
                .collect();
            app.chat_state.add_system_message(&format!(
                "Attachments:\n{}\n\nUse /detach <name> or /detach all to remove.",
                list.join("\n")
            ));
        }
        return;
    }

    let root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let path = if std::path::Path::new(&arg).is_absolute() {
        std::path::PathBuf::from(&arg)
    } else {
        root.join(&arg)
    };

    if !path.exists() {
        app.chat_state
            .add_system_message(&format!("File not found: {}", path.display()));
        return;
    }

    if !path.is_file() {
        app.chat_state
            .add_system_message(&format!("Not a file: {}", path.display()));
        return;
    }

    let kind = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") => AttachmentKind::Image,
        _ => AttachmentKind::Text,
    };

    let display_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| arg.clone());

    app.chat_state.add_attachment(path, kind);
    app.chat_state
        .add_system_message(&format!("Attached: {}", display_name));
}

pub(super) fn handle_detach_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let arg = input
        .trim()
        .strip_prefix("/detach")
        .unwrap_or("")
        .trim()
        .to_string();

    app.chat_state.add_user_message(&input);

    if arg.is_empty() {
        app.chat_state
            .add_system_message("Usage: /detach <name> or /detach all");
        return;
    }

    if arg == "all" {
        let count = app.chat_state.attachments.len();
        app.chat_state.attachments.clear();
        app.chat_state
            .add_system_message(&format!("Removed {} attachment(s).", count));
    } else if app.chat_state.remove_attachment(&arg) {
        app.chat_state
            .add_system_message(&format!("Removed: {}", arg));
    } else {
        app.chat_state
            .add_system_message(&format!("No attachment named: {}", arg));
    }
}
