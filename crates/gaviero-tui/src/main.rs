mod app;
mod editor;
mod event;
mod keymap;
mod notify;
mod panels;
mod platform;
mod setup;
mod theme;
mod widgets;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::IsTerminal;
use std::path::PathBuf;
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
/// updates immediate. Keyboard, mouse, resize, terminal, review, and lifecycle
/// events still repaint immediately; only visible active chat stream output and
/// spinner-only ticks are budgeted.
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
            | event::Event::StreamingStatus { conv_id, .. } => {
                self.should_render_chat_stream_event(conv_id, app)
            }
            event::Event::Tick => self.should_render_tick(app),
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

#[tokio::main]
async fn main() -> Result<()> {
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

    // Simplify away Windows' `\\?\` verbatim prefix — this path becomes
    // the workspace root and flows into shell cwds, prompts, and configs.
    let path = gaviero_core::util::fs::canonicalize_simplified(&cli.path)
        .with_context(|| format!("resolving path: {}", cli.path.display()))?;

    // Launch dispatch. An explicit `*.gaviero-workspace` argument always wins.
    // `--workspace` on a directory reuses the workspace file already inside it;
    // when there is none — or, in plain folder mode, when there is no
    // `.gaviero/settings.json` — the first-run wizard runs before the editor
    // and writes the configuration it collects.
    let is_workspace_file = path
        .extension()
        .is_some_and(|ext| ext == "gaviero-workspace");
    let launch_mode = if cli.workspace {
        setup::LaunchMode::Workspace
    } else {
        setup::LaunchMode::Folder
    };

    let mut workspace_file: Option<PathBuf> = if is_workspace_file {
        Some(path.clone())
    } else if cli.workspace {
        setup::existing_workspace_file(&path)
    } else {
        None
    };
    let mut init_providers = false;

    // Skip the wizard without a TTY (piped stdin, CI): it needs raw mode, and
    // built-in defaults still give a working session.
    if workspace_file.is_none()
        && !is_workspace_file
        && setup::needs_setup(&path, launch_mode)
        && std::io::stdin().is_terminal()
        && let Some(outcome) = setup::run(&path, launch_mode)?
    {
        workspace_file = outcome.workspace_file;
        init_providers = outcome.init_providers;
    }

    let workspace = match &workspace_file {
        Some(file) => gaviero_core::workspace::Workspace::load(file)?,
        None => gaviero_core::workspace::Workspace::single_folder(path),
    };

    if init_providers {
        let written = setup::synthesize_provider_configs(&workspace);
        tracing::info!(
            "first-run setup wrote {} agent config file(s)",
            written.len()
        );
    }

    // C1: prompt the user for consent on a pending typed-stores
    // migration BEFORE entering raw mode. If the user declines, exit
    // cleanly without taking the snapshot or migrating. The plan's
    // anti-pattern explicitly forbids silent migration here — a yes/no
    // stdin prompt is the minimum non-silent surface.
    {
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
            return Ok(());
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
    let global_skills = gaviero_core::skills::SkillCatalog::global_skills_dir();
    if global_skills.is_dir() {
        watch_paths.push(global_skills);
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

    // Wire the terminal manager's bounded event channel into the TUI's unified channel
    let terminal_rx = app.terminal_manager.take_event_rx();
    event_loop.spawn_terminal_bridge(terminal_rx);

    app.restore_session();

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

    // Warm up the code graph in the background (non-blocking).
    crate::app::session::warm_up_repo_map(&app);

    // Remote sidecar (Plan A A6). Disabled by default; a bind failure or a
    // missing certificate disables the sidecar, never the TUI. `/remote`
    // reports the same diagnostics on demand.
    {
        use crate::app::remote_setup;
        match remote_setup::resolve_config(&app.workspace) {
            Ok(config) if config.enabled => {
                app.remote.max_prompt_bytes = config.max_prompt_bytes as usize;
                match remote_setup::check_availability(&config) {
                    Ok(availability) => match remote_setup::load_or_create_token(&config) {
                        Ok(token) => {
                            let instance_id =
                                gaviero_remote::pairing::generate_token()[..16].to_string();
                            match remote_setup::start(
                                &config,
                                availability,
                                token,
                                instance_id,
                                event_loop.tx(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    tracing::info!(
                                        port = config.port,
                                        host = %config.magic_dns_host,
                                        "remote sidecar listening"
                                    );
                                    app.remote.handle = Some(handle);
                                    app.remote.snapshot_dirty = true;
                                }
                                Err(e) => tracing::warn!("remote sidecar disabled: {e}"),
                            }
                        }
                        Err(e) => tracing::warn!("remote sidecar disabled: token error: {e}"),
                    },
                    Err(e) => tracing::warn!("remote sidecar unavailable: {e}"),
                }
            }
            Ok(_) => tracing::debug!("remote sidecar disabled by settings"),
            Err(e) => tracing::warn!("remote sidecar not configured: {e}"),
        }
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
        }

        // Block until at least one event arrives
        if let Some(event) = event_rx.recv().await {
            if render_scheduler.should_render(&event, &app) {
                needs_render = true;
            }
            app.handle_event(event);
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

    // Save session state before exit
    app.save_session();

    // Explicit call so errors are reported on the happy path (the guard will
    // run it again, but that's harmless — the calls are idempotent).
    restore_terminal().context("restoring terminal")?;
    terminal.show_cursor().context("showing cursor")?;

    Ok(())
}
