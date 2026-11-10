mod app;
mod editor;
mod event;
mod keymap;
mod notify;
mod panels;
mod platform;
mod session_host;
mod setup;
mod supervisor;
mod theme;
mod thinking;
mod widgets;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{
        DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use app::App;
use event::EventLoop;

const CHAT_STREAM_RENDER_INTERVAL: Duration = Duration::from_millis(100);
const CHAT_SPINNER_RENDER_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Parser)]
#[command(
    name = "gaviero",
    about = "Terminal code editor for AI agent orchestration"
)]
struct Cli {
    /// Path to open (directory or .gaviero-workspace file)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Open PATH as a multi-folder workspace. Reuses the
    /// `*.gaviero-workspace` file already in PATH, or asks which sub-folders
    /// to include and writes `<dirname>.gaviero-workspace`.
    #[arg(long)]
    workspace: bool,

    /// Open PATH as a plain folder even when it holds a
    /// `*.gaviero-workspace` file, without asking.
    #[arg(long, conflicts_with = "workspace")]
    folder: bool,

    /// Run a session host: several gaviero sessions in this terminal,
    /// switched with Alt+Left/Right (Alt+Shift+Left/Right moves a tab,
    /// Alt+N opens one, Alt+Q closes one, Ctrl+Q quits all and reopens
    /// them next time).
    /// With PATH, opens only PATH; without, reopens the last set.
    #[arg(long, conflicts_with_all = ["workspace", "folder"])]
    sessions: bool,
}

/// C1: synchronous yes/no prompt on stdin/stderr asking the user to
/// consent to the typed-stores migration. Runs before raw mode is
/// enabled so plain stdin works. Returns `Ok(true)` on consent.
///
/// Inputs are interpreted permissively:
/// - `y`, `yes`, blank line on a TTY default → consent
/// - `n`, `no`, anything else → decline
/// - EOF (e.g., `</dev/null`) → decline (treat headless TUI invocation
///   as "no consent" rather than silently migrating).
fn prompt_c1_consent(proposals: &[gaviero_core::memory::C1MigrationProposal]) -> Result<bool> {
    use std::io::{BufRead, Write};

    let mut stderr = std::io::stderr().lock();
    writeln!(
        stderr,
        "\nGaviero's memory schema is upgrading to typed stores (C1)."
    )?;
    writeln!(
        stderr,
        "This is a one-time, load-bearing migration. A backup of each affected"
    )?;
    writeln!(
        stderr,
        "memory.db will be taken automatically before the migration runs."
    )?;
    writeln!(stderr)?;
    writeln!(stderr, "Affected databases:")?;
    for p in proposals {
        writeln!(
            stderr,
            "  - {}  (v{} → v{})",
            p.db_path.display(),
            p.current_version,
            p.target_version
        )?;
        writeln!(stderr, "    backup → {}", p.proposed_backup_path.display())?;
    }
    writeln!(stderr)?;
    write!(stderr, "Continue? [y/N] ")?;
    stderr.flush()?;
    drop(stderr);

    let stdin = std::io::stdin();
    let mut line = String::new();
    let n = stdin.lock().read_line(&mut line)?;
    if n == 0 {
        // EOF — treat as decline.
        return Ok(false);
    }
    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

/// Restore the host terminal to a sane state. Called on every exit path:
/// normal exit (where the error is reported), `?` early returns, and panics
/// (where it is ignored). Every step is attempted even if an earlier one
/// fails; the first error is returned.
fn restore_terminal() -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    let raw = disable_raw_mode();
    let vt_mouse = platform::disable_vt_mouse_passthrough(&mut stdout);
    let modes = execute!(
        stdout,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange,
        crossterm::cursor::Show
    );
    let paste = platform::set_bracketed_paste(&mut stdout, false);
    raw.and(vt_mouse).and(modes).and(paste)
}

/// Coalesces expensive Agent Chat streaming redraws while leaving state
/// updates immediate. Keyboard, mouse (except pointer-move), resize, terminal,
/// review, and lifecycle events still repaint immediately; only visible active
/// chat stream output and spinner-only ticks are budgeted. `MouseEventKind::Moved`
/// is skipped here and requests a paint only when preview link hover changes —
/// a Unix-only path since Windows stopped requesting any-motion tracking.
struct RenderScheduler {
    last_chat_render: Instant,
    pending_chat_render: bool,
}

impl RenderScheduler {
    fn new() -> Self {
        Self {
            last_chat_render: Instant::now()
                .checked_sub(CHAT_STREAM_RENDER_INTERVAL)
                .unwrap_or_else(Instant::now),
            pending_chat_render: false,
        }
    }

    fn should_render(&mut self, event: &event::Event, app: &App) -> bool {
        match event {
            event::Event::StreamChunk { conv_id, .. }
            | event::Event::ToolCallStarted { conv_id, .. }
            | event::Event::StreamingStatus { conv_id, .. }
            | event::Event::BackgroundTaskStarted { conv_id, .. }
            | event::Event::BackgroundTaskFinished { conv_id, .. } => {
                self.should_render_chat_stream_event(conv_id, app)
            }
            event::Event::Tick => self.should_render_tick(app),
            event::Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved) => false,
            _ => true,
        }
    }

    fn should_render_chat_stream_event(&mut self, conv_id: &str, app: &App) -> bool {
        if !app.agent_chat_visible() || !app.is_active_chat_conversation(conv_id) {
            return false;
        }

        if self.last_chat_render.elapsed() >= CHAT_STREAM_RENDER_INTERVAL {
            true
        } else {
            self.pending_chat_render = true;
            false
        }
    }

    fn should_render_tick(&mut self, app: &App) -> bool {
        if !app.active_agent_chat_stream_visible() {
            self.pending_chat_render = false;
            return false;
        }

        if self.pending_chat_render
            && self.last_chat_render.elapsed() >= CHAT_STREAM_RENDER_INTERVAL
        {
            return true;
        }

        self.last_chat_render.elapsed() >= CHAT_SPINNER_RENDER_INTERVAL
    }

    fn record_render(&mut self, app: &App) {
        if app.agent_chat_visible() {
            self.last_chat_render = Instant::now();
            self.pending_chat_render = false;
        }
    }
}

/// RAII guard that restores the terminal when dropped — covers `?` returns
/// and normal scope exit.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn main() -> Result<()> {
    // Session host (`gaviero --sessions`): a separate program sharing this
    // binary. Checked first — before the probe below, so a host started with
    // `GAVIERO_INPUT_PROBE` stays a host and only its children become
    // probes, and before the supervisor, which the host replaces.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let Some(launch) = session_host::sessions_requested(&args) {
        let launch = match launch {
            Ok(launch) => launch,
            Err(msg) => {
                eprintln!("gaviero: {msg}");
                std::process::exit(2);
            }
        };
        // Nested host refusal: every session child carries
        // `GAVIERO_SESSION_HOST` (`session_host::HOST_ENV`).
        if std::env::var_os(session_host::HOST_ENV).is_some() {
            eprintln!("gaviero: --sessions cannot run inside a session host");
            std::process::exit(2);
        }
        // Restart-on-new-build for the host itself: the process a user
        // starts supervises the real host and relaunches it, with the same
        // tabs, after a deploy.
        if let Some(code) = supervisor::supervise_host() {
            std::process::exit(code);
        }
        std::process::exit(session_host::run(launch));
    }

    // Test-only plumbing for `tests/session_host_*.rs`: with
    // `GAVIERO_INPUT_PROBE` set, the binary is an input probe that prints
    // every event it receives (`session_host::probe`) instead of the TUI.
    if std::env::var_os("GAVIERO_INPUT_PROBE").is_some() {
        return session_host::probe::run_probe();
    }

    // Restart-on-new-build: the process a user starts supervises the real
    // TUI and relaunches it in place after a deploy (`supervisor`). It runs
    // before the runtime, logging, or argument parsing — the child does all
    // of that.
    if let Some(code) = supervisor::supervise() {
        std::process::exit(code);
    }

    let restart = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?
        .block_on(run())?;
    if restart {
        std::process::exit(supervisor::RESTART_EXIT_CODE);
    }
    Ok(())
}

/// A plain directory launch (`gaviero <dir>`, no `--workspace` / `--folder`)
/// whose directory holds a `.gaviero-workspace` file: ask whether to open
/// that workspace instead. Opening the folder splits memory, code graph and
/// settings from the workspace's, so it should be a choice, not an accident.
/// Returns the workspace file to open, or `None` for the plain folder.
///
/// A restart under the supervisor reuses the earlier answer. Without a TTY
/// the folder opens as asked, with a warning in the log.
fn folder_launch_workspace(path: &Path) -> Result<Option<PathBuf>> {
    use setup::FolderLaunch;
    let Some(file) = setup::existing_workspace_file(path) else {
        return Ok(None);
    };
    let choice = match supervisor::recall_launch_choice(path) {
        Some(choice) => choice,
        None if std::io::stdin().is_terminal() => {
            let choice = setup::ask_folder_launch(path, &file)?;
            supervisor::remember_launch_choice(path, &choice);
            choice
        }
        None => {
            tracing::warn!(
                "{} holds {}; opening it as a plain folder (no TTY to ask). \
                 Memory, code graph and settings are separate from the workspace's.",
                path.display(),
                file.display()
            );
            FolderLaunch::Folder
        }
    };
    Ok(match choice {
        FolderLaunch::Workspace(file) => Some(file),
        FolderLaunch::Folder => None,
    })
}

/// The TUI itself. Returns true when it quit to restart into a newly
/// installed build.
async fn run() -> Result<bool> {
    // Write tracing output to a log file (stderr is hidden in TUI mode)
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gaviero");
    std::fs::create_dir_all(&log_dir).context("creating log directory")?;
    let log_file =
        std::fs::File::create(log_dir.join("gaviero.log")).context("creating log file")?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        // Default stays DEBUG; GAVIERO_LOG (EnvFilter syntax) overrides it.
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GAVIERO_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
        )
        .init();

    let cli = Cli::parse();
    // `--sessions` is documented here for `--help` only: `main` hands it to
    // the session host before this point.
    if cli.sessions {
        anyhow::bail!("--sessions is handled by the session host");
    }

    // Simplify away Windows' `\\?\` verbatim prefix — this path becomes
    // the workspace root and flows into shell cwds, prompts, and configs.
    let path = gaviero_core::util::fs::canonicalize_simplified(&cli.path)
        .with_context(|| format!("resolving path: {}", cli.path.display()))?;

    // Launch dispatch. An explicit `*.gaviero-workspace` argument always wins.
    // `--workspace` on a directory reuses the workspace file already inside it.
    // The setup screen runs before the editor while the target is
    // unconfigured — a folder without `.gaviero/settings.json`, a workspace
    // file without one beside it, or `--workspace` with no workspace file
    // yet: it writes the settings chosen there, or opens an editor-only session.
    let is_workspace_file = path
        .extension()
        .is_some_and(|ext| ext == "gaviero-workspace");

    let mut workspace_file: Option<PathBuf> = if is_workspace_file {
        Some(path.clone())
    } else if cli.workspace {
        setup::existing_workspace_file(&path)
    } else if cli.folder {
        None
    } else {
        folder_launch_workspace(&path)?
    };
    let launch_mode = setup::LaunchMode::for_launch(workspace_file.as_deref(), cli.workspace);
    // Editor-only: explorer, find, editor and terminal; no agents, memory,
    // code graph, MCP or remote, and no settings written.
    let mut editor_only = false;

    // Skip the setup screen without a TTY (piped stdin, CI): it needs raw
    // mode, and built-in defaults still give a working session.
    if setup::needs_setup(&path, &launch_mode) && std::io::stdin().is_terminal() {
        let outcome = setup::run(&path, launch_mode)?;
        workspace_file = outcome.workspace_file;
        editor_only = outcome.editor_only;
    }

    let workspace = match &workspace_file {
        Some(file) => gaviero_core::workspace::Workspace::load(file)?,
        None => gaviero_core::workspace::Workspace::single_folder(path),
    };

    // Cursor has no flag to switch its instruction files off; its only bypass
    // is `.cursorignore`. Sync it before the first agent can spawn, whichever
    // setup path this launch took.
    if !editor_only {
        let roots: Vec<std::path::PathBuf> = workspace
            .roots()
            .iter()
            .map(|r| r.to_path_buf())
            .collect();
        for changed in gaviero_core::instructions::sync_cursorignore_roots(&roots) {
            tracing::info!(".cursorignore synced for {}", changed.display());
        }
    }

    // C1: prompt the user for consent on a pending typed-stores
    // migration BEFORE entering raw mode. If the user declines, exit
    // cleanly without taking the snapshot or migrating. The plan's
    // anti-pattern explicitly forbids silent migration here — a yes/no
    // stdin prompt is the minimum non-silent surface.
    if !editor_only {
        let workspace_root = workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let pending = gaviero_core::memory::MemoryStores::probe_pending_c1_migrations(
            &workspace_root,
            &workspace,
        )
        .context("probing for pending C1 typed-stores migration")?;
        if !pending.is_empty() && !prompt_c1_consent(&pending)? {
            eprintln!(
                "Aborted. Memory schema upgrade declined; \
                 Gaviero will not start until the upgrade is accepted \
                 or the affected memory.db files are removed."
            );
            return Ok(false);
        }
    }

    // Install panic hook BEFORE entering raw mode so that any panic
    // (including inside ratatui, crossterm, or our own code) restores
    // the terminal before printing the panic message.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));

    // Terminal setup
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange
    )
    .context("entering alternate screen")?;
    // Bracketed paste and VT mouse passthrough are platform-dependent —
    // see `platform` for why each is gated the way it is.
    platform::set_bracketed_paste(&mut stdout, true).context("enabling bracketed paste")?;
    platform::enable_vt_mouse_passthrough(&mut stdout).context("enabling VT mouse passthrough")?;

    // RAII guard: if anything below returns Err via `?`, the terminal
    // is still restored when `_guard` is dropped.
    let _guard = TerminalGuard;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).context("creating terminal")?;

    // Event loop setup
    let mut event_loop = EventLoop::new();
    let event_tx = event_loop.tx();
    let mut event_rx = event_loop.take_rx();

    // Windows: console CTRL_C/CTRL_BREAK events must never terminate the
    // TUI — swallow them and re-route into the normal key path (see
    // `platform::install_console_ctrl_forwarder`). No-op elsewhere.
    platform::install_console_ctrl_forwarder(event_loop.tx());

    // Spawn background event producers
    event_loop.spawn_crossterm_reader();
    event_loop.spawn_tick_timer();
    let roots = workspace.roots();
    // Drop file-watcher events under `target/`, `.git/`, `node_modules/`,
    // gaviero's own `.gaviero/worktrees/`, and anything matching the
    // user's `files.exclude`. Without this filter a single `cargo test`
    // floods notify with thousands of events and the main loop wedges
    // inside synchronous file-tree + git-status refresh.
    let watcher_excludes = app::parse_exclude_patterns(&workspace, None);
    let mut watch_paths: Vec<PathBuf> = roots.iter().map(|p| p.to_path_buf()).collect();
    // Global and workspace-level skills (the latter beside the workspace
    // file, outside the member folders). Either `skills/` may not exist yet:
    // then its `.gaviero/` is watched, so creating it later still triggers a
    // catalog rebuild.
    for skills_dir in [
        gaviero_core::skills::SkillCatalog::global_skills_dir(),
        workspace.workspace_skills_dir(),
    ] {
        if let Some(watch) = skills_watch_path(&skills_dir)
            && !roots.iter().any(|r| watch.starts_with(r))
            && !watch_paths.contains(&watch)
        {
            watch_paths.push(watch);
        }
    }
    let watch_refs: Vec<&std::path::Path> = watch_paths.iter().map(|p| p.as_path()).collect();
    let _file_watcher = event_loop
        .spawn_file_watcher(&watch_refs, watcher_excludes)
        .ok();

    let memory_root = workspace.roots().first().map(|p| p.to_path_buf());
    // B1: pick up `memory.embedder.model` so workspace settings drive
    // the embedder choice. Empty/unknown → nomic.
    let embedder_name = workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_EMBEDDER_MODEL,
            memory_root.as_deref(),
        )
        .as_str()
        .unwrap_or("")
        .to_string();

    // Snapshot the workspace for the memory bootstrap before App takes
    // ownership — the registry needs the folder list to register
    // per-folder DBs.
    let workspace_for_memory = workspace.clone();

    // Application state
    let mut app = App::new(workspace, event_tx);
    if editor_only {
        app.enter_editor_only();
    }

    // Wire the terminal manager's bounded event channel into the TUI's unified channel
    let terminal_rx = app.terminal_manager.take_event_rx();
    event_loop.spawn_terminal_bridge(terminal_rx);

    app.restore_session();
    // A restored session may have the side panel open or focused.
    app.enforce_editor_only();
    app.show_settings_errors();

    if !editor_only {
        start_agent_services(&mut app, &event_loop, workspace_for_memory, memory_root, embedder_name);
    }

    // Main loop — drain all pending events before each render to reduce latency.
    // Without draining, each event triggers a full redraw; during streaming bursts
    // this means the MessageComplete event sits behind many intermediate events,
    // each causing an unnecessary render. Draining processes them all at once,
    // so the UI jumps straight to the final state.
    //
    // Render is gated on `needs_render`: chat stream events update state
    // immediately, but expensive Agent Chat repainting is coalesced to a small
    // budget. Most non-chat events still draw immediately.
    let mut render_scheduler = RenderScheduler::new();
    let mut needs_render = true;
    // Under a session host only (`GAVIERO_SESSION_HOST`, set by
    // `gaviero --sessions` on its children): report this session's status in
    // the window title so the host can mark its tab.
    let mut session_status: Option<String> = None;
    if std::env::var_os(session_host::HOST_ENV).is_some() {
        let status = app.session_host_status();
        write_session_status(&mut terminal, &status)?;
        session_status = Some(status);
    }
    loop {
        if app.needs_full_redraw {
            terminal.clear()?;
            app.needs_full_redraw = false;
            needs_render = true;
        }
        if needs_render {
            terminal.draw(|frame| app.render(frame))?;
            render_scheduler.record_render(&app);
            needs_render = false;
            if let Some(last) = &session_status {
                let status = app.session_host_status();
                if status != *last {
                    write_session_status(&mut terminal, &status)?;
                    session_status = Some(status);
                }
            }
        }

        // Block until at least one event arrives
        if let Some(event) = event_rx.recv().await {
            if render_scheduler.should_render(&event, &app) {
                needs_render = true;
            }
            app.handle_event(event);
            if app.take_needs_immediate_render() {
                needs_render = true;
            }
        }

        // Drain any additional events that arrived while we were rendering/handling.
        // Cap at 64 to avoid starving the renderer if events come faster than we
        // can process them (e.g., rapid file-watcher events).
        for _ in 0..64 {
            match event_rx.try_recv() {
                Ok(event) => {
                    if render_scheduler.should_render(&event, &app) {
                        needs_render = true;
                    }
                    app.handle_event(event);
                    if app.take_needs_immediate_render() {
                        needs_render = true;
                    }
                }
                Err(_) => break,
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Tell the remote client we're going away (close 4007) before the
    // socket dies with the process.
    if let Some(handle) = app.remote.handle.take() {
        let _ = handle.try_send(gaviero_remote::server::HubInput::Shutdown);
    }

    // Save session state before exit. Panel widths/heights and the chat
    // composer (input height + unsent draft) ride along in the same
    // `state.json` write (see `app/session.rs`).
    app.save_session();
    // Land queued turn-review saves/archives, or a finished review would
    // reopen as pending next launch.
    app.turn_capture.flush();

    // Explicit call so errors are reported on the happy path (the guard will
    // run it again, but that's harmless — the calls are idempotent).
    restore_terminal().context("restoring terminal")?;
    terminal.show_cursor().context("showing cursor")?;

    Ok(app.restart_requested())
}

/// Session host status title: `gaviero-session:<idle|busy|attention>`
/// (OSC 2), parsed by `session_host::session::parse_status`. Written only
/// when `GAVIERO_SESSION_HOST` is set.
fn write_session_status(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    status: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    let backend = terminal.backend_mut();
    write!(backend, "\x1b]2;gaviero-session:{status}\x07")?;
    backend.flush()
}

/// What to watch for a `.gaviero/skills` directory: the directory itself,
/// else its `.gaviero/` parent so the directory's later creation is seen,
/// else nothing.
fn skills_watch_path(skills_dir: &std::path::Path) -> Option<PathBuf> {
    if skills_dir.is_dir() {
        return Some(skills_dir.to_path_buf());
    }
    skills_dir
        .parent()
        .filter(|p| p.is_dir())
        .map(std::path::Path::to_path_buf)
}

/// Agent-side startup, skipped in an editor-only session: the Cursor hook
/// file, the egress proxy, the memory bootstrap (which brings up the MCP
/// server and provider configs on `MemoryReady`), the live code graph and
/// the remote sidecar.
fn start_agent_services(
    app: &mut App,
    event_loop: &EventLoop,
    workspace_for_memory: gaviero_core::workspace::Workspace,
    memory_root: Option<PathBuf>,
    embedder_name: String,
) {
    // Cursor's read policy lives in `.cursor/hooks.json`. Written here, before
    // any turn starts, so it is a host write and never lands in TURN REVIEW;
    // the per-spawn call in the Cursor session is then a no-op.
    {
        let cli = gaviero_core::mcp::resolver::resolve_cli_binary();
        for folder in app.workspace.folders() {
            if let Err(e) = gaviero_core::agent_guard::cursor::ensure_cursor_hooks(&folder.path, &cli)
            {
                tracing::warn!(error = %e, "could not write .cursor/hooks.json");
            }
        }
    }

    // Egress proxy (`agent.network.mode`, default `log`): every agent spawn
    // and in-process HTTP client is routed through it from here on.
    {
        let root = app
            .workspace
            .folders()
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        crate::app::egress::install(&app, &root, event_loop.tx());
    }

    // User-level Claude allow rules pre-approve file/shell access and so
    // bypass gaviero's prompts; `.gaviero/settings.json` deny rules still win.
    if let Some(home) = dirs::home_dir() {
        let overlaps = gaviero_core::agent_guard::claude::user_allow_overlaps(&home);
        if !overlaps.is_empty() {
            app.chat_state.add_system_message(&format!(
                "Claude user settings pre-approve: {} — these bypass gaviero's prompts \
                 (deny rules from .gaviero/settings.json still apply).",
                overlaps.join(", ")
            ));
        }
    }

    // Spawn background memory initialization (non-blocking).
    // Constructs the multi-DB registry: global + workspace + lazy
    // per-folder stores keyed by every folder listed in the workspace.
    {
        let tx = event_loop.tx();
        let ws = workspace_for_memory;
        tokio::task::spawn(async move {
            let init_result = tokio::task::spawn_blocking(move || match memory_root {
                Some(root) => gaviero_core::memory::init_workspace_stores_with_embedder_name(
                    &root,
                    &ws,
                    &embedder_name,
                ),
                None => {
                    // No workspace root → fall back to a single-store
                    // registry pointing at the legacy default DB. This
                    // path is hit when the TUI is invoked with no
                    // folder argument; rare but supported.
                    gaviero_core::memory::init_with_embedder_name(None, &embedder_name)
                        .map(gaviero_core::memory::MemoryStores::from_single_store)
                }
            })
            .await;
            match init_result {
                Ok(Ok(stores)) => {
                    let _ = tx.send(event::Event::MemoryReady(stores));
                }
                Ok(Err(e)) => {
                    tracing::warn!("Memory initialization failed: {}", e);
                }
                Err(e) => {
                    tracing::warn!("Memory initialization panicked: {}", e);
                }
            }
        });
    }

    // Start every folder's live code graph (a background writer thread per
    // folder; its first job is the startup scan). Non-blocking.
    app.graph_services.start_all();

    // Remote sidecar (Plan A A6, Plan C §2.2). Enabled by default; host
    // detection, certificate provisioning, and binding run in a background
    // task that reports through `Event::RemoteStarted` /
    // `Event::RemoteUnavailable` — startup never waits for it (invariant
    // 13). A failure disables the sidecar, never the TUI; `/remote` shows
    // the same diagnostics on demand.
    tokio::spawn(crate::app::remote_setup::bootstrap(
        app.workspace.clone(),
        event_loop.tx(),
    ));
}
