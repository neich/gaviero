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
    let excludes = parse_exclude_patterns(&app.workspace);

    tokio::spawn(async move {
        use gaviero_core::swarm::{pipeline, planner};

        let memory_ctx = if let Some(ref mem) = memory {
            mem.search_context(&read_ns, &task_desc, 5).await
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
        for rel_path in &refs {
            let abs_path = root.join(rel_path);
            if let Ok(content) = std::fs::read_to_string(&abs_path) {
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

pub(super) fn handle_remember_command(app: &mut App) {
    let input = app.chat_state.take_input();
    let text = input.trim().strip_prefix("/remember").unwrap_or("").trim();

    if text.is_empty() {
        app.chat_state.add_system_message(
            "Usage: /remember <text to remember>\n\
             Stores text to semantic memory for future retrieval.",
        );
        return;
    }

    app.chat_state.add_user_message(&input);

    let Some(ref memory) = app.memory else {
        app.chat_state.add_system_message(
            "Memory is not available (initialization may still be in progress).",
        );
        return;
    };

    let mem = memory.clone();
    let ns = app.chat_state.agent_settings.write_namespace.clone();
    let content = text.to_string();
    let key = format!(
        "user:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let tx = app.event_tx.clone();
    let conv_id = app.chat_state.conversations[app.chat_state.active_conv]
        .id
        .clone();

    tokio::spawn(async move {
        match mem.store(&ns, &key, &content, None).await {
            Ok(_) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id,
                    role: "system".to_string(),
                    content: format!("Remembered: \"{}\"", content),
                });
            }
            Err(e) => {
                let _ = tx.send(Event::MessageComplete {
                    conv_id,
                    role: "system".to_string(),
                    content: format!("Failed to store memory: {}", e),
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
