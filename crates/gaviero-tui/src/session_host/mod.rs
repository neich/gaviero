//! Session host (`gaviero --sessions`): several gaviero sessions in one
//! terminal, each an unmodified `gaviero` child in a pseudo-terminal.

pub(crate) mod encode;
pub(crate) mod input_term;
pub(crate) mod picker;
#[cfg(windows)]
pub(crate) mod input_windows;
pub(crate) mod probe;
pub(crate) mod render;
pub(crate) mod restart;
pub(crate) mod session;
pub(crate) mod state;

use anyhow::Context;
use crossterm::event::{
    EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use encode::MouseTranslator;
use session::Session;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// Set on every session child: the child knows it runs under a host (status
/// title, quit-all control file), and a nested `--sessions` refuses to start.
pub(crate) const HOST_ENV: &str = "GAVIERO_SESSION_HOST";

/// Set on every session child: a file the host writes `quit` into to ask
/// the child to quit through its normal path (quit all).
pub(crate) const CONTROL_ENV: &str = "GAVIERO_SESSION_CONTROL";

/// Set on every session child of a supervised host: the host restarts the
/// child onto a new build (with every other tab, once all are idle), so the
/// child never acts on a new build itself (`app/restart.rs`).
pub(crate) const HOST_RESTARTS_ENV: &str = "GAVIERO_SESSION_HOST_RESTARTS";

/// Everything the host loop reacts to, on one channel.
#[derive(Debug)]
pub(crate) enum HostEvent {
    /// Bytes a session's child wrote.
    Output { id: u64, bytes: Vec<u8> },
    /// A session's output ended. `code` is filled in from the child itself.
    Exited { id: u64, code: Option<u32> },
    /// A raw console key record (Windows).
    #[cfg(windows)]
    Key(encode::KeyRecord),
    /// A raw console mouse record (Windows).
    #[cfg(windows)]
    Mouse(encode::MouseRecord),
    /// The host terminal changed size.
    Resize,
    /// A crossterm event (Unix input path).
    #[cfg_attr(windows, allow(dead_code))]
    Term(crossterm::event::Event),
    /// Restart a session that asked for it too soon after starting.
    Relaunch { id: u64 },
}

/// What `gaviero --sessions` was asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostLaunch {
    /// No path: reopen the sessions open when the host last ended.
    Reopen,
    /// Open only this path.
    Path(PathBuf),
}

/// `None` when `args` (without the program name) do not ask for a session
/// host; otherwise what to open, or why the arguments are invalid.
pub(crate) fn sessions_requested(args: &[OsString]) -> Option<Result<HostLaunch, String>> {
    if !args.iter().any(|a| a == "--sessions") {
        return None;
    }
    let mut paths = Vec::new();
    for arg in args.iter().filter(|a| *a != "--sessions") {
        if arg.to_string_lossy().starts_with('-') {
            return Some(Err(format!(
                "--sessions takes only an optional PATH, not {}",
                arg.to_string_lossy()
            )));
        }
        paths.push(PathBuf::from(arg));
    }
    Some(match paths.len() {
        0 => Ok(HostLaunch::Reopen),
        1 => Ok(HostLaunch::Path(paths.remove(0))),
        _ => Err("--sessions takes at most one PATH".to_string()),
    })
}

/// A key the host keeps for itself instead of forwarding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chord {
    /// Alt+Left: previous session.
    Prev,
    /// Alt+Right: next session.
    Next,
    /// Alt+N: open the new-session picker.
    New,
    /// Alt+Q: close the active tab and end its session (asks first).
    Close,
    /// Ctrl+Q: quit the host, keeping every tab for the next start (asks
    /// first). A session never sees Ctrl+Q, so it never quits on its own.
    QuitAll,
}

const VK_Q: u16 = 0x51;

/// The host chord a Windows key record is, if any. Key-down only. Ctrl+Q
/// is Ctrl without Alt and the rest Alt without Ctrl, so AltGr
/// (`RIGHT_ALT | LEFT_CTRL`) chords stay text. Alt+X is never a chord:
/// Windows keeps it for itself.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn host_chord(r: &encode::KeyRecord) -> Option<Chord> {
    if !r.key_down {
        return None;
    }
    if encode::ctrl_only(r.ctrl_state) {
        return (r.vk == VK_Q).then_some(Chord::QuitAll);
    }
    if !encode::alt_only(r.ctrl_state) {
        return None;
    }
    let shift = r.ctrl_state & encode::SHIFT != 0;
    match r.vk {
        0x25 => Some(Chord::Prev),
        0x27 => Some(Chord::Next),
        0x4E if !shift => Some(Chord::New),
        VK_Q => Some(Chord::Close),
        _ => None,
    }
}

/// The host chord a crossterm key event is, if any (Unix input path).
fn term_chord(k: &KeyEvent) -> Option<Chord> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let q = matches!(k.code, KeyCode::Char('q') | KeyCode::Char('Q'));
    if ctrl && !alt {
        return q.then_some(Chord::QuitAll);
    }
    if !alt || ctrl {
        return None;
    }
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    match k.code {
        KeyCode::Left => Some(Chord::Prev),
        KeyCode::Right => Some(Chord::Next),
        KeyCode::Char('n') if !shift => Some(Chord::New),
        _ if q => Some(Chord::Close),
        _ => None,
    }
}

/// A question the host asks before it ends work in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Confirm {
    /// Close the tab of session `id` (Alt+Q or its close button).
    Close { id: u64 },
    /// Quit the host (Ctrl+Q).
    Quit,
}

/// Virtual-key codes of modifier keys: never an answer to a host question.
#[cfg(windows)]
fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5)
}

const VK_Y: u16 = 0x59;

/// Renders are coalesced to at most one per this interval.
const RENDER_EVERY: Duration = Duration::from_millis(16);
/// How often every child is checked for having exited. A pseudo console
/// does not always end its output when its child exits.
const EXIT_POLL_EVERY: Duration = Duration::from_millis(250);
/// How long a session asked to exit gets before the host kills it.
const EXIT_GRACE: Duration = Duration::from_secs(3);
/// How long a bar message stays up.
const MESSAGE_FOR: Duration = Duration::from_secs(5);

pub(crate) struct Host {
    sessions: Vec<Session>,
    active: usize,
    next_id: u64,
    #[cfg_attr(not(windows), allow(dead_code))]
    mouse: MouseTranslator,
    #[cfg_attr(not(windows), allow(dead_code))]
    swallow_keyups: Vec<u16>,
    /// The close guard, while it asks.
    confirm: Option<Confirm>,
    /// The host is quitting: every session was asked to exit, and those
    /// still running at this deadline are killed.
    quit_deadline: Option<Instant>,
    /// Closed sessions still being ended in the background; the host waits
    /// for them before it exits, so a closing session can save its state.
    ending: Vec<std::thread::JoinHandle<()>>,
    message: Option<(String, Instant)>,
    rows: u16,
    cols: u16,
    state_dir: PathBuf,
    tx: UnboundedSender<HostEvent>,
    /// Something visible changed since the last render.
    dirty: bool,
    /// Recents and the open set, saved to `state_path` after every change.
    state: state::HostState,
    state_path: Option<PathBuf>,
    /// The new-session picker, while open (Alt+N).
    picker: Option<picker::Picker>,
    /// Where relative paths typed in the picker are resolved.
    cwd: PathBuf,
    /// Restart-on-new-build for the host and all its tabs.
    restart: restart::HostRestart,
}

impl Host {
    fn new(
        rows: u16,
        cols: u16,
        state_dir: PathBuf,
        tx: UnboundedSender<HostEvent>,
        state: state::HostState,
        state_path: Option<PathBuf>,
    ) -> Self {
        Host {
            state,
            state_path,
            sessions: Vec::new(),
            active: 0,
            next_id: 1,
            mouse: MouseTranslator::default(),
            swallow_keyups: Vec::new(),
            confirm: None,
            quit_deadline: None,
            ending: Vec::new(),
            message: None,
            rows,
            cols,
            state_dir,
            tx,
            dirty: true,
            picker: None,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            restart: restart::HostRestart::new(),
        }
    }

    /// A question or the picker owns the keyboard.
    fn modal_open(&self) -> bool {
        self.confirm.is_some() || self.picker.is_some()
    }

    fn open_picker(&mut self) {
        self.picker = Some(picker::Picker::new(self.state.recent.clone(), &self.cwd));
        self.dirty = true;
    }

    /// Apply a key to the open picker; Enter on a valid path opens it.
    fn picker_key(&mut self, key: picker::PickerKey) {
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        self.dirty = true;
        match p.on_key(key, &self.cwd) {
            picker::PickerOutcome::Stay => {}
            picker::PickerOutcome::Close => self.picker = None,
            picker::PickerOutcome::Open(path) => {
                self.picker = None;
                if let Err(e) = self.open(path.clone()) {
                    tracing::error!(path = %path.display(), "opening the session failed: {e:#}");
                    self.notify(format!("could not open {}: {e}", path.display()));
                }
            }
        }
    }

    /// Rows available to a session: everything under the tab bar.
    fn session_rows(&self) -> u16 {
        self.rows.saturating_sub(1)
    }

    /// Start a session for `path` in a new tab and make it active.
    fn open(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.open_with(path, None)
    }

    /// [`Host::open`], with the tab's launch-state file seeded from `launch`
    /// (what a restarting host recorded for it).
    fn open_with(&mut self, path: PathBuf, launch: Option<&str>) -> anyhow::Result<()> {
        let taken: Vec<String> = self.sessions.iter().map(|s| s.label.clone()).collect();
        let label = session::label_for(&path, &taken);
        let id = self.next_id;
        self.next_id += 1;
        if let Some(text) = launch {
            let file = session::launch_state_path(&self.state_dir, id);
            if let Err(e) = std::fs::write(&file, text) {
                tracing::warn!(id, "could not restore the launch choice: {e}");
            }
        }
        let s = session::spawn(
            id,
            &path,
            label,
            self.session_rows(),
            self.cols,
            &self.state_dir,
            self.tx.clone(),
            self.restart.supervised(),
        )?;
        tracing::info!(id, path = %path.display(), "session opened");
        self.sessions.push(s);
        self.switch_to(self.sessions.len() - 1);
        self.state.note_opened(&path);
        self.save_state();
        Ok(())
    }

    fn save_state(&self) {
        if let Some(path) = &self.state_path
            && let Err(e) = state::save(path, &self.state)
        {
            tracing::warn!("saving {}: {e}", path.display());
        }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    fn switch_to(&mut self, i: usize) {
        if let Some(s) = self.sessions.get_mut(i) {
            self.active = i;
            s.agent_done_unseen = false;
            s.shell_done_unseen = false;
            self.dirty = true;
        }
    }

    fn notify(&mut self, text: String) {
        self.message = Some((text, Instant::now()));
        self.dirty = true;
    }

    /// The message shown at the right of the tab bar.
    pub(crate) fn bar_notice(&self) -> Option<String> {
        self.message.as_ref().map(|(text, _)| text.clone())
    }

    /// The close guard's title and lines, while it asks.
    pub(crate) fn confirm_popup(&self) -> Option<(String, Vec<String>)> {
        match self.confirm? {
            Confirm::Close { id } => {
                let s = &self.sessions[self.index_of(id)?];
                let mut lines = vec![format!("{} has work in progress:", s.label)];
                lines.extend(s.work.iter().map(|w| format!("  • {w}")));
                lines.push(String::new());
                lines.push("Closing ends it now; that work is lost.".to_string());
                lines.push("y close · any other key cancel".to_string());
                Some((format!(" Close {}? ", s.label), lines))
            }
            Confirm::Quit => {
                let mut lines = vec!["Work in progress:".to_string()];
                lines.extend(
                    self.sessions
                        .iter()
                        .filter(|s| !s.work.is_empty())
                        .map(|s| format!("  • {}: {}", s.label, s.work.join(", "))),
                );
                lines.push(String::new());
                lines.push("Every tab reopens next start, but this work is lost.".to_string());
                lines.push("y quit · any other key cancel".to_string());
                Some((" Quit gaviero? ".to_string(), lines))
            }
        }
    }

    fn expire_message(&mut self) {
        if self
            .message
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= MESSAGE_FOR)
        {
            self.message = None;
            self.dirty = true;
        }
    }

    fn active_session(&self) -> Option<&Session> {
        self.sessions.get(self.active)
    }

    fn send_active(&self, bytes: Vec<u8>) {
        if let Some(s) = self.active_session() {
            s.send(bytes);
        }
    }

    fn handle(&mut self, event: HostEvent) {
        if is_input(&event) {
            self.restart.note_input();
        }
        match event {
            HostEvent::Output { id, bytes } => self.on_output(id, &bytes),
            HostEvent::Exited { id, code } => {
                tracing::debug!(id, ?code, "session output ended");
                self.poll_exits();
            }
            #[cfg(windows)]
            HostEvent::Key(r) => self.on_key(r),
            #[cfg(windows)]
            HostEvent::Mouse(m) => self.on_mouse(m),
            HostEvent::Resize => self.on_resize(),
            HostEvent::Term(e) => self.on_term(e),
            HostEvent::Relaunch { id } => {
                if let Some(i) = self.index_of(id)
                    && self.sessions[i].restart_pending
                {
                    self.relaunch(i);
                }
            }
        }
    }

    fn on_output(&mut self, id: u64, bytes: &[u8]) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        let active = i == self.active;
        let s = &mut self.sessions[i];
        let before = s.activity;
        s.feed(bytes);
        let changed = s.activity != before;
        if changed && !active {
            // Work that ends in a background tab is marked finished until
            // the user makes the tab current.
            let (agent, shell) = session::finished(before, s.activity);
            s.agent_done_unseen |= agent;
            s.shell_done_unseen |= shell;
        }
        if active || changed {
            self.dirty = true;
        }
    }

    /// Handle every session whose child has exited.
    fn poll_exits(&mut self) {
        for i in (0..self.sessions.len()).rev() {
            if self.sessions[i].restart_pending {
                continue;
            }
            if let Some(code) = self.sessions[i].exit_code() {
                self.on_exit(i, code);
            }
        }
    }

    fn on_exit(&mut self, i: usize, code: u32) {
        if code == crate::supervisor::RESTART_EXIT_CODE as u32 && !self.quitting() {
            let s = &mut self.sessions[i];
            if s.started.elapsed() < crate::supervisor::MIN_RUN {
                // Same back-off as the supervisor: a build that asks for a
                // restart right away must not spin the tab.
                s.restart_pending = true;
                let (id, tx) = (s.id, self.tx.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(crate::supervisor::RESTART_BACKOFF).await;
                    let _ = tx.send(HostEvent::Relaunch { id });
                });
            } else {
                self.relaunch(i);
            }
            return;
        }
        self.close_tab(i, Some(code));
    }

    /// Start the session at `i` again, in the same tab with the same id.
    fn relaunch(&mut self, i: usize) {
        let (id, path, label) = {
            let s = &self.sessions[i];
            (s.id, s.path.clone(), s.label.clone())
        };
        match session::spawn(
            id,
            &path,
            label.clone(),
            self.session_rows(),
            self.cols,
            &self.state_dir,
            self.tx.clone(),
            self.restart.supervised(),
        ) {
            Ok(fresh) => {
                tracing::info!(id, "session restarted onto a new build");
                let old = std::mem::replace(&mut self.sessions[i], fresh);
                retire(old);
                self.notify(format!("{label} restarted onto a new build"));
            }
            Err(e) => {
                tracing::error!(id, "restarting the session failed: {e:#}");
                self.close_tab(i, None);
                self.notify(format!("{label} could not restart: {e}"));
            }
        }
    }

    /// Remove the tab at `i` and return its session. The open set forgets
    /// it unless the host is quitting. When the current tab goes, the one
    /// before it becomes current.
    fn remove_tab(&mut self, i: usize) -> Session {
        let old = self.sessions.remove(i);
        // A guard asking about this tab has nothing left to ask.
        if self.confirm == Some(Confirm::Close { id: old.id }) {
            self.confirm = None;
        }
        // Launch answers are kept across restarts of a tab, not after it.
        let _ = std::fs::remove_file(&old.launch_state);
        self.state.note_tab_ended(&old.path, self.quitting());
        self.save_state();
        self.active = previous_active(self.active, i, self.sessions.len());
        let active = self.active;
        self.switch_to(active);
        self.dirty = true;
        old
    }

    /// The tab at `i` whose child has ended with `code`.
    fn close_tab(&mut self, i: usize, code: Option<u32>) {
        let old = self.remove_tab(i);
        tracing::info!(id = old.id, ?code, "session closed");
        if let Some(code) = code
            && !self.quitting()
        {
            self.notify(format!("{} exited (code {code})", old.label));
        }
        retire(old);
    }

    /// Alt+Q or the tab's close button: the tab goes now; its child is asked
    /// to exit (saving its session state, no question) and killed if it is
    /// still running after `EXIT_GRACE`.
    fn close_session(&mut self, i: usize) {
        if i >= self.sessions.len() {
            return;
        }
        let old = self.remove_tab(i);
        tracing::info!(id = old.id, "session closed by the user");
        if let Err(e) = std::fs::write(&old.control, "exit") {
            tracing::warn!(id = old.id, "could not ask the session to exit: {e}");
        }
        self.notify(format!("closed {}", old.label));
        self.ending.retain(|h| !h.is_finished());
        if let Some(handle) = end_session(old, EXIT_GRACE) {
            self.ending.push(handle);
        }
    }

    fn quitting(&self) -> bool {
        self.quit_deadline.is_some()
    }

    /// Restart-on-new-build: once a new build is installed and nothing is in
    /// the way, record the tabs and quit with the restart code (supervised),
    /// or say a restart by hand is needed (not supervised).
    fn check_new_build(&mut self) {
        if self.quitting() {
            return;
        }
        let tabs: Vec<restart::TabWork<'_>> = self
            .sessions
            .iter()
            .map(|s| restart::TabWork {
                label: &s.label,
                work: &s.work,
            })
            .collect();
        let modal_open = self.modal_open();
        let step = self
            .restart
            .check(Instant::now(), |since_input| {
                restart::blocker(&tabs, modal_open, since_input)
            });
        match step {
            restart::Step::Nothing => {}
            restart::Step::Notify(text) => self.notify(text),
            restart::Step::Restart => self.restart_host(),
        }
    }

    /// Record every tab for the next host, then quit like Ctrl+Q does.
    fn restart_host(&mut self) {
        let Some(path) = self.restart.record.clone() else {
            return;
        };
        let record = restart::RestartRecord {
            tabs: self
                .sessions
                .iter()
                .map(|s| restart::RestartTab {
                    path: s.path.clone(),
                    launch: std::fs::read_to_string(&s.launch_state).ok(),
                })
                .collect(),
            active: self.active,
        };
        if let Err(e) = restart::save_record(&path, &record) {
            tracing::error!("could not record the tabs for a restart: {e}");
            self.restart.give_up();
            self.notify(format!(
                "New gaviero build installed, but the tabs could not be saved for a restart ({e}); restart by hand"
            ));
            return;
        }
        tracing::info!(tabs = record.tabs.len(), "new build installed and every tab idle: restarting the host");
        self.restart.requested = true;
        self.quit_host();
    }

    /// The host quits: every tab is kept for the next start and every child
    /// is asked to exit; `finish_quit` kills those left at the deadline.
    fn quit_host(&mut self) {
        tracing::info!(sessions = self.sessions.len(), "quitting the host");
        self.quit_deadline = Some(Instant::now() + EXIT_GRACE);
        self.save_state();
        for s in &self.sessions {
            if let Err(e) = std::fs::write(&s.control, "exit") {
                tracing::warn!(id = s.id, "could not ask the session to exit: {e}");
            }
        }
    }

    /// While quitting: kill every session still running once the deadline
    /// passes. True when the host is done.
    fn finish_quit(&mut self) -> bool {
        let Some(deadline) = self.quit_deadline else {
            return false;
        };
        if self.sessions.is_empty() {
            return true;
        }
        if Instant::now() < deadline {
            return false;
        }
        for mut s in self.sessions.drain(..) {
            tracing::warn!(id = s.id, "session did not exit in time: killed");
            s.kill();
            retire(s);
        }
        true
    }

    fn run_chord(&mut self, chord: Chord) {
        let n = self.sessions.len();
        match chord {
            Chord::Prev if n > 1 => self.switch_to((self.active + n - 1) % n),
            Chord::Next if n > 1 => self.switch_to((self.active + 1) % n),
            Chord::Prev | Chord::Next => {}
            Chord::New if !self.quitting() => self.open_picker(),
            Chord::New => {}
            Chord::Close => self.request_close(self.active),
            Chord::QuitAll if !self.quitting() => self.request_quit(),
            Chord::QuitAll => {}
        }
    }

    /// A left click on the tab bar: the close button closes its tab,
    /// anywhere else on a tab switches to it.
    fn click_tab_bar(&mut self, col: u16) {
        if let Some(i) = self.close_at(col) {
            self.request_close(i);
        } else if let Some(i) = self.tab_at(col) {
            self.switch_to(i);
        }
    }

    /// Close the tab at `i`, asking first when its session has work in
    /// progress (an agent, swarm or shell command running, unsaved files).
    fn request_close(&mut self, i: usize) {
        let Some(s) = self.sessions.get(i) else {
            return;
        };
        if s.work.is_empty() {
            self.close_session(i);
        } else {
            self.confirm = Some(Confirm::Close { id: s.id });
            self.dirty = true;
        }
    }

    /// Quit the host, asking first when any session has work in progress.
    fn request_quit(&mut self) {
        if self.sessions.iter().all(|s| s.work.is_empty()) {
            self.quit_host();
        } else {
            self.confirm = Some(Confirm::Quit);
            self.dirty = true;
        }
    }

    /// The answer to the close guard.
    fn answer_confirm(&mut self, yes: bool) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        self.dirty = true;
        if !yes {
            return;
        }
        match confirm {
            Confirm::Close { id } => {
                if let Some(i) = self.index_of(id) {
                    self.close_session(i);
                }
            }
            Confirm::Quit => self.quit_host(),
        }
    }

    #[cfg(windows)]
    fn swallow_keyup(&mut self, vk: u16) {
        if !self.swallow_keyups.contains(&vk) {
            self.swallow_keyups.push(vk);
        }
    }

    #[cfg(windows)]
    fn on_key(&mut self, r: encode::KeyRecord) {
        if !r.key_down {
            if let Some(pos) = self.swallow_keyups.iter().position(|vk| *vk == r.vk) {
                self.swallow_keyups.remove(pos);
                return;
            }
            // Key-ups never leak out of a question or the picker.
            if self.modal_open() {
                return;
            }
        } else if self.confirm.is_some() {
            if !is_modifier_vk(r.vk) {
                self.swallow_keyup(r.vk);
                let plain = r.ctrl_state & 0x0F == 0;
                self.answer_confirm(r.vk == VK_Y && plain);
            }
            return;
        } else if self.picker.is_some() {
            if let Some(key) = picker::key_from_record(&r) {
                self.swallow_keyup(r.vk);
                self.picker_key(key);
            }
            return;
        } else if let Some(chord) = host_chord(&r) {
            self.swallow_keyup(r.vk);
            self.run_chord(chord);
            return;
        }
        self.send_active(encode::win32_key_seq(&r).into_bytes());
    }

    #[cfg(windows)]
    fn on_mouse(&mut self, m: encode::MouseRecord) {
        if self.modal_open() {
            return;
        }
        if m.row == 0 {
            let press = m.flags == 0 && m.buttons & 1 != 0;
            if press {
                self.click_tab_bar(m.col);
            }
            return;
        }
        for report in self.mouse.translate(&m, 1) {
            self.send_active(report.into_bytes());
        }
    }

    /// The tab under column `col` of the tab bar.
    fn tab_at(&self, col: u16) -> Option<usize> {
        let titles = render::tab_titles(self);
        let badges = render::tab_badges(self);
        render::tab_bar(self, &titles, &badges).tab_at_x(col, 0)
    }

    /// The tab whose close button is at column `col` of the tab bar.
    fn close_at(&self, col: u16) -> Option<usize> {
        let titles = render::tab_titles(self);
        let badges = render::tab_badges(self);
        render::tab_bar(self, &titles, &badges).close_at_x(col, 0)
    }

    fn on_resize(&mut self) {
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            self.rows = rows;
            self.cols = cols;
        }
        let (rows, cols) = (self.session_rows(), self.cols);
        for s in &mut self.sessions {
            s.resize(rows, cols);
        }
        self.dirty = true;
    }

    /// Unix input path: crossterm events, re-encoded as VT bytes.
    fn on_term(&mut self, event: Event) {
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_term_key(k),
            Event::Mouse(m) => self.on_term_mouse(m),
            Event::Paste(text) => {
                if let Some(s) = self.active_session() {
                    let bracketed = s.parser.screen().bracketed_paste();
                    s.send(crate::panels::terminal::paste_bytes(bracketed, &text));
                }
            }
            _ => {}
        }
    }

    fn on_term_key(&mut self, k: KeyEvent) {
        if self.confirm.is_some() {
            let yes = k.code == KeyCode::Char('y')
                && !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            self.answer_confirm(yes);
            return;
        }
        if self.picker.is_some() {
            if let Some(key) = picker::key_from_event(&k) {
                self.picker_key(key);
            }
            return;
        }
        if let Some(chord) = term_chord(&k) {
            self.run_chord(chord);
            return;
        }
        if let Some(s) = self.active_session() {
            let app_cursor = s.parser.screen().application_cursor();
            s.send(crate::panels::terminal::key_event_to_bytes(&k, app_cursor));
        }
    }

    fn on_term_mouse(&mut self, m: MouseEvent) {
        if self.modal_open() {
            return;
        }
        if m.row == 0 {
            if m.kind == MouseEventKind::Down(MouseButton::Left) {
                self.click_tab_bar(m.column);
            }
            return;
        }
        if let Some(report) = term_mouse_report(&m, 1) {
            self.send_active(report.into_bytes());
        }
    }
}

impl Drop for Host {
    /// A host that stops with sessions still open (an error) takes their
    /// children with it rather than leaving them without a terminal.
    fn drop(&mut self) {
        for s in &mut self.sessions {
            s.kill();
        }
    }
}

/// The SGR report for a crossterm mouse event, rows shifted up by
/// `row_offset`; `None` for bare motion and rows above the offset.
fn term_mouse_report(m: &MouseEvent, row_offset: u16) -> Option<String> {
    if m.row < row_offset {
        return None;
    }
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (code, press) = match m.kind {
        MouseEventKind::Down(b) => (button(b), true),
        MouseEventKind::Up(b) => (button(b), false),
        MouseEventKind::Drag(b) => (button(b) + 32, true),
        MouseEventKind::ScrollUp => (64, true),
        MouseEventKind::ScrollDown => (65, true),
        MouseEventKind::ScrollLeft => (66, true),
        MouseEventKind::ScrollRight => (67, true),
        MouseEventKind::Moved => return None,
    };
    let mut mods = 0;
    if m.modifiers.contains(KeyModifiers::SHIFT) {
        mods += 4;
    }
    if m.modifiers.contains(KeyModifiers::ALT) {
        mods += 8;
    }
    if m.modifiers.contains(KeyModifiers::CONTROL) {
        mods += 16;
    }
    Some(encode::sgr_mouse(
        code + mods,
        m.column,
        m.row - row_offset,
        press,
    ))
}

/// Keyboard and mouse events: the user is working in the host.
fn is_input(event: &HostEvent) -> bool {
    match event {
        #[cfg(windows)]
        HostEvent::Key(_) | HostEvent::Mouse(_) => true,
        HostEvent::Term(e) => matches!(e, Event::Key(_) | Event::Mouse(_) | Event::Paste(_)),
        _ => false,
    }
}

/// The current tab after removing tab `removed`, `len` tabs left: the one
/// before a removed current tab, otherwise the same session as before.
fn previous_active(active: usize, removed: usize, len: usize) -> usize {
    let active = if removed <= active {
        active.saturating_sub(1)
    } else {
        active
    };
    active.min(len.saturating_sub(1))
}

/// End a closed session off the host loop: give its child `grace` to exit
/// on its own (it was asked to), then kill it.
fn end_session(mut session: Session, grace: Duration) -> Option<std::thread::JoinHandle<()>> {
    let spawned = std::thread::Builder::new()
        .name("session-end".into())
        .spawn(move || {
            let deadline = Instant::now() + grace;
            while session.exit_code().is_none() {
                if Instant::now() >= deadline {
                    tracing::warn!(id = session.id, "closed session did not exit in time: killed");
                    session.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = std::fs::remove_file(&session.control);
            drop(session);
        });
    spawned
        .map_err(|e| tracing::warn!("could not end a session in the background: {e}"))
        .ok()
}

/// Wait (bounded) for sessions still being ended: one closed with the last
/// tab may still be saving its state.
fn wait_for_ending(ending: Vec<std::thread::JoinHandle<()>>) {
    let deadline = Instant::now() + EXIT_GRACE + Duration::from_secs(1);
    while ending.iter().any(|h| !h.is_finished()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Drop a finished session off the host loop: closing a pseudo console can
/// wait for its output to drain.
fn retire(session: Session) {
    let _ = std::fs::remove_file(&session.control);
    let spawned = std::thread::Builder::new()
        .name("session-retire".into())
        .spawn(move || drop(session));
    if let Err(e) = spawned {
        tracing::warn!("could not close a session in the background: {e}");
    }
}

/// Start the host's input reader for this platform.
fn spawn_input_reader(tx: UnboundedSender<HostEvent>) {
    #[cfg(windows)]
    input_windows::spawn_console_reader(tx);
    #[cfg(not(windows))]
    input_term::spawn_crossterm_reader(tx);
}

/// The paths the host opens at startup: the given path, or the sessions
/// still open when the host last ended (those that still exist), else the
/// current directory.
fn initial_paths(launch: &HostLaunch, saved: &state::HostState) -> Result<Vec<PathBuf>, String> {
    let cwd = || {
        std::env::current_dir()
            .map(|p| vec![p])
            .map_err(|e| format!("reading the current directory: {e}"))
    };
    match launch {
        HostLaunch::Path(p) => gaviero_core::util::fs::canonicalize_simplified(p)
            .map(|p| vec![p])
            .map_err(|e| format!("resolving path {}: {e}", p.display())),
        HostLaunch::Reopen => {
            let open: Vec<PathBuf> = saved
                .open
                .iter()
                .filter(|p| state::still_openable(p))
                .cloned()
                .collect();
            if open.is_empty() { cwd() } else { Ok(open) }
        }
    }
}

/// Log to `gaviero-sessions.log` beside `gaviero.log`; children keep theirs.
fn init_logging() {
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gaviero");
    let _ = std::fs::create_dir_all(&log_dir);
    let Ok(log_file) = std::fs::File::create(log_dir.join("gaviero-sessions.log")) else {
        return;
    };
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GAVIERO_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
        )
        .init();
}

/// Run the session host; returns the process exit code.
pub(crate) fn run(launch: HostLaunch) -> i32 {
    init_logging();
    let state_path = state::state_path();
    let mut saved = state_path.as_deref().map(state::load).unwrap_or_default();
    // After a restart onto a new build: the tabs the previous host had,
    // whatever this host was launched with.
    let restarted = crate::supervisor::host_restart_record()
        .and_then(|path| restart::take_record(&path));
    let initial = match restarted {
        Some(record) => record,
        None => match initial_paths(&launch, &saved) {
            Ok(paths) => restart::RestartRecord {
                tabs: paths
                    .into_iter()
                    .map(|path| restart::RestartTab { path, launch: None })
                    .collect(),
                active: usize::MAX,
            },
            Err(msg) => {
                eprintln!("gaviero: {msg}");
                return 2;
            }
        },
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("gaviero: starting the async runtime: {e}");
            return 1;
        }
    };
    let state_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gaviero")
        .join(format!("session-host-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&state_dir) {
        eprintln!("gaviero: creating {}: {e}", state_dir.display());
        return 1;
    }
    // The open set is rebuilt from the tabs this run actually opens.
    saved.open.clear();
    let result = runtime.block_on(host_loop(initial, state_dir.clone(), saved, state_path));
    // Sessions retired on background threads may still be closing.
    runtime.shutdown_timeout(Duration::from_secs(2));
    let _ = std::fs::remove_dir_all(&state_dir);
    match result {
        Ok(code) => code,
        Err(e) => {
            let _ = crate::restore_terminal();
            tracing::error!("session host failed: {e:#}");
            eprintln!("gaviero: {e:#}");
            1
        }
    }
}

/// Runs the host until it ends; the exit code (the restart code when it
/// ends to restart onto a new build). `initial.active` is the tab to make
/// current after a restart, `usize::MAX` on a normal start (the last tab
/// opened stays current).
async fn host_loop(
    initial: restart::RestartRecord,
    state_dir: PathBuf,
    saved: state::HostState,
    state_path: Option<PathBuf>,
) -> anyhow::Result<i32> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crate::restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("entering alternate screen")?;
    crate::platform::set_bracketed_paste(&mut stdout, true).context("enabling bracketed paste")?;
    crate::platform::enable_vt_mouse_passthrough(&mut stdout)
        .context("enabling VT mouse passthrough")?;
    // The sessions own the keyboard; Ctrl+C must never end the host.
    crate::platform::ignore_console_interrupts();
    let _guard = crate::TerminalGuard;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))
        .context("creating terminal")?;

    let (tx, mut rx) = unbounded_channel();
    spawn_input_reader(tx.clone());
    let (cols, rows) = crossterm::terminal::size().context("reading the terminal size")?;
    let mut host = Host::new(rows, cols, state_dir, tx, saved, state_path);
    let mut last_error = None;
    let restarted = initial.active != usize::MAX;
    for tab in initial.tabs {
        if let Err(e) = host.open_with(tab.path.clone(), tab.launch.as_deref()) {
            tracing::error!(path = %tab.path.display(), "opening the session failed: {e:#}");
            host.notify(format!("could not open {}: {e}", tab.path.display()));
            last_error = Some(e);
        }
    }
    if host.sessions.is_empty() {
        return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no session to open")));
    }
    if restarted {
        host.switch_to(initial.active.min(host.sessions.len() - 1));
        if last_error.is_none() {
            host.notify("restarted onto a new build".to_string());
        }
    }

    let mut tick = tokio::time::interval(RENDER_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_render: Option<Instant> = None;
    let mut last_poll = Instant::now();
    loop {
        if host.dirty && last_render.is_none_or(|at| at.elapsed() >= RENDER_EVERY) {
            terminal
                .draw(|frame| render::draw(&host, frame))
                .context("drawing")?;
            host.dirty = false;
            last_render = Some(Instant::now());
        }
        tokio::select! {
            event = rx.recv() => {
                if let Some(event) = event {
                    host.handle(event);
                    for _ in 0..64 {
                        match rx.try_recv() {
                            Ok(event) => host.handle(event),
                            Err(_) => break,
                        }
                    }
                }
            }
            _ = tick.tick() => {}
        }
        if last_poll.elapsed() >= EXIT_POLL_EVERY {
            host.poll_exits();
            host.expire_message();
            host.check_new_build();
            last_poll = Instant::now();
        }
        if host.finish_quit() || host.sessions.is_empty() {
            break;
        }
    }
    drop(terminal);
    crate::restore_terminal().context("restoring terminal")?;
    wait_for_ending(std::mem::take(&mut host.ending));
    Ok(if host.restart.requested {
        crate::supervisor::RESTART_EXIT_CODE
    } else {
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn sessions_alone_reopens() {
        assert_eq!(
            sessions_requested(&args(&["--sessions"])),
            Some(Ok(HostLaunch::Reopen))
        );
    }

    #[test]
    fn sessions_with_a_path_on_either_side() {
        let want = Some(Ok(HostLaunch::Path(PathBuf::from("C:\\x"))));
        assert_eq!(sessions_requested(&args(&["--sessions", "C:\\x"])), want);
        assert_eq!(sessions_requested(&args(&["C:\\x", "--sessions"])), want);
    }

    #[test]
    fn sessions_rejects_other_flags_and_extra_paths() {
        assert!(matches!(
            sessions_requested(&args(&["--sessions", "--folder"])),
            Some(Err(_))
        ));
        assert!(matches!(
            sessions_requested(&args(&["--sessions", "a", "b"])),
            Some(Err(_))
        ));
    }

    fn down(vk: u16, uchar: u16, ctrl_state: u32) -> encode::KeyRecord {
        encode::KeyRecord {
            vk,
            scan: 0,
            uchar,
            key_down: true,
            ctrl_state,
            repeat: 1,
        }
    }

    #[test]
    fn host_chords_are_alt_keys_and_ctrl_q() {
        use encode::{LEFT_ALT, LEFT_CTRL, RIGHT_ALT, RIGHT_CTRL, SHIFT};
        assert_eq!(host_chord(&down(0x25, 0, LEFT_ALT)), Some(Chord::Prev));
        assert_eq!(host_chord(&down(0x27, 0, RIGHT_ALT)), Some(Chord::Next));
        assert_eq!(host_chord(&down(0x4E, 'n' as u16, LEFT_ALT)), Some(Chord::New));
        assert_eq!(host_chord(&down(0x51, 'q' as u16, LEFT_ALT)), Some(Chord::Close));
        assert_eq!(
            host_chord(&down(0x51, 'Q' as u16, LEFT_ALT | SHIFT)),
            Some(Chord::Close)
        );
        assert_eq!(host_chord(&down(0x51, 17, LEFT_CTRL)), Some(Chord::QuitAll));
        assert_eq!(host_chord(&down(0x51, 17, RIGHT_CTRL)), Some(Chord::QuitAll));
        // Alt+X belongs to Windows; other Ctrl keys belong to the session.
        assert_eq!(host_chord(&down(0x58, 'x' as u16, LEFT_ALT)), None);
        assert_eq!(host_chord(&down(0x53, 19, LEFT_CTRL)), None);
        // Key-up records never trigger a chord.
        let mut up = down(0x25, 0, LEFT_ALT);
        up.key_down = false;
        assert_eq!(host_chord(&up), None);
        let mut up = down(0x51, 17, LEFT_CTRL);
        up.key_down = false;
        assert_eq!(host_chord(&up), None);
    }

    #[test]
    fn altgr_and_plain_keys_are_not_chords() {
        use encode::{LEFT_CTRL, RIGHT_ALT};
        assert_eq!(host_chord(&down(0x4E, 'n' as u16, RIGHT_ALT | LEFT_CTRL)), None);
        // AltGr+Q is '@' on German layouts: text, not quit.
        assert_eq!(host_chord(&down(0x51, '@' as u16, RIGHT_ALT | LEFT_CTRL)), None);
        assert_eq!(host_chord(&down(0x4E, 'n' as u16, 0)), None);
        assert_eq!(host_chord(&down(0x51, 'q' as u16, 0)), None);
        assert_eq!(host_chord(&down(0x25, 0, 0)), None);
    }

    #[test]
    fn crossterm_chords_and_mouse_reports() {
        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        assert_eq!(term_chord(&alt(KeyCode::Left)), Some(Chord::Prev));
        assert_eq!(term_chord(&alt(KeyCode::Right)), Some(Chord::Next));
        assert_eq!(term_chord(&alt(KeyCode::Char('n'))), Some(Chord::New));
        assert_eq!(term_chord(&alt(KeyCode::Char('Q'))), Some(Chord::Close));
        assert_eq!(term_chord(&alt(KeyCode::Char('q'))), Some(Chord::Close));
        assert_eq!(term_chord(&alt(KeyCode::Char('x'))), None);
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
        assert_eq!(term_chord(&ctrl(KeyCode::Char('q'))), Some(Chord::QuitAll));
        assert_eq!(term_chord(&ctrl(KeyCode::Char('s'))), None);
        assert_eq!(
            term_chord(&KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            None
        );
        assert_eq!(
            term_chord(&KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
            None
        );

        let mouse = |kind, row| MouseEvent {
            kind,
            column: 9,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            term_mouse_report(&mouse(MouseEventKind::Down(MouseButton::Left), 5), 1),
            Some("\x1b[<0;10;5M".to_string())
        );
        assert_eq!(
            term_mouse_report(&mouse(MouseEventKind::ScrollDown, 5), 1),
            Some("\x1b[<65;10;5M".to_string())
        );
        assert_eq!(
            term_mouse_report(&mouse(MouseEventKind::Down(MouseButton::Left), 0), 1),
            None
        );
        assert_eq!(term_mouse_report(&mouse(MouseEventKind::Moved, 5), 1), None);
    }

    #[test]
    fn closing_the_current_tab_selects_the_one_before_it() {
        // Tabs 0..=3, current 2, close 2: tab 1 is current.
        assert_eq!(previous_active(2, 2, 3), 1);
        // Closing the first tab while it is current: the new first one.
        assert_eq!(previous_active(0, 0, 2), 0);
        // Closing a tab before the current one keeps the same session.
        assert_eq!(previous_active(2, 0, 3), 1);
        // Closing a tab after the current one changes nothing.
        assert_eq!(previous_active(1, 3, 3), 1);
        // The last tab gone.
        assert_eq!(previous_active(0, 0, 0), 0);
    }

    #[test]
    fn no_sessions_flag_is_not_a_host() {
        assert_eq!(sessions_requested(&args(&["a"])), None);
        assert_eq!(sessions_requested(&args(&[])), None);
    }
}
