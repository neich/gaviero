mod app;
mod editor;
mod event;
mod keymap;
mod panels;
mod theme;
mod widgets;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::path::PathBuf;

use app::App;
use event::EventLoop;

#[derive(Parser)]
#[command(name = "gaviero", about = "Terminal code editor for AI agent orchestration")]
struct Cli {
    /// Path to open (directory or .gaviero-workspace file)
    #[arg(default_value = ".")]
    path: PathBuf,
}

/// Restore the host terminal to a sane state. Called on every exit path:
/// normal exit, `?` early returns, and panics.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    );
    let _ = crossterm::cursor::Show;
    // Print a newline so the shell prompt starts on a clean line
    let _ = execute!(std::io::stdout(), crossterm::cursor::Show);
}

/// RAII guard that restores the terminal when dropped — covers `?` returns
/// and normal scope exit.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Write tracing output to a log file (stderr is hidden in TUI mode)
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gaviero");
    std::fs::create_dir_all(&log_dir)
        .context("creating log directory")?;
    let log_file = std::fs::File::create(log_dir.join("gaviero.log"))
        .context("creating log file")?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let cli = Cli::parse();

    let path = std::fs::canonicalize(&cli.path)
        .with_context(|| format!("resolving path: {}", cli.path.display()))?;

    let workspace = if path.extension().is_some_and(|ext| ext == "gaviero-workspace") {
        gaviero_core::workspace::Workspace::load(&path)?
    } else {
        gaviero_core::workspace::Workspace::single_folder(path)
    };

    // Install panic hook BEFORE entering raw mode so that any panic
    // (including inside ratatui, crossterm, or our own code) restores
    // the terminal before printing the panic message.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // Terminal setup
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)
        .context("entering alternate screen")?;

    // RAII guard: if anything below returns Err via `?`, the terminal
    // is still restored when `_guard` is dropped.
    let _guard = TerminalGuard;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).context("creating terminal")?;

    // Event loop setup
    let mut event_loop = EventLoop::new();
    let event_tx = event_loop.tx();
    let mut event_rx = event_loop.take_rx();

    // Spawn background event producers
    event_loop.spawn_crossterm_reader();
    event_loop.spawn_tick_timer();
    let roots = workspace.roots();
    let _file_watcher = event_loop.spawn_file_watcher(&roots).ok();

    // Application state
    let mut app = App::new(workspace, event_tx);

    // Wire the terminal manager's bounded event channel into the TUI's unified channel
    let terminal_rx = app.terminal_manager.take_event_rx();
    event_loop.spawn_terminal_bridge(terminal_rx);

    app.restore_session();

    // Spawn background memory initialization (non-blocking)
    {
        let tx = event_loop.tx();
        tokio::task::spawn(async move {
            match tokio::task::spawn_blocking(|| gaviero_core::memory::init(None)).await {
                Ok(Ok(store)) => {
                    let _ = tx.send(event::Event::MemoryReady(store));
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

    // Main loop — drain all pending events before each render to reduce latency.
    // Without draining, each event triggers a full redraw; during streaming bursts
    // this means the MessageComplete event sits behind many intermediate events,
    // each causing an unnecessary render. Draining processes them all at once,
    // so the UI jumps straight to the final state.
    loop {
        if app.needs_full_redraw {
            terminal.clear()?;
            app.needs_full_redraw = false;
        }
        terminal.draw(|frame| app.render(frame))?;

        // Block until at least one event arrives
        if let Some(event) = event_rx.recv().await {
            app.handle_event(event);
        }

        // Drain any additional events that arrived while we were rendering/handling.
        // Cap at 64 to avoid starving the renderer if events come faster than we
        // can process them (e.g., rapid file-watcher events).
        for _ in 0..64 {
            match event_rx.try_recv() {
                Ok(event) => app.handle_event(event),
                Err(_) => break,
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Save session state before exit
    app.save_session();

    // Explicit cleanup (guard will also run, but that's harmless — the calls are idempotent)
    // We keep the explicit block so errors are reported on the happy path.
    disable_raw_mode().context("disabling raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )
    .context("leaving alternate screen")?;
    terminal.show_cursor().context("showing cursor")?;

    Ok(())
}
