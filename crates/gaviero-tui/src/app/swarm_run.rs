//! Launching and steering DSL script runs from the SWARM panel.
//!
//! The run is `gaviero_core::swarm::pipeline::execute` on the same setup as
//! `gaviero-cli --script` ([`gaviero_core::swarm::script_run`]), so a script
//! behaves the same from either entry point. Its writes go through the
//! pipeline's Write Gate, not a chat turn's review.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::mpsc;

use super::App;
use super::state::{Focus, SidePanelMode};
use crate::event::Event;
use crate::keymap::Action;
use crate::panels::swarm_dashboard::{
    DashboardFocus, FieldRole, PlannedAgent, ScriptRow, SwarmForm, SwarmUpdate, SwarmView,
};
use crate::widgets::render_utils::strip_ansi;
use crate::widgets::text_input::TextInput;
use gaviero_core::scripts::{ScriptCatalog, is_repo_scoped};
use gaviero_core::swarm::models::{AgentStatus, SwarmResult};
use gaviero_core::swarm::plan::ExecutionMode;
use gaviero_core::swarm::{pipeline, script_run};
use gaviero_core::workspace::settings as S;

// ── Observers ───────────────────────────────────────────────────

struct TuiSwarmObserver {
    tx: mpsc::UnboundedSender<Event>,
    run: u64,
}

impl TuiSwarmObserver {
    fn send(&self, update: SwarmUpdate) {
        let _ = self.tx.send(Event::Swarm {
            run: self.run,
            update,
        });
    }
}

impl gaviero_core::observer::SwarmObserver for TuiSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        self.send(SwarmUpdate::Phase(phase.to_string()));
    }

    fn on_agent_state_changed(&self, id: &str, status: &AgentStatus, detail: &str) {
        self.send(SwarmUpdate::Agent {
            id: id.to_string(),
            status: status.clone(),
            detail: detail.to_string(),
        });
    }

    fn on_tier_started(&self, current: usize, total: usize) {
        self.send(SwarmUpdate::Tier { current, total });
    }

    fn on_merge_conflict(&self, branch: &str, files: &[String]) {
        self.send(SwarmUpdate::Notice(format!(
            "Merge conflict in {branch}: {}",
            files.join(", ")
        )));
    }

    /// `execute`'s return value is what ends the run in the panel.
    fn on_completed(&self, _result: &SwarmResult) {}

    fn on_tier_dispatch(&self, id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        self.send(SwarmUpdate::Dispatch {
            id: id.to_string(),
            tier,
            backend: backend.to_string(),
        });
    }

    fn on_loop_iteration_started(&self, current: u32, max: u32, _agents: &[String]) {
        self.send(SwarmUpdate::Notice(format!("Loop pass {current}/{max}")));
    }

    fn on_loop_gate_failed(&self, probe: &str, status: &str, output: &str) {
        let diagnostic = output.lines().next().unwrap_or_default().trim();
        self.send(SwarmUpdate::Notice(if diagnostic.is_empty() {
            format!("Loop gate failed: {probe} ({status})")
        } else {
            format!("Loop gate failed: {probe} ({status}) - {diagnostic}")
        }));
    }

    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        self.send(SwarmUpdate::Cost(estimate.estimated_usd));
    }
}

/// Per-agent stream. Permission requests keep the trait default, as they do
/// under `gaviero-cli`: what agents may do is set by the script and settings.
struct TuiSwarmAgentObserver {
    tx: mpsc::UnboundedSender<Event>,
    run: u64,
    agent: String,
}

impl TuiSwarmAgentObserver {
    fn send(&self, update: SwarmUpdate) {
        let _ = self.tx.send(Event::Swarm {
            run: self.run,
            update,
        });
    }
}

impl gaviero_core::observer::AcpObserver for TuiSwarmAgentObserver {
    fn on_stream_chunk(&self, text: &str) {
        self.send(SwarmUpdate::Text {
            id: self.agent.clone(),
            text: text.to_string(),
        });
    }

    fn on_tool_call_started(&self, tool_name: &str) {
        self.send(SwarmUpdate::Tool {
            id: self.agent.clone(),
            summary: tool_name.to_string(),
        });
    }

    fn on_streaming_status(&self, status: &str) {
        self.send(SwarmUpdate::Status {
            id: self.agent.clone(),
            status: status.to_string(),
        });
    }

    fn on_message_complete(&self, role: &str, content: &str) {
        if role == "assistant" {
            self.send(SwarmUpdate::Message {
                id: self.agent.clone(),
                content: content.to_string(),
            });
        }
    }

    fn on_proposal_deferred(&self, path: &Path, old_content: Option<&str>, new_content: &str) {
        let old_lines = old_content.map(|s| s.lines().count()).unwrap_or(0);
        let new_lines = new_content.lines().count();
        self.send(SwarmUpdate::File {
            id: self.agent.clone(),
            path: path.display().to_string(),
            additions: new_lines.saturating_sub(old_lines),
            deletions: old_lines.saturating_sub(new_lines),
        });
    }
}

// ── Opening the panel ───────────────────────────────────────────

/// `Alt+w`: show the panel. The picker is refreshed; a run stays on screen.
pub(super) fn open_panel(app: &mut App) {
    app.panel_visible.side_panel = true;
    app.side_panel = SidePanelMode::SwarmDashboard;
    app.focus = Focus::SidePanel;
    if app.swarm_dashboard.view == SwarmView::Picker {
        refresh_picker(app);
    }
}

/// Reload the script list from every scope.
pub(super) fn refresh_picker(app: &mut App) {
    let catalog = ScriptCatalog::scan(&app.workspace, &ScriptCatalog::global_scripts_dir());
    let active = super::side_panel::skill_completion_repo_id(app);

    let rows: Vec<ScriptRow> = catalog
        .complete("", active.as_deref())
        .into_iter()
        .map(|script| ScriptRow {
            name: script.name.clone(),
            source: catalog.source_label(script),
            description: script.description.clone(),
            path: script.source_path.clone(),
            owner: is_repo_scoped(script)
                .then(|| app.workspace.folder_for_path(&script.source_path))
                .flatten()
                .map(Path::to_path_buf),
        })
        .collect();

    let picker = &mut app.swarm_dashboard.picker;
    picker.rows = rows;
    picker.searched = catalog
        .scanned_dirs()
        .iter()
        .map(|d| d.path.display().to_string())
        .collect();
    picker.warnings = catalog
        .warnings()
        .iter()
        .map(|w| format!("{}: {}", w.name, w.message))
        .collect();
    let visible = picker.visible().len();
    picker.scroll.selected = picker.scroll.selected.min(visible.saturating_sub(1));
}

fn workspace_folders(app: &App) -> Vec<(String, PathBuf)> {
    let folders: Vec<(String, PathBuf)> = app
        .workspace
        .folders()
        .iter()
        .map(|f| (f.display_name().to_string(), f.path.clone()))
        .collect();
    if folders.is_empty() {
        vec![(".".to_string(), PathBuf::from("."))]
    } else {
        folders
    }
}

/// The folder a script starts in: the one that owns it, else the active
/// buffer's, else the first.
fn default_folder(app: &App, script: &ScriptRow, folders: &[(String, PathBuf)]) -> usize {
    let preferred = script.owner.clone().or_else(|| {
        app.buffers
            .get(app.active_buffer)
            .and_then(|b| b.path.as_deref())
            .and_then(|p| app.workspace.folder_for_path(p))
            .map(Path::to_path_buf)
    });
    preferred
        .and_then(|p| folders.iter().position(|(_, f)| *f == p))
        .unwrap_or(0)
}

/// Picker `Enter`: read the script's inputs and show the form.
fn open_form(app: &mut App, script: ScriptRow) {
    match gaviero_dsl::inspect_file(&script.path) {
        Ok(inspection) => {
            let folders = workspace_folders(app);
            let selected = default_folder(app, &script, &folders);
            app.swarm_dashboard.form =
                Some(SwarmForm::new(script, inspection, folders, selected));
            app.swarm_dashboard.view = SwarmView::Form;
            refresh_form_warnings(app);
        }
        Err(report) => {
            app.swarm_dashboard.picker.error =
                Some(format!("{}: {}", script.name, report_text(&report)));
        }
    }
}

/// A path typed in the filter, resolved against the active folder.
fn script_from_typed_path(app: &App, typed: &str) -> Option<ScriptRow> {
    let typed = typed.trim().strip_prefix('@').unwrap_or(typed.trim());
    if !typed.ends_with(".gaviero") {
        return None;
    }
    let folders = workspace_folders(app);
    let candidate = Path::new(typed);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        folders
            .iter()
            .map(|(_, folder)| folder.join(candidate))
            .find(|p| p.is_file())?
    };
    if !path.is_file() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_string();
    let owner = app.workspace.folder_for_path(&path).map(Path::to_path_buf);
    Some(ScriptRow {
        name,
        source: "path".to_string(),
        description: None,
        path,
        owner,
    })
}

/// What the user should know before starting: writes land in this checkout.
fn refresh_form_warnings(app: &mut App) {
    let chat_turn_running = app
        .chat_state
        .conversations
        .iter()
        .any(|c| c.is_streaming || c.has_running_background_agents());
    let Some(form) = app.swarm_dashboard.form.as_mut() else {
        return;
    };
    let mut warnings = Vec::new();
    if form.execution_mode() == ExecutionMode::Repo {
        let root = &form.folders[form.selected_folder()].1;
        if let Ok(repo) = gaviero_core::git::GitRepo::open(root)
            && let Ok(status) = repo.file_status()
        {
            let changed = status
                .iter()
                .filter(|e| !matches!(e.status, gaviero_core::git::FileStatus::Untracked))
                .count();
            if changed > 0 {
                warnings.push(format!(
                    "{changed} uncommitted change(s) here. The run writes to this checkout."
                ));
            }
        }
    }
    if chat_turn_running {
        warnings.push("A chat turn is running: its review will include this run's edits.".into());
    }
    form.warnings = warnings;
}

/// A DSL diagnostic with its source span, not just its one-line summary.
fn report_text(report: &impl std::fmt::Debug) -> String {
    strip_ansi(&format!("{report:?}"))
}

// ── Launch ──────────────────────────────────────────────────────

/// Form `Run`: compile with the entered values and start the run. Any problem
/// stays on the form; nothing has run yet.
fn launch(app: &mut App) {
    match prepare_and_start(app) {
        Ok(()) => {}
        Err(message) => {
            if let Some(form) = app.swarm_dashboard.form.as_mut() {
                form.error = Some(message);
            }
        }
    }
}

fn prepare_and_start(app: &mut App) -> Result<(), String> {
    let form = app
        .swarm_dashboard
        .form
        .as_mut()
        .ok_or_else(|| "no script selected".to_string())?;
    form.error = None;
    let values = form.collect()?;
    let script = form.script.clone();
    let execution_mode = form.execution_mode();

    let anchor = script_run::anchor_script_run(
        &values.root,
        &values.root,
        None,
        true,
        execution_mode,
        values.vars,
    )
    .map_err(|e| format!("{e:#}"))?;
    let root = std::fs::canonicalize(&anchor.root)
        .map_err(|e| format!("resolving {}: {e}", anchor.root.display()))?;
    let mut notes = anchor.notes;
    let mut vars = anchor.vars;
    notes.extend(
        script_run::materialize_external_vars(&root, &mut vars).map_err(|e| format!("{e:#}"))?,
    );
    let worktree_context_paths = script_run::worktree_context_paths(&root, &root, &vars, None);

    let plan = gaviero_dsl::compile_file(
        &script.path,
        values.workflow.as_deref(),
        values.prompt.as_deref(),
        &vars,
        &[],
        &values.params,
    )
    .map_err(|report| report_text(&report))?;

    let workspace = &app.workspace;
    let model = workspace
        .resolve_setting(S::AGENT_MODEL, Some(&root))
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| "claude:sonnet".to_string());
    let ollama_base_url = workspace
        .resolve_setting(S::AGENT_OLLAMA_BASE_URL, Some(&root))
        .as_str()
        .map(str::to_string);
    pipeline::preflight_plan_models(&plan, &model, ollama_base_url.as_deref())
        .map_err(|e| format!("{e:#}"))?;

    let mcp_config = mcp_config_for_run(app, &root, &vars);
    gaviero_core::mcp::validate_codex_trust_for_extras(&mcp_config, &plan, &model)
        .map_err(|e| format!("{e:#}"))?;

    let planned = planned_agents(&plan)?;
    let config = swarm_config(
        app,
        &plan,
        root.clone(),
        model,
        ollama_base_url,
        mcp_config,
        worktree_context_paths,
    );
    let memory = app.memory.clone();

    let label = match &values.workflow {
        Some(workflow) if workflow != &script.name => format!("{} / {workflow}", script.name),
        _ => script.name.clone(),
    };
    let run = app.swarm_dashboard.begin_run(label, root, planned, notes);

    let tx = app.event_tx.clone();
    let task = tokio::spawn({
        let tx = tx.clone();
        async move {
            let observer = TuiSwarmObserver {
                tx: tx.clone(),
                run,
            };
            let agent_tx = tx.clone();
            let make_observer =
                move |agent: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
                    Box::new(TuiSwarmAgentObserver {
                        tx: agent_tx.clone(),
                        run,
                        agent: agent.to_string(),
                    })
                };
            let update =
                match pipeline::execute(&plan, &config, None, memory, &observer, make_observer)
                    .await
                {
                    Ok(result) => SwarmUpdate::Completed(Box::new(result)),
                    Err(e) => SwarmUpdate::Failed(format!("{e:#}")),
                };
            let _ = tx.send(Event::Swarm { run, update });
        }
    });
    app.swarm_dashboard.abort = Some(task.abort_handle());
    tokio::spawn(async move {
        if let Err(e) = task.await {
            let update = if e.is_cancelled() {
                SwarmUpdate::Stopped
            } else {
                SwarmUpdate::Failed(format!("the run crashed: {e}"))
            };
            let _ = tx.send(Event::Swarm { run, update });
        }
    });
    Ok(())
}

fn planned_agents(
    plan: &gaviero_core::swarm::plan::CompiledPlan,
) -> Result<Vec<PlannedAgent>, String> {
    let ordered = plan.work_units_ordered().map_err(|e| format!("{e:#}"))?;
    Ok(ordered
        .iter()
        .chain(plan.loop_judge_units.iter())
        .map(|unit| PlannedAgent {
            id: unit.id.clone(),
            tier: unit.tier.clone(),
            model: unit.model.clone(),
        })
        .collect())
}

/// Agents reach this TUI's MCP server, which serves the first folder's
/// endpoint whichever folder the run is in.
fn mcp_config_for_run(
    app: &App,
    root: &Path,
    vars: &[(String, String)],
) -> gaviero_core::mcp::McpConfigSynth {
    use gaviero_core::mcp::{McpConfigOverrides, McpEndpoint, TrustConsent};

    let server_root = app
        .workspace
        .roots()
        .first()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| root.to_path_buf());
    let codex_trust = match app
        .workspace
        .resolve_setting(S::MCP_GAVIERO_CODEX_TRUST, Some(root))
        .as_str()
        .unwrap_or("unknown")
    {
        "granted" | "trusted" => TrustConsent::Granted,
        "denied" | "untrusted" => TrustConsent::Denied,
        _ => TrustConsent::Unknown,
    };
    let overrides = McpConfigOverrides {
        codex_trust: Some(codex_trust),
        extra_urls: script_run::mcp_urls_from_script_vars(vars),
        ..Default::default()
    };
    gaviero_core::mcp::resolve_mcp_config_synth(
        &app.workspace,
        root,
        McpEndpoint::for_workspace(&server_root),
        &overrides,
    )
}

/// The same configuration `gaviero-cli --script` builds, from this session.
fn swarm_config(
    app: &App,
    plan: &gaviero_core::swarm::plan::CompiledPlan,
    root: PathBuf,
    model: String,
    ollama_base_url: Option<String>,
    mcp_config: gaviero_core::mcp::McpConfigSynth,
    worktree_context_paths: Vec<String>,
) -> pipeline::SwarmConfig {
    let workspace = &app.workspace;
    let write_namespace = workspace.resolve_namespace(Some(&root));
    let mut read_namespaces = workspace.resolve_read_namespaces(Some(&root));
    if !read_namespaces.contains(&write_namespace) {
        read_namespaces.insert(0, write_namespace.clone());
    }
    let max_parallel = plan.max_parallel.unwrap_or(1);
    let (swarm_extra_tools, _) = workspace.resolve_agent_tools(Some(&root));
    let extract_agent_findings = app.memory_writer.is_some()
        && workspace
            .resolve_setting(S::MEMORY_EXTRACTOR_ENABLED, Some(&root))
            .as_bool()
            .unwrap_or(true);

    // A document run anchored inside a folder is not itself a folder: its
    // memory belongs to the owner, and it must not share the owner's graph,
    // since a build deletes every file it did not scan.
    let owner = workspace.folder_for_path(&root).map(Path::to_path_buf);
    let graph_db_path = owner
        .as_deref()
        .filter(|folder| std::fs::canonicalize(folder).ok().as_deref() != Some(root.as_path()))
        .map(|folder| {
            folder
                .join(".gaviero")
                .join("graphs")
                .join(gaviero_core::memory::hash_path(&root))
                .join("code_graph.db")
        });

    pipeline::SwarmConfig {
        execution_mode: plan.execution_mode,
        max_parallel,
        use_worktrees: plan.execution_mode == ExecutionMode::Repo && max_parallel > 1,
        model,
        ollama_base_url,
        read_namespaces,
        write_namespace,
        context_files: vec![],
        worktree_context_paths,
        excludes: vec![],
        memory_writer: app.memory_writer.clone(),
        mcp_config: Some(mcp_config),
        specificity: workspace.resolve_specificity_config(Some(&root)),
        swarm_extra_tools,
        tool_policy: Some(
            gaviero_core::agent_session::tool_agent::policy::ToolPolicy::from_workspace(
                workspace,
                Some(&root),
            ),
        ),
        exposed_tools: Some(gaviero_core::mcp::resolve_exposed_tools(
            workspace,
            Some(&root),
        )),
        extract_agent_findings,
        resume_from_artifacts: true,
        knowledge_invalidation: None,
        run_timeout_secs: 0,
        chat_injection: workspace.resolve_chat_injection_config(Some(&root)),
        skills_emit: gaviero_core::skills::emit::EmitSettings::from_workspace(
            workspace,
            Some(&root),
        ),
        skill_catalog: Some(Arc::clone(&app.skill_catalog)),
        memory_root: owner,
        graph_db_path,
        workspace_root: root,
    }
}

// ── Input ───────────────────────────────────────────────────────

pub(super) fn handle_action(app: &mut App, action: Action) {
    match app.swarm_dashboard.view {
        SwarmView::Picker => handle_picker(app, action),
        SwarmView::Form => handle_form(app, action),
        SwarmView::Run => handle_run(app, action),
    }
}

/// `Esc` steps back one level. Returns whether it did anything.
pub(super) fn handle_escape(app: &mut App) -> bool {
    let dash = &mut app.swarm_dashboard;
    match dash.view {
        SwarmView::Form => {
            dash.form = None;
            dash.view = SwarmView::Picker;
            refresh_picker(app);
            true
        }
        SwarmView::Picker if dash.has_run() => {
            dash.view = SwarmView::Run;
            true
        }
        SwarmView::Picker => false,
        SwarmView::Run => {
            if dash.diff_agent.is_some() {
                dash.diff_agent = None;
                true
            } else if dash.pending_stop_confirm || dash.pending_undo_confirm {
                dash.pending_stop_confirm = false;
                dash.pending_undo_confirm = false;
                true
            } else if dash.output_maximized {
                dash.output_maximized = false;
                true
            } else {
                false
            }
        }
    }
}

/// Text typed or pasted while the picker or a form field has the keyboard.
pub(super) fn handle_paste(app: &mut App, text: &str) {
    let dash = &mut app.swarm_dashboard;
    match dash.view {
        SwarmView::Picker => {
            dash.picker.filter.insert_str(text.trim());
            dash.picker.filter_changed();
        }
        SwarmView::Form => {
            if let Some(form) = dash.form.as_mut() {
                let multiline = form.focused_role() == Some(FieldRole::Prompt);
                if let Some(input) = form.focused_input() {
                    if multiline {
                        input.insert_str(text);
                    } else {
                        input.insert_str(&text.replace(['\r', '\n'], " "));
                    }
                }
            }
        }
        SwarmView::Run => {}
    }
}

/// Apply a text-editing action. Returns false if it was not one.
fn edit_input(input: &mut TextInput, action: &Action) -> bool {
    match action {
        Action::InsertChar(ch) => input.insert_char(*ch),
        Action::Backspace => input.backspace(),
        Action::Delete => input.delete(),
        Action::DeleteWordBack => input.delete_word_back(),
        Action::CursorLeft => input.move_left(),
        Action::CursorRight => input.move_right(),
        Action::WordLeft => input.move_word_left(),
        Action::WordRight => input.move_word_right(),
        Action::Home => input.move_home(),
        Action::End => input.move_end(),
        Action::Undo => input.undo(),
        Action::Redo => input.redo(),
        _ => return false,
    }
    true
}

fn handle_picker(app: &mut App, action: Action) {
    let visible = app.swarm_dashboard.picker.visible().len();
    match action {
        Action::CursorUp => app.swarm_dashboard.picker.scroll.move_up(),
        Action::CursorDown => app.swarm_dashboard.picker.scroll.move_down(visible),
        Action::Enter => {
            let typed = app.swarm_dashboard.picker.filter.text.clone();
            let chosen = app
                .swarm_dashboard
                .picker
                .selected()
                .or_else(|| script_from_typed_path(app, &typed));
            match chosen {
                Some(script) => open_form(app, script),
                None if typed.trim().ends_with(".gaviero") => {
                    app.swarm_dashboard.picker.error = Some(format!("No such file: {typed}"));
                }
                None => {}
            }
        }
        other => {
            let picker = &mut app.swarm_dashboard.picker;
            let before = picker.filter.text.clone();
            edit_input(&mut picker.filter, &other);
            if picker.filter.text != before {
                picker.filter_changed();
            }
        }
    }
}

fn handle_form(app: &mut App, action: Action) {
    let Some(form) = app.swarm_dashboard.form.as_mut() else {
        return;
    };
    match action {
        Action::Tab | Action::CursorDown => form.focus_next(),
        Action::CursorUp => form.focus_previous(),
        Action::Enter if form.on_run_button() => launch(app),
        Action::Enter => form.focus_next(),
        Action::AltEnter if form.focused_role() == Some(FieldRole::Prompt) => {
            if let Some(input) = form.focused_input() {
                input.insert_char('\n');
            }
        }
        Action::CursorLeft | Action::CursorRight | Action::InsertChar(' ')
            if form.focused_input().is_none() =>
        {
            let delta = if action == Action::CursorLeft { -1 } else { 1 };
            form.cycle_focused_choice(delta);
            refresh_form_warnings(app);
        }
        other => {
            form.error = None;
            if let Some(input) = form.focused_input() {
                edit_input(input, &other);
            }
        }
    }
}

fn handle_run(app: &mut App, action: Action) {
    let dash = &mut app.swarm_dashboard;

    match action {
        Action::InsertChar('x') if dash.running => {
            if dash.pending_stop_confirm {
                dash.pending_stop_confirm = false;
                if let Some(abort) = &dash.abort {
                    abort.abort();
                    dash.phase = "stopping".to_string();
                }
            } else {
                dash.pending_stop_confirm = true;
            }
            return;
        }
        Action::InsertChar('n') if !dash.running => {
            dash.view = SwarmView::Picker;
            refresh_picker(app);
            return;
        }
        Action::InsertChar('d') if !dash.running => {
            show_selected_diff(app);
            return;
        }
        Action::InsertChar('u') if !dash.running => {
            undo_pressed(app);
            return;
        }
        Action::Enter => {
            dash.output_maximized = !dash.output_maximized;
            return;
        }
        Action::Tab => {
            dash.focus = match dash.focus {
                DashboardFocus::Table => DashboardFocus::Detail,
                DashboardFocus::Detail => DashboardFocus::Table,
            };
            return;
        }
        _ => {}
    }

    if let Some(diff) = dash.diff_agent.as_mut() {
        match action {
            Action::CursorUp | Action::InsertChar('k') => diff.scroll = diff.scroll.saturating_sub(1),
            Action::CursorDown | Action::InsertChar('j') => {
                diff.scroll = diff.scroll.saturating_add(1)
            }
            Action::PageUp => diff.scroll = diff.scroll.saturating_sub(10),
            Action::PageDown => diff.scroll = diff.scroll.saturating_add(10),
            _ => {}
        }
        return;
    }

    let target_output = dash.focus == DashboardFocus::Detail || dash.output_maximized;
    match action {
        Action::CursorUp | Action::InsertChar('k') if !target_output => {
            let index = dash.scroll.selected.saturating_sub(1);
            dash.select_by_user(index);
        }
        Action::CursorDown | Action::InsertChar('j') if !target_output => {
            let index = dash.scroll.selected + 1;
            dash.select_by_user(index);
        }
        Action::PageUp if !target_output => {
            let index = dash.scroll.selected.saturating_sub(10);
            dash.select_by_user(index);
        }
        Action::PageDown if !target_output => {
            let index = dash.scroll.selected + 10;
            dash.select_by_user(index);
        }
        Action::Home if !target_output => dash.select_by_user(0),
        Action::End if !target_output => dash.select_by_user(usize::MAX),
        Action::CursorUp | Action::InsertChar('k') => scroll_output(dash, -1),
        Action::CursorDown | Action::InsertChar('j') => scroll_output(dash, 1),
        Action::PageUp => scroll_output(dash, -10),
        Action::PageDown => scroll_output(dash, 10),
        Action::Home => {
            dash.detail_auto_scroll = false;
            dash.detail_scroll = 0;
        }
        Action::End | Action::InsertChar('f') => dash.detail_auto_scroll = true,
        _ => {}
    }
}

/// Move through the output. Reaching the end resumes following it.
pub(super) fn scroll_output(
    dash: &mut crate::panels::swarm_dashboard::SwarmDashboardState,
    delta: isize,
) {
    let total = dash.output_row_count();
    let viewport = dash.detail_rect.height as usize;
    let bottom = total.saturating_sub(viewport);
    let current = if dash.detail_auto_scroll {
        bottom
    } else {
        dash.detail_scroll.min(bottom)
    };
    let next = current.saturating_add_signed(delta).min(bottom);
    dash.detail_scroll = next;
    dash.detail_auto_scroll = next >= bottom;
}

fn show_selected_diff(app: &mut App) {
    let dash = &app.swarm_dashboard;
    let Some(agent) = dash.agents.get(dash.scroll.selected) else {
        return;
    };
    let agent_id = agent.id.clone();
    let Some(branch) = agent.branch.clone() else {
        app.swarm_dashboard.status_message =
            format!("No diff for {agent_id}: it ran without worktree isolation");
        return;
    };
    let pre_sha = dash
        .result
        .as_ref()
        .map(|r| r.pre_swarm_sha.clone())
        .unwrap_or_default();
    let Some(root) = dash.run_root.clone() else {
        return;
    };
    let diff = gaviero_core::git::diff_branch_vs_sha(&root, &pre_sha, &branch).unwrap_or_default();
    app.swarm_dashboard.show_diff(agent_id, diff);
}

fn undo_pressed(app: &mut App) {
    let dash = &mut app.swarm_dashboard;
    let undoable = dash
        .result
        .as_ref()
        .is_some_and(|r| !r.pre_swarm_sha.is_empty());
    if !undoable {
        dash.status_message = "Nothing to undo: this run made no merges.".to_string();
        return;
    }
    if !dash.pending_undo_confirm {
        dash.pending_undo_confirm = true;
        return;
    }
    dash.pending_undo_confirm = false;
    let (Some(result), Some(root)) = (dash.result.clone(), dash.run_root.clone()) else {
        return;
    };
    let run = dash.run_id;
    let tx = app.event_tx.clone();
    tokio::task::spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || pipeline::revert_swarm(&root, &result))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
        let _ = tx.send(Event::Swarm {
            run,
            update: SwarmUpdate::Reverted(outcome),
        });
    });
}

/// Wheel over the panel: the output when the pointer is on it, else the list.
pub(super) fn scroll_wheel(app: &mut App, up: bool, col: u16, row: u16) {
    let delta = crate::theme::MOUSE_SCROLL_DELTA;
    let dash = &mut app.swarm_dashboard;
    match dash.view {
        SwarmView::Picker => {
            let visible = dash.picker.visible().len();
            if up {
                dash.picker.scroll.scroll_up(delta);
            } else {
                dash.picker.scroll.scroll_down(delta, visible);
            }
        }
        SwarmView::Form => {}
        SwarmView::Run => {
            if dash.detail_rect.contains((col, row).into()) {
                scroll_output(dash, if up { -(delta as isize) } else { delta as isize });
            } else if up {
                dash.scroll.scroll_up(1);
            } else {
                let count = dash.agents.len();
                dash.scroll.scroll_down(1, count);
            }
        }
    }
}

/// Left click: pick a script or agent, or give the output pane the keyboard.
pub(super) fn click(app: &mut App, col: u16, row: u16) {
    let dash = &mut app.swarm_dashboard;
    match dash.view {
        SwarmView::Picker => {
            if let Some(index) = dash.picker.index_at(col, row) {
                dash.picker.scroll.selected = index;
            }
        }
        SwarmView::Form => {}
        SwarmView::Run => {
            let position = (col, row).into();
            if dash.table_rect.contains(position) {
                dash.focus = DashboardFocus::Table;
                let index = dash.scroll.offset + (row - dash.table_rect.y) as usize;
                if index < dash.agents.len() {
                    dash.select_by_user(index);
                }
            } else if dash.detail_rect.contains(position) {
                dash.focus = DashboardFocus::Detail;
            }
        }
    }
}

/// Running agents, for the quit dialog.
pub(super) fn running_agent_ids(app: &App) -> Vec<String> {
    if !app.swarm_dashboard.running {
        return Vec::new();
    }
    app.swarm_dashboard
        .agents
        .iter()
        .filter(|a| matches!(a.status, AgentStatus::Running))
        .map(|a| a.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::swarm_dashboard::SwarmDashboardState;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    const SCRIPT: &str = r#"// description: Write one report
client local {
    model "ollama:qwen"
    privacy public
}

agent writer {
    description "write the report"
    client local
    scope { owned ["{{OUT_DIR}}/report.md"] }
    produces ["{{OUT_DIR}}/report.md"]
    prompt "Write the report about {{PROMPT}}."
    max_retries 0
}

vars { OUT_DIR "default_out" }

workflow run {
    execution_mode document
    steps [writer]
    max_parallel 1
}
"#;

    /// Reads one HTTP request and returns its body.
    fn read_request(stream: &std::net::TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        let _ = reader.read_exact(&mut body);
        String::from_utf8_lossy(&body).to_string()
    }

    /// An Ollama that writes `path` on every chat and records what it was asked.
    fn fake_ollama(path: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = seen_tx.send(read_request(&stream));
                let content = format!("<file path=\"{path}\">\nreport body\n</file>");
                let chunk = serde_json::json!({
                    "model": "qwen",
                    "message": {"role": "assistant", "content": content},
                    "done": false
                });
                let done = serde_json::json!({"model": "qwen", "done": true});
                let body = format!("{chunk}\n{done}\n");
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{addr}"), seen_rx)
    }

    /// An Ollama that accepts the request and never answers.
    fn silent_ollama() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                held.push(stream);
            }
        });
        format!("http://{addr}")
    }

    fn app_with_script(
        dir: &Path,
        base_url: &str,
    ) -> (App, mpsc::UnboundedReceiver<Event>) {
        let script_dir = dir.join(".gaviero/dsl/report");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(script_dir.join("DSL.gaviero"), SCRIPT).unwrap();
        std::fs::write(
            dir.join(".gaviero/settings.json"),
            serde_json::json!({
                "agent": { "ollamaBaseUrl": base_url },
                "memory": { "extractor": { "enabled": false } },
            })
            .to_string(),
        )
        .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let app = App::new(
            gaviero_core::workspace::Workspace::single_folder(dir.to_path_buf()),
            tx,
        );
        (app, rx)
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            handle_action(app, Action::InsertChar(ch));
        }
    }

    /// Feed run events to the panel until `done` holds.
    async fn pump_until(
        app: &mut App,
        rx: &mut mpsc::UnboundedReceiver<Event>,
        done: impl Fn(&App) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !done(app) {
                match rx.recv().await {
                    Some(Event::Swarm { run, update }) => {
                        app.swarm_dashboard.apply(run, update);
                    }
                    Some(_) => {}
                    None => panic!("event channel closed"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out; phase={} status={}",
                app.swarm_dashboard.phase, app.swarm_dashboard.status_message
            )
        });
    }

    /// Open the panel, pick the only script, and fill in its prompt.
    fn open_filled_form(app: &mut App) {
        open_panel(app);
        assert_eq!(app.swarm_dashboard.picker.rows.len(), 1);
        assert_eq!(app.swarm_dashboard.picker.rows[0].name, "report");
        assert_eq!(
            app.swarm_dashboard.picker.rows[0].description.as_deref(),
            Some("Write one report")
        );
        handle_action(app, Action::Enter);
        assert_eq!(app.swarm_dashboard.view, SwarmView::Form);
        type_text(app, "the moon");
    }

    fn press_run(app: &mut App) {
        while !app.swarm_dashboard.form.as_ref().unwrap().on_run_button() {
            handle_action(app, Action::Enter);
        }
        handle_action(app, Action::Enter);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_runs_from_the_panel_with_the_entered_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (url, asked) = fake_ollama("custom_out/report.md");
        let (mut app, mut rx) = app_with_script(&root, &url);

        open_filled_form(&mut app);
        handle_action(&mut app, Action::Tab);
        // The var field is pre-filled with the script's default.
        for _ in 0.."default_out".len() {
            handle_action(&mut app, Action::Backspace);
        }
        type_text(&mut app, "custom_out");
        press_run(&mut app);

        assert_eq!(
            app.swarm_dashboard.view,
            SwarmView::Run,
            "error: {:?}",
            app.swarm_dashboard.form.as_ref().and_then(|f| f.error.clone())
        );
        assert_eq!(app.swarm_dashboard.agents[0].id, "writer");

        pump_until(&mut app, &mut rx, |a| !a.swarm_dashboard.running).await;

        let dash = &app.swarm_dashboard;
        assert_eq!(dash.phase, "completed", "{}", dash.status_message);
        assert!(matches!(dash.agents[0].status, AgentStatus::Completed));
        assert!(
            !dash.agents[0].activity.is_empty(),
            "the agent's output reached the panel"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("custom_out/report.md"))
                .unwrap()
                .trim_end(),
            "report body",
            "the var override reached the agent's scope and the file was written"
        );
        let request = asked.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.contains("the moon"), "prompt reached the agent: {request}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_ends_a_run_that_would_otherwise_never_finish() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, mut rx) = app_with_script(&root, &silent_ollama());

        open_filled_form(&mut app);
        press_run(&mut app);
        assert_eq!(app.swarm_dashboard.view, SwarmView::Run);
        pump_until(&mut app, &mut rx, |a| {
            matches!(a.swarm_dashboard.agents[0].status, AgentStatus::Running)
        })
        .await;

        handle_action(&mut app, Action::InsertChar('x'));
        assert!(app.swarm_dashboard.running, "one press only asks to confirm");
        assert!(app.swarm_dashboard.pending_stop_confirm);
        handle_action(&mut app, Action::InsertChar('x'));

        pump_until(&mut app, &mut rx, |a| !a.swarm_dashboard.running).await;
        assert_eq!(app.swarm_dashboard.phase, "stopped");
        assert!(matches!(app.swarm_dashboard.agents[0].status, AgentStatus::Failed(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_compile_error_stays_on_the_form_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, _rx) = app_with_script(&root, "http://127.0.0.1:9");

        open_filled_form(&mut app);
        std::fs::write(
            root.join(".gaviero/dsl/report/DSL.gaviero"),
            "agent writer { client missing prompt \"x\" }\n",
        )
        .unwrap();
        press_run(&mut app);

        let dash = &app.swarm_dashboard;
        assert_eq!(dash.view, SwarmView::Form);
        assert!(!dash.running);
        assert!(!dash.has_run());
        let error = dash.form.as_ref().unwrap().error.clone().unwrap();
        assert!(error.contains("missing"), "diagnostic keeps its detail: {error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_prompt_is_refused_before_compiling() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, _rx) = app_with_script(&root, "http://127.0.0.1:9");

        open_panel(&mut app);
        handle_action(&mut app, Action::Enter);
        press_run(&mut app);

        let form = app.swarm_dashboard.form.as_ref().unwrap();
        assert_eq!(form.error.as_deref(), Some("Prompt is required"));
        assert_eq!(form.fields[form.focus].name, "Prompt");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_path_typed_in_the_filter_opens_that_script() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, _rx) = app_with_script(&root, "http://127.0.0.1:9");
        std::fs::write(root.join("other.gaviero"), SCRIPT).unwrap();

        open_panel(&mut app);
        type_text(&mut app, "other.gaviero");
        handle_action(&mut app, Action::Enter);

        assert_eq!(app.swarm_dashboard.view, SwarmView::Form);
        assert_eq!(app.swarm_dashboard.form.as_ref().unwrap().script.source, "path");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn escape_walks_back_from_form_to_picker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, _rx) = app_with_script(&root, "http://127.0.0.1:9");

        open_panel(&mut app);
        handle_action(&mut app, Action::Enter);
        assert!(handle_escape(&mut app));
        assert_eq!(app.swarm_dashboard.view, SwarmView::Picker);
        assert!(!handle_escape(&mut app), "nothing further to step back to");
    }

    #[test]
    fn scrolling_up_stops_following_and_the_end_resumes_it() {
        let mut dash = SwarmDashboardState::new();
        let run = dash.begin_run(
            "s".into(),
            PathBuf::from("/a"),
            vec![PlannedAgent {
                id: "a".into(),
                tier: gaviero_core::types::ModelTier::Cheap,
                model: None,
            }],
            vec![],
        );
        dash.apply(
            run,
            SwarmUpdate::Text {
                id: "a".into(),
                text: (0..50).map(|i| format!("line {i}\n")).collect(),
            },
        );
        dash.detail_rect = ratatui::layout::Rect::new(0, 0, 40, 10);

        scroll_output(&mut dash, -1);
        assert!(!dash.detail_auto_scroll);
        assert_eq!(dash.detail_scroll, 39);

        scroll_output(&mut dash, 1);
        assert!(dash.detail_auto_scroll, "back at the end, following resumes");
    }
}
