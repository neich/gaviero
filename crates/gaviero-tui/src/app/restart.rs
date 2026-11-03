//! App side of restart-on-new-build (see [`crate::supervisor`]).
//!
//! Every tick polls the [`BuildWatch`]. Once a new build is installed, a
//! supervised TUI quits as soon as nothing is running — the normal quit path
//! saves the session and flushes turn reviews, and the supervisor starts the
//! new build in the same pane. An unsupervised TUI only says a restart is
//! needed.

use std::time::{Duration, Instant};

use super::App;
use crate::supervisor::BuildWatch;

/// No restart within this long of the last key, click or paste: the user is
/// working in this instance right now.
const INPUT_QUIET: Duration = Duration::from_secs(15);

/// Repeat the "restart pending" status line this often while it waits.
const NOTICE_EVERY: Duration = Duration::from_secs(30);

pub(crate) struct RestartState {
    watch: BuildWatch,
    supervised: bool,
    last_input: Instant,
    last_notice: Option<Instant>,
    /// Set when the app quits to restart; `main` turns it into the exit code.
    pub(crate) requested: bool,
}

impl RestartState {
    pub(crate) fn new() -> Self {
        Self {
            watch: BuildWatch::for_current_exe(),
            supervised: crate::supervisor::is_supervised(),
            last_input: Instant::now(),
            last_notice: None,
            requested: false,
        }
    }

    pub(crate) fn note_input(&mut self) {
        self.last_input = Instant::now();
    }
}

/// Why a restart has to wait, or `None` when nothing is running.
fn blocker(app: &App, now: Instant) -> Option<&'static str> {
    let agent_running = !app.acp_tasks.is_empty()
        || app
            .chat_state
            .conversations
            .iter()
            .any(|c| c.is_streaming || c.has_running_background_agents());
    if agent_running {
        return Some("an agent is running");
    }
    let terminal_busy = app.terminal_manager.tab_order().iter().any(|id| {
        app.terminal_manager
            .instance(*id)
            .is_some_and(|inst| inst.shell_state.is_running())
    });
    if terminal_busy {
        return Some("a terminal command is running");
    }
    if app.buffers.iter().any(|b| b.modified) {
        return Some("there are unsaved files");
    }
    if app.diff_review.is_some() || app.batch_review.is_some() {
        return Some("a review is open");
    }
    let dialog_open = app.quit_confirm
        || app.codex_trust_dialog.is_some()
        || app.settings_error_dialog.is_some()
        || app.tree_dialog.is_some()
        || app.move_state.is_some()
        || app.bulk_op_state.is_some()
        || app.definition_picker.is_some();
    if dialog_open {
        return Some("a dialog is open");
    }
    if now.duration_since(app.restart.last_input) < INPUT_QUIET {
        return Some("you are using it");
    }
    None
}

pub(super) fn on_tick(app: &mut App) {
    let now = Instant::now();
    let newly_ready = app.restart.watch.poll(now);
    if !app.restart.watch.ready() || app.restart.requested {
        return;
    }

    if !app.restart.supervised {
        if newly_ready {
            app.status_message = Some((
                "New gaviero build installed: quit and relaunch to use it".to_string(),
                now,
            ));
        }
        return;
    }

    match blocker(app, now) {
        None => {
            tracing::info!("new build installed and nothing running: restarting");
            app.restart.requested = true;
            app.should_quit = true;
        }
        Some(reason) => {
            let due = app
                .restart
                .last_notice
                .is_none_or(|last| now.duration_since(last) >= NOTICE_EVERY);
            if due {
                app.restart.last_notice = Some(now);
                app.status_message = Some((
                    format!("New gaviero build installed: restarting once idle ({reason})"),
                    now,
                ));
            }
        }
    }
}
