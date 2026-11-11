//! App side of restart-on-new-build (see [`crate::supervisor`]).
//!
//! Every tick polls the [`BuildWatch`]. Once a new build is installed, the
//! decision is taken once:
//!
//! - **A task is running** (agent turn, background agent, terminal command):
//!   the instance is left alone and a popup says a new version is installed
//!   and gaviero must be restarted by hand. No automatic restart follows.
//! - **Not supervised** (`GAVIERO_NO_SUPERVISOR`, no TTY at launch): same
//!   popup — nothing would relaunch us.
//! - **Otherwise** the instance restarts as soon as nothing transient is in
//!   the way (unsaved files, an open review or dialog, recent input). The
//!   normal quit path saves the session and flushes turn reviews, and the
//!   supervisor starts the new build in the same pane. A task that starts
//!   while it waits turns this into the popup case.
//!
//! A tab of a supervised session host (`GAVIERO_SESSION_HOST_RESTARTS`) does
//! none of this: the host restarts every tab together once all are idle
//! (`session_host/restart.rs`).

use std::time::{Duration, Instant};

use super::App;
use crate::supervisor::BuildWatch;

/// No restart within this long of the last key, click or paste: the user is
/// working in this instance right now.
const INPUT_QUIET: Duration = Duration::from_secs(15);

/// Repeat the "restart pending" status line this often while it waits.
const NOTICE_EVERY: Duration = Duration::from_secs(30);

/// Why this instance was not restarted automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManualRestart {
    Busy(&'static str),
    Unsupervised,
}

pub(crate) struct RestartState {
    watch: BuildWatch,
    supervised: bool,
    /// The session host restarts this instance; it never acts on a new
    /// build itself.
    host_restarts: bool,
    last_input: Instant,
    last_notice: Option<Instant>,
    /// Set when the app quits to restart; `main` turns it into the exit code.
    pub(crate) requested: bool,
    /// Set once the instance was left running on the old build; no automatic
    /// restart after that.
    manual: Option<ManualRestart>,
    /// The "restart manually" popup is showing.
    pub(crate) popup_open: bool,
}

impl RestartState {
    pub(crate) fn new() -> Self {
        Self {
            watch: BuildWatch::for_current_exe(),
            supervised: crate::supervisor::is_supervised(),
            host_restarts: std::env::var_os(crate::session_host::HOST_RESTARTS_ENV).is_some(),
            last_input: Instant::now(),
            last_notice: None,
            requested: false,
            manual: None,
            popup_open: false,
        }
    }

    pub(crate) fn note_input(&mut self) {
        self.last_input = Instant::now();
    }

    /// Why the popup is showing, while it is.
    pub(crate) fn popup(&self) -> Option<ManualRestart> {
        self.manual.filter(|_| self.popup_open)
    }
}

/// A running task that a restart would cut short.
pub(super) fn running_task(app: &App) -> Option<&'static str> {
    if agent_running(app) {
        return Some("an agent is running");
    }
    if app.swarm_dashboard.running {
        return Some("a swarm script is running");
    }
    if terminal_busy(app) {
        return Some("a terminal command is running");
    }
    None
}

/// What ending this instance now would cut off, as the keywords a session
/// host reads from the status title (`session_host::session::parse_work`):
/// `agent`, `swarm`, `terminal` (a command running in a shell), `unsaved`.
pub(super) fn work_in_progress(app: &App) -> Vec<&'static str> {
    let mut work = Vec::new();
    if agent_running(app) {
        work.push("agent");
    }
    if app.swarm_dashboard.running {
        work.push("swarm");
    }
    if terminal_busy(app) {
        work.push("terminal");
    }
    if app.buffers.iter().any(|b| b.modified) {
        work.push("unsaved");
    }
    work
}

fn agent_running(app: &App) -> bool {
    !app.acp_tasks.is_empty()
        || app
            .chat_state
            .conversations
            .iter()
            .any(|c| c.is_streaming || c.has_running_background_agents())
}

/// A command in one of the shells seems to wait for the user
/// (`TerminalInstance::awaiting_input`).
pub(super) fn terminal_awaiting_input(app: &App) -> bool {
    app.terminal_manager.tab_order().iter().any(|id| {
        app.terminal_manager
            .instance(*id)
            .is_some_and(|inst| inst.awaiting_input())
    })
}

fn terminal_busy(app: &App) -> bool {
    app.terminal_manager.tab_order().iter().any(|id| {
        app.terminal_manager
            .instance(*id)
            .is_some_and(|inst| inst.shell_state.is_running())
    })
}

/// Something that only delays the restart until it clears.
fn transient_blocker(app: &App, now: Instant) -> Option<&'static str> {
    if app.buffers.iter().any(|b| b.modified) {
        return Some("there are unsaved files");
    }
    if app.diff_review.is_some() || app.batch_review.is_some() {
        return Some("a review is open");
    }
    let dialog_open = app.quit_confirm
        || app.codex_trust_dialog.is_some()
        || app.settings_error_dialog.is_some()
        || app.syntax_error_dialog.is_some()
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

enum Decision {
    Restart,
    Wait(&'static str),
    Manual(ManualRestart),
}

fn decide(supervised: bool, task: Option<&'static str>, blocker: Option<&'static str>) -> Decision {
    if !supervised {
        return Decision::Manual(ManualRestart::Unsupervised);
    }
    if let Some(task) = task {
        return Decision::Manual(ManualRestart::Busy(task));
    }
    match blocker {
        Some(reason) => Decision::Wait(reason),
        None => Decision::Restart,
    }
}

pub(super) fn on_tick(app: &mut App) {
    if app.restart.host_restarts {
        return;
    }
    let now = Instant::now();
    app.restart.watch.poll(now);
    if !app.restart.watch.ready() || app.restart.requested || app.restart.manual.is_some() {
        return;
    }

    let decision = decide(
        app.restart.supervised,
        running_task(app),
        transient_blocker(app, now),
    );
    match decision {
        Decision::Restart => {
            tracing::info!("new build installed and nothing running: restarting");
            app.restart.requested = true;
            app.should_quit = true;
        }
        Decision::Wait(reason) => {
            let due = app
                .restart
                .last_notice
                .is_none_or(|last| now.duration_since(last) >= NOTICE_EVERY);
            if due {
                app.restart.last_notice = Some(now);
                app.status_message = Some((
                    format!("New gaviero version installed: restarting once idle ({reason})"),
                    now,
                ));
            }
        }
        Decision::Manual(why) => {
            tracing::info!(?why, "new build installed; leaving this instance running");
            app.restart.manual = Some(why);
            app.restart.popup_open = true;
        }
    }
}

/// Any key closes the popup; the key is consumed.
pub(super) fn handle_popup_key(app: &mut App) {
    app.restart.popup_open = false;
}

/// Text of the popup.
pub(super) fn popup_lines(why: ManualRestart) -> Vec<String> {
    let cause = match why {
        ManualRestart::Busy(task) => {
            format!("It was installed while this instance was busy ({task}), so it was not restarted.")
        }
        ManualRestart::Unsupervised => {
            "This instance was started without the gaviero supervisor, so it cannot restart itself."
                .to_string()
        }
    };
    vec![
        String::new(),
        "A new version of gaviero has been installed.".to_string(),
        String::new(),
        cause,
        "This instance keeps running the previous version.".to_string(),
        "Quit (Ctrl+Q) and start gaviero again when you are done to use the new one.".to_string(),
        String::new(),
        "[any key] Dismiss".to_string(),
        String::new(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_task_means_a_manual_restart_even_when_otherwise_idle() {
        assert!(matches!(
            decide(true, Some("an agent is running"), None),
            Decision::Manual(ManualRestart::Busy("an agent is running"))
        ));
    }

    #[test]
    fn a_running_task_wins_over_a_transient_blocker() {
        assert!(matches!(
            decide(true, Some("a terminal command is running"), Some("you are using it")),
            Decision::Manual(ManualRestart::Busy(_))
        ));
    }

    #[test]
    fn transient_blockers_only_delay_the_restart() {
        assert!(matches!(
            decide(true, None, Some("there are unsaved files")),
            Decision::Wait("there are unsaved files")
        ));
        assert!(matches!(decide(true, None, None), Decision::Restart));
    }

    #[test]
    fn an_unsupervised_instance_always_asks_for_a_manual_restart() {
        assert!(matches!(
            decide(false, None, None),
            Decision::Manual(ManualRestart::Unsupervised)
        ));
    }

    #[test]
    fn the_popup_names_the_running_task() {
        let text = popup_lines(ManualRestart::Busy("an agent is running")).join("\n");
        assert!(text.contains("new version of gaviero has been installed"));
        assert!(text.contains("an agent is running"));
        assert!(text.contains("Quit (Ctrl+Q)"));
    }
}
