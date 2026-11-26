//! One session: a `gaviero` child in a pseudo-terminal, its emulated screen,
//! and the status it reports through its window title.

use super::shell::{self, ShellWatch};
use super::target::Target;
use super::{CONTROL_ENV, HOST_ENV, HOST_RESTARTS_ENV, HostEvent, encode};
use crate::panels::terminal::TerminalSelectionState;
use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

/// Title prefix a session child uses to report its status (`main.rs`).
const STATUS_TITLE_PREFIX: &str = "gaviero-session:";

/// Keeps the last window title (OSC 0 / OSC 2) the child set, and the
/// folder it reported last (OSC 7, `super::osc7`) until the host takes it.
#[derive(Default)]
pub(crate) struct TitleCb {
    pub title: Option<String>,
    pub reported_path: Option<String>,
    /// Base64 text the child asked to copy (OSC 52), until the host takes it.
    pub copied: Option<Vec<u8>>,
}

impl vt100::Callbacks for TitleCb {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        self.copied = Some(data.to_vec());
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() == Some(&&b"7"[..]) {
            let url = params[1..].join(&b";"[..]);
            if let Some(path) = super::osc7::decode_file_url(&url) {
                self.reported_path = Some(path);
            }
        }
    }
}

/// What a session is doing, from its status title: the two circles on
/// its tab (`render::tab_badges`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Activity {
    /// Waiting for the user to continue the turn: a permission prompt or an
    /// agent's question (`attention`).
    pub waiting: bool,
    /// A turn finished and its review awaits decisions (`review`). The turn
    /// itself is done — the user can continue whenever — so the tab shows the
    /// finished colour, not the waiting one.
    pub review: bool,
    /// An agent or a swarm script is running.
    pub agent: bool,
    /// A command is running in one of the session's shells.
    pub shell: bool,
    /// A shell command seems to wait for the user (`shell-input`).
    pub shell_waiting: bool,
}

/// The activity a child reports in its window title
/// (`gaviero-session:<idle|busy|attention>[:work,...]`); anything else is
/// none. A child that reports `busy` with no detail (an older build) counts
/// as an agent running.
pub(crate) fn parse_activity(title: Option<&str>) -> Activity {
    let Some((status, detail)) = status_fields(title) else {
        return Activity::default();
    };
    let has = |keyword: &str| detail.split(',').any(|k| k == keyword);
    let agent = has("agent") || has("swarm");
    let shell = has("terminal");
    Activity {
        waiting: status == "attention",
        // Reported beside the work keywords, so it is not `attention`.
        review: has("review"),
        agent: agent || (status == "busy" && !shell),
        shell,
        shell_waiting: shell && has("shell-input"),
    }
}

/// Which of a background tab's work ended between `before` and `after`:
/// `(agent, shell)`. Each sets that tab's "finished" circle until the tab is
/// made current.
pub(crate) fn finished(before: Activity, after: Activity) -> (bool, bool) {
    (before.agent && !after.agent, before.shell && !after.shell)
}

/// What closing the child now would cut off, from its status title, as the
/// phrases the close guard shows. A child that reports `busy` with no
/// detail (an older build) has "a task" running.
pub(crate) fn parse_work(title: Option<&str>) -> Vec<&'static str> {
    let Some((status, detail)) = status_fields(title) else {
        return Vec::new();
    };
    let mut work: Vec<&'static str> = detail
        .split(',')
        .filter_map(|keyword| match keyword {
            "agent" => Some("an agent is running"),
            "swarm" => Some("a swarm is running"),
            "terminal" => Some("a shell is running a command"),
            "unsaved" => Some("files are not saved"),
            _ => None,
        })
        .collect();
    if work.is_empty() && status == "busy" {
        work.push("a task is running");
    }
    work
}

/// The child has files modified and not saved (`unsaved` in its status
/// title): its tab gets the `●` an editor tab shows for an unsaved buffer.
/// Kept apart from [`Activity`], whose changes latch and clear the badges'
/// "finished" marks — saving a file is not the end of any work.
pub(crate) fn parse_unsaved(title: Option<&str>) -> bool {
    status_fields(title).is_some_and(|(_, detail)| detail.split(',').any(|k| k == "unsaved"))
}

/// `(status, detail)` of a status title; detail is empty when absent.
fn status_fields(title: Option<&str>) -> Option<(&str, &str)> {
    let rest = title?.strip_prefix(STATUS_TITLE_PREFIX)?;
    Some(rest.split_once(':').unwrap_or((rest, "")))
}

/// The tab label for `path`: a workspace file's stem, a folder's name
/// (after [`shell::SHELL_MARK`] for a shell); ` (2)`, ` (3)`, … when `taken`
/// already holds that label.
pub(crate) fn label_for(target: &Target, taken: &[String]) -> String {
    let base = match target {
        Target::Local(path) => local_label(path),
        Target::Shell(dir) => format!("{} {}", shell::SHELL_MARK, local_label(dir)),
        Target::Ssh { host, path } => {
            let name = path
                .as_deref()
                .and_then(|p| p.trim_end_matches('/').rsplit('/').next())
                .filter(|name| !name.is_empty())
                .unwrap_or("~");
            format!("{host}:{name}")
        }
    };
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|label| !taken.contains(label))
        .expect("an unused label")
}

/// A local tab's label: the folder name, or a workspace file's stem.
fn local_label(path: &Path) -> String {
    let is_workspace = path
        .extension()
        .is_some_and(|ext| ext == "gaviero-workspace");
    let name = if is_workspace {
        path.file_stem()
    } else {
        path.file_name()
    };
    name.map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) struct Session {
    pub id: u64,
    pub target: Target,
    pub label: String,
    pub parser: vt100::Parser<TitleCb>,
    master: Box<dyn MasterPty + Send>,
    input: std::sync::mpsc::Sender<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pub started: Instant,
    pub launch_state: PathBuf,
    pub control: PathBuf,
    pub activity: Activity,
    /// What closing this session now would cut off (`parse_work`).
    pub work: Vec<&'static str>,
    /// The child reports files modified and not saved (`parse_unsaved`):
    /// drawn as `●` before the tab label. Never set for a shell tab.
    pub unsaved: bool,
    /// An agent turn / a shell command ended while this tab was not
    /// current. The agent mark clears when the user leaves the tab after
    /// making it current, the shell mark as soon as the tab is made current;
    /// both when its activity changes while it is current.
    pub agent_done_unseen: bool,
    pub shell_done_unseen: bool,
    /// Exited for a restart too soon after starting; a delayed
    /// [`HostEvent::Relaunch`] is on its way.
    pub restart_pending: bool,
    /// An SSH tab whose connection was lost (`ssh` exited with 255): kept
    /// with its last screen until the user reconnects (Enter) or closes it.
    pub disconnected: bool,
    /// The child has reported its folder (OSC 7), which gaviero does right
    /// before its event loop: from then on it reads focus reports as such.
    pub focus_ready: bool,
    /// The focus report last sent (`true` = gained), so each change is
    /// sent once.
    focus_sent: Option<bool>,
    /// A shell tab's command state; `None` for a gaviero session, whose
    /// activity comes from its status title instead.
    pub shell: Option<ShellWatch>,
    /// A shell tab's mouse selection (the embedded terminal's): drawn over
    /// the screen, copied when the button comes up, cleared by input, a new
    /// click or a resize. A gaviero session selects for itself.
    pub selection: TerminalSelectionState,
}

/// The largest copy taken from a session (OSC 52), decoded.
const COPY_LIMIT: usize = 1 << 20;

/// The text in an OSC 52 copy: base64 of UTF-8, at most [`COPY_LIMIT`]
/// bytes once decoded.
fn copied_text(data: &[u8]) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    if bytes.len() > COPY_LIMIT {
        tracing::warn!(bytes = bytes.len(), "ignoring an oversized clipboard copy");
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `gaviero <path>` as a supervised session-host child.
fn local_command(
    path: &Path,
    launch_state: &Path,
    control: &Path,
    host_restarts: bool,
) -> Result<CommandBuilder> {
    let exe = std::env::current_exe().context("locating the gaviero executable")?;
    let mut cmd = CommandBuilder::new(exe);
    cmd.arg(path);
    let cwd = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    cmd.cwd(cwd);
    for (key, value) in crate::supervisor::child_env(launch_state) {
        cmd.env(key, value);
    }
    cmd.env(HOST_ENV, "1");
    cmd.env(CONTROL_ENV, control);
    // The tab logs where its host does (`host_log_dir`).
    cmd.env(crate::LOG_DIR_ENV, super::host_log_dir());
    // The host's own restart file is no business of its tabs.
    cmd.env_remove(crate::supervisor::HOST_RESTART_ENV);
    if host_restarts {
        cmd.env(HOST_RESTARTS_ENV, "1");
    }
    Ok(cmd)
}

/// The system `ssh` running gaviero on `host` ([`Target::remote_command`]).
/// None of the session-host variables are set: they would only reach the
/// local `ssh`, and the remote gaviero gets `GAVIERO_SESSION_HOST` from the
/// remote command. It supervises itself there, so a remote build update
/// restarts it on the remote side.
fn ssh_command(target: &Target, host: &str, gaviero_override: Option<&str>) -> Result<CommandBuilder> {
    let remote = target
        .remote_command(gaviero_override)
        .map_err(anyhow::Error::msg)?;
    let mut cmd = CommandBuilder::new(super::target::ssh_program());
    for arg in super::target::ssh_args(host, &remote) {
        cmd.arg(arg);
    }
    if let Some(home) = dirs::home_dir().or_else(|| std::env::current_dir().ok()) {
        cmd.cwd(home);
    }
    cmd.env_remove(crate::supervisor::HOST_RESTART_ENV);
    Ok(cmd)
}

/// Session `id`'s launch-state file (`supervisor::LAUNCH_STATE_ENV`).
pub(crate) fn launch_state_path(state_dir: &Path, id: u64) -> PathBuf {
    state_dir.join(format!("{id}.launch"))
}

/// Start a session in a `rows` × `cols` pseudo-terminal: `gaviero <path>`
/// for a local target, the system `ssh` running gaviero on the remote for
/// an SSH one, the user's shell for a shell tab ([`shell::command`]).
/// Output and end-of-output arrive on `tx` as
/// [`HostEvent::Output`] / [`HostEvent::Exited`].
/// `host_restarts`: the host restarts this child onto a new build, so the
/// child must not do it itself ([`HOST_RESTARTS_ENV`]).
/// `gaviero_override`: where gaviero is on an SSH target's machine
/// (`sshHosts.<host>.gaviero`); ignored for local targets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    id: u64,
    target: &Target,
    label: String,
    rows: u16,
    cols: u16,
    state_dir: &Path,
    tx: UnboundedSender<HostEvent>,
    host_restarts: bool,
    gaviero_override: Option<&str>,
) -> Result<Session> {
    let rows = rows.max(2);
    let cols = cols.max(10);
    let launch_state = launch_state_path(state_dir, id);
    let control = state_dir.join(format!("{id}.ctl"));

    let cmd = match target {
        Target::Local(path) => local_command(path, &launch_state, &control, host_restarts)?,
        Target::Ssh { host, .. } => ssh_command(target, host, gaviero_override)?,
        Target::Shell(dir) => shell::command(dir)?,
    };
    // A gaviero session draws its own history; a shell's scrolls off the
    // top and is kept here for the wheel.
    let scrollback = if target.is_shell() { shell::scrollback_lines() } else { 0 };

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("opening a pseudo-terminal")?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("starting a session for {}", target.display()))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("cloning the pseudo-terminal reader")?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("taking the pseudo-terminal writer")?;

    std::thread::Builder::new()
        .name(format!("session-{id}-reader"))
        .spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(HostEvent::Exited { id, code: None });
                        break;
                    }
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        if tx.send(HostEvent::Output { id, bytes }).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .context("starting the session reader")?;

    // A child that stops reading its input must never block the host loop,
    // so writes happen on their own thread.
    let (input, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name(format!("session-{id}-writer"))
        .spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })
        .context("starting the session writer")?;

    Ok(Session {
        id,
        target: target.clone(),
        label,
        parser: vt100::Parser::new_with_callbacks(rows, cols, scrollback, TitleCb::default()),
        master: pair.master,
        input,
        child,
        started: Instant::now(),
        launch_state,
        control,
        activity: Activity::default(),
        work: Vec::new(),
        unsaved: false,
        agent_done_unseen: false,
        shell_done_unseen: false,
        restart_pending: false,
        disconnected: false,
        focus_ready: false,
        focus_sent: None,
        shell: target.is_shell().then(|| ShellWatch::new(Instant::now())),
        selection: TerminalSelectionState::default(),
    })
}

impl Session {
    /// Queue bytes for the child's input.
    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
    }

    /// Queue what the user typed, pasted or clicked. A shell tab notes it
    /// (the command's question is answered) and leaves its history view.
    pub fn send_input(&mut self, bytes: Vec<u8>) {
        if let Some(watch) = &mut self.shell {
            watch.note_input(Instant::now());
            self.parser.screen_mut().set_scrollback(0);
            self.selection.clear();
        }
        self.send(bytes);
    }

    /// Scroll a shell tab's history view by one wheel notch.
    pub fn scroll_history(&mut self, up: bool) {
        let screen = self.parser.screen_mut();
        let now = screen.scrollback();
        screen.set_scrollback(if up {
            now + shell::WHEEL_LINES
        } else {
            now.saturating_sub(shell::WHEEL_LINES)
        });
    }

    /// The left button went down at screen `row`, `col`: a new selection
    /// starts there (any earlier one goes).
    pub fn select_start(&mut self, row: u16, col: u16) {
        let row = row.min(self.parser.screen().size().0.saturating_sub(1));
        self.selection.start(row, col, self.parser.screen());
    }

    /// The pointer moved to `row`, `col` with the button held; `row` is
    /// `None` above the screen (over the tab bar), where the history
    /// scrolls back one line per move so a selection can reach past the top.
    pub fn select_extend(&mut self, row: Option<u16>, col: u16) {
        let row = match row {
            Some(row) => row.min(self.parser.screen().size().0.saturating_sub(1)),
            None => {
                let screen = self.parser.screen_mut();
                let now = screen.scrollback();
                screen.set_scrollback(now + 1);
                0
            }
        };
        self.selection.extend(row, col, self.parser.screen());
    }

    /// The button came up: the selected text, kept highlighted until the
    /// next click or input. A click without a drag selects nothing and
    /// clears the selection.
    pub fn select_end(&mut self) -> Option<String> {
        let text = self.selection.extract_text(self.parser.screen_mut());
        if text.is_none() {
            self.selection.clear();
        }
        self.selection.dragging = false;
        text
    }

    /// Tell a local child whether its tab is the one shown (`ESC[I` /
    /// `ESC[O`, which ConPTY turns into focus events). Every hidden tab's
    /// gaviero would otherwise believe it has focus and act on input it
    /// detects outside its own console: a Ctrl+V anywhere attached the
    /// clipboard image in every idle local chat. Sent only once the child
    /// is ready (before that it may be on a setup screen reading keys), and
    /// never to an SSH tab, whose remote end may be a shell, nor to a shell
    /// tab, which would read the reports as typed text.
    pub fn sync_focus(&mut self, focused: bool) {
        if self.target.is_ssh()
            || self.shell.is_some()
            || !self.focus_ready
            || self.focus_sent == Some(focused)
        {
            return;
        }
        self.focus_sent = Some(focused);
        let report: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
        self.send(report.to_vec());
    }

    /// Feed child output into the screen, answer its terminal queries, and
    /// update its activity: a gaviero session's from its title, a shell
    /// tab's from its shell integration (taken out of the output first).
    pub fn feed(&mut self, bytes: &[u8]) {
        match &mut self.shell {
            Some(watch) => {
                let clean = watch.filter(bytes, Instant::now());
                self.parser.process(&clean);
            }
            None => self.parser.process(bytes),
        }
        let replies = encode::query_replies(bytes, self.parser.screen().cursor_position());
        if !replies.is_empty() {
            self.send(replies);
        }
        if self.shell.is_some() {
            self.refresh_shell_activity();
        } else {
            let title = self.parser.callbacks().title.as_deref();
            self.activity = parse_activity(title);
            self.work = parse_work(title);
            self.unsaved = parse_unsaved(title);
        }
        if let Some(data) = self.parser.callbacks_mut().copied.take()
            && let Some(text) = copied_text(&data)
        {
            super::clipboard::set_text(&text);
        }
    }

    /// A shell tab's activity and work, now: its "waits for the user" guess
    /// changes with time alone, so the host also calls this between outputs.
    /// The guess reads the live screen, not the history the user may have
    /// scrolled to.
    pub fn refresh_shell_activity(&mut self) {
        let Some(watch) = &self.shell else {
            return;
        };
        let screen = self.parser.screen_mut();
        let scrolled = screen.scrollback();
        screen.set_scrollback(0);
        self.activity = watch.activity(screen, Instant::now());
        screen.set_scrollback(scrolled);
        self.work = watch.work();
    }

    /// The working directory a shell tab reported since the last call.
    pub fn take_shell_cwd(&mut self) -> Option<PathBuf> {
        self.shell.as_mut()?.take_cwd()
    }

    /// The folder the child reported since the last call (OSC 7), if any.
    pub fn take_reported_path(&mut self) -> Option<String> {
        self.parser.callbacks_mut().reported_path.take()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(2);
        let cols = cols.max(10);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser.screen_mut().set_size(rows, cols);
        // The text moves under it: a selection would no longer mark it.
        self.selection.clear();
    }

    /// The child's exit code once it has exited; `None` while it runs.
    /// Never blocks: a pseudo console may report end-of-output before (or
    /// long after) its child is gone.
    pub fn exit_code(&mut self) -> Option<u32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            Ok(None) => None,
            // The handle is unusable: treat it as gone.
            Err(_) => Some(1),
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_parses_from_the_title() {
        let activity = |t| parse_activity(Some(t));
        let none = Activity::default();
        assert_eq!(activity("gaviero-session:idle"), none);
        assert_eq!(activity("gaviero-session:other"), none);
        assert_eq!(parse_activity(Some("vim")), none);
        assert_eq!(parse_activity(None), none);
        assert_eq!(
            activity("gaviero-session:busy:agent,terminal"),
            Activity { agent: true, shell: true, ..none }
        );
        assert_eq!(
            activity("gaviero-session:busy:swarm,unsaved"),
            Activity { agent: true, ..none }
        );
        assert_eq!(
            activity("gaviero-session:busy:terminal"),
            Activity { shell: true, ..none }
        );
        // A permission prompt or question while the agent runs.
        assert_eq!(
            activity("gaviero-session:attention:agent"),
            Activity { waiting: true, agent: true, ..none }
        );
        // Waiting with no agent running (a question with other detail).
        assert_eq!(
            activity("gaviero-session:attention:unsaved"),
            Activity { waiting: true, ..none }
        );
        // A pending turn review after the turn: review, not waiting.
        assert_eq!(
            activity("gaviero-session:idle:review"),
            Activity { review: true, ..none }
        );
        // A review pending while another turn runs keeps both facts.
        assert_eq!(
            activity("gaviero-session:busy:agent,review"),
            Activity { agent: true, review: true, ..none }
        );
        // An older child: busy, no detail.
        assert_eq!(
            activity("gaviero-session:busy"),
            Activity { agent: true, ..none }
        );
        // Unsaved files alone are no activity.
        assert_eq!(activity("gaviero-session:idle:unsaved"), none);
        // A shell command waiting for the user.
        assert_eq!(
            activity("gaviero-session:busy:terminal,shell-input"),
            Activity { shell: true, shell_waiting: true, ..none }
        );
        // `shell-input` without a running command means nothing.
        assert_eq!(activity("gaviero-session:idle:shell-input"), none);
    }

    #[test]
    fn work_that_ends_is_reported_finished() {
        let both = Activity { agent: true, shell: true, ..Activity::default() };
        let none = Activity::default();
        assert_eq!(finished(both, none), (true, true));
        assert_eq!(finished(none, both), (false, false));
        assert_eq!(finished(both, both), (false, false));
        let shell_only = Activity { agent: false, ..both };
        assert_eq!(finished(both, shell_only), (true, false));
        // The turn ends into a turn review: the agent finished.
        let turn_review = Activity { review: true, ..none };
        assert_eq!(finished(both, turn_review), (true, true));
    }

    #[test]
    fn work_in_progress_parses_from_the_title() {
        assert_eq!(
            parse_work(Some("gaviero-session:busy:agent,terminal")),
            vec!["an agent is running", "a shell is running a command"]
        );
        assert_eq!(
            parse_work(Some("gaviero-session:idle:unsaved")),
            vec!["files are not saved"]
        );
        assert_eq!(
            parse_work(Some("gaviero-session:attention:swarm")),
            vec!["a swarm is running"]
        );
        assert!(parse_work(Some("gaviero-session:idle")).is_empty());
        // An older child: busy, no detail.
        assert_eq!(
            parse_work(Some("gaviero-session:busy")),
            vec!["a task is running"]
        );
        assert!(parse_work(Some("vim")).is_empty());
        assert!(parse_work(None).is_empty());
    }

    #[test]
    fn unsaved_files_parse_from_the_title() {
        assert!(parse_unsaved(Some("gaviero-session:idle:unsaved")));
        assert!(parse_unsaved(Some("gaviero-session:busy:agent,unsaved,review")));
        assert!(parse_unsaved(Some("gaviero-session:attention:unsaved")));
        assert!(!parse_unsaved(Some("gaviero-session:busy:agent")));
        assert!(!parse_unsaved(Some("gaviero-session:idle")));
        // Only the detail counts, and only the whole keyword.
        assert!(!parse_unsaved(Some("gaviero-session:unsaved")));
        assert!(!parse_unsaved(Some("gaviero-session:idle:unsaved-ish")));
        assert!(!parse_unsaved(Some("vim unsaved")));
        assert!(!parse_unsaved(None));
    }

    #[test]
    fn labels_use_the_folder_name_and_workspace_stem() {
        let folder = Target::Local(Path::new("C:\\work").join("alpha"));
        assert_eq!(label_for(&folder, &[]), "alpha");
        let ws = Target::Local(Path::new("C:\\work").join("team.gaviero-workspace"));
        assert_eq!(label_for(&ws, &[]), "team");
    }

    #[test]
    fn shell_labels_carry_the_shell_mark() {
        let shell = Target::Shell(Path::new("C:\\work").join("alpha"));
        assert_eq!(label_for(&shell, &[]), ">_ alpha");
        // A gaviero tab on the same folder is no clash.
        assert_eq!(label_for(&shell, &["alpha".to_string()]), ">_ alpha");
        assert_eq!(label_for(&shell, &[">_ alpha".to_string()]), ">_ alpha (2)");
    }

    #[test]
    fn ssh_labels_name_the_host_and_folder() {
        let ssh = |path: Option<&str>| Target::Ssh {
            host: "vps".into(),
            path: path.map(str::to_string),
        };
        assert_eq!(label_for(&ssh(None), &[]), "vps:~");
        assert_eq!(label_for(&ssh(Some("~")), &[]), "vps:~");
        assert_eq!(label_for(&ssh(Some("/srv/my proj")), &[]), "vps:my proj");
        assert_eq!(label_for(&ssh(Some("/srv/proj/")), &[]), "vps:proj");
        assert_eq!(label_for(&ssh(Some("~/code")), &[]), "vps:code");
        let taken = vec!["vps:proj".to_string()];
        assert_eq!(label_for(&ssh(Some("/srv/proj")), &taken), "vps:proj (2)");
    }

    #[test]
    fn osc52_copies_decode_to_text() {
        assert_eq!(copied_text(b"cHJvYmUtY29weQ==").as_deref(), Some("probe-copy"));
        assert_eq!(copied_text(b"not base64!"), None);
        assert_eq!(copied_text(b"/w=="), None, "not UTF-8");
    }

    #[test]
    fn duplicate_labels_are_numbered() {
        let folder = Target::Local(Path::new("C:\\a").join("alpha"));
        let taken = vec!["alpha".to_string()];
        assert_eq!(label_for(&folder, &taken), "alpha (2)");
        let taken = vec!["alpha".to_string(), "alpha (2)".to_string()];
        assert_eq!(label_for(&folder, &taken), "alpha (3)");
    }
}
