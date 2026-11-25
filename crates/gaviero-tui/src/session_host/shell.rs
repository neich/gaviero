//! Shell tabs: the user's own shell (pwsh 7.2+ on Windows, `$SHELL`
//! elsewhere — the embedded terminal's `ShellConfig::default_for_user`) in a
//! pseudo-terminal, opened with Alt+T or the picker's Shell mode.
//!
//! The shell is started with gaviero's shell integration (core
//! `shell_integration`, keeping the user's own history file), so it reports
//! when a command runs (OSC 133 `C` … `D`) and which folder it is in
//! (OSC 7). [`ShellWatch`] takes those sequences out of the output before the
//! screen sees them and turns them into the tab's `S` badge: green while a
//! command runs, yellow while it seems to wait for the user (a bell or a
//! notification, or — after `INPUT_WAIT_QUIET` with no output and no input —
//! a full-screen program or a prompt left on its line, the same guess the
//! embedded terminal makes), blue once it finished out of view. A shell
//! without integration (an unknown `$SHELL`) runs, but never shows a badge.

use super::session::Activity;
use gaviero_core::terminal::config::ShellConfig;
use gaviero_core::terminal::instance::{INPUT_WAIT_QUIET, screen_awaits_input};
use gaviero_core::terminal::osc::{Osc133Marker, OscParser, OscResult};
use gaviero_core::terminal::shell_integration;
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// How a shell tab's label starts, so it never reads as a gaviero tab.
pub(crate) const SHELL_MARK: &str = ">_";

/// What closing a shell tab with a command running cuts off (close guard).
const RUNNING_WORK: &str = "a shell is running a command";

/// Lines one wheel notch scrolls a shell tab's history.
pub(crate) const WHEEL_LINES: usize = 3;

/// Lines a shell tab keeps above its screen: the embedded terminal's.
pub(crate) fn scrollback_lines() -> usize {
    gaviero_core::terminal::TerminalConfig::default().scrollback_lines as usize
}

/// Variables that make a gaviero a supervised child or a host's tab. A
/// shell tab inherits none, even when the host itself was started from a
/// shell that had them: a gaviero started in the shell supervises itself.
const NOT_INHERITED: [&str; 6] = [
    super::HOST_ENV,
    super::CONTROL_ENV,
    super::HOST_RESTARTS_ENV,
    crate::supervisor::HOST_RESTART_ENV,
    crate::supervisor::CHILD_ENV,
    crate::supervisor::LAUNCH_STATE_ENV,
];

/// The user's shell, with shell integration, started in `dir`. None of the
/// session host's own variables reach it ([`NOT_INHERITED`]): a gaviero
/// started from this shell is a session of its own, not a tab of this host.
pub(crate) fn command(dir: &Path) -> anyhow::Result<CommandBuilder> {
    let mut config = ShellConfig::default_for_user();
    #[cfg(windows)]
    if config.shell_type == gaviero_core::terminal::ShellType::PowerShell {
        gaviero_core::terminal::config::ensure_pwsh_version(&config.shell_path)?;
    }
    if config.enable_integration {
        match shell_integration::create_init_file_with_own_history(&config.shell_type, "session-host") {
            Ok(init) => shell_integration::build_shell_args(&mut config, &init),
            Err(e) => tracing::warn!(
                shell = config.shell_type.name(),
                "shell tab without integration (no activity badge): {e:#}"
            ),
        }
    }
    let mut cmd = gaviero_core::terminal::pty::build_command(&config, dir);
    for key in NOT_INHERITED {
        cmd.env_remove(key);
    }
    Ok(cmd)
}

/// A shell tab's command state, read from its shell integration.
pub(crate) struct ShellWatch {
    osc: OscParser,
    /// Between OSC 133 `C` (the user ran a command) and `D` / `A` (the
    /// prompt is back).
    running: bool,
    /// The running command rang the bell or asked for a notification;
    /// cleared by the user's next input or the command's end.
    attention: bool,
    last_output: Instant,
    last_input: Instant,
    /// The last OSC 7 report, to skip the unchanged one every prompt sends.
    last_report: Option<String>,
    /// A working directory reported since the host last took it.
    cwd: Option<PathBuf>,
}

impl ShellWatch {
    pub fn new(now: Instant) -> Self {
        ShellWatch {
            osc: OscParser::new(),
            running: false,
            attention: false,
            last_output: now,
            last_input: now,
            last_report: None,
            cwd: None,
        }
    }

    /// Take the shell-integration sequences (OSC 133, OSC 7, titles,
    /// notifications) out of `bytes` and note what they say; the rest is
    /// what the screen gets. A sequence split across reads is kept until it
    /// ends.
    pub fn filter(&mut self, bytes: &[u8], now: Instant) -> Vec<u8> {
        let (results, clean) = self.osc.feed(bytes);
        self.last_output = now;
        for result in results {
            self.apply(result);
        }
        clean
    }

    fn apply(&mut self, result: OscResult) {
        match result {
            OscResult::Osc133(Osc133Marker::CommandOutputStart) => {
                self.running = true;
                self.attention = false;
            }
            // `A` too: a prompt drawn means no command is running, even if
            // its `D` was lost.
            OscResult::Osc133(
                Osc133Marker::CommandFinished { .. } | Osc133Marker::PromptStart,
            ) => {
                self.running = false;
                self.attention = false;
            }
            OscResult::Osc133(Osc133Marker::CommandInputStart) => {}
            OscResult::Osc7(url) => {
                if self.last_report.as_deref() != Some(url.as_str()) {
                    if let Some(dir) = cwd_from_url(&url) {
                        self.cwd = Some(dir);
                    }
                    self.last_report = Some(url);
                }
            }
            // A completion beep at the prompt is not a request.
            OscResult::Bell | OscResult::Notify => self.attention |= self.running,
            OscResult::Title(_) => {}
        }
    }

    /// The user typed, pasted or clicked in the tab: whatever the command
    /// waited for has been answered.
    pub fn note_input(&mut self, now: Instant) {
        self.last_input = now;
        self.attention = false;
    }

    /// The working directory the shell reported since the last call.
    pub fn take_cwd(&mut self) -> Option<PathBuf> {
        self.cwd.take()
    }

    /// The tab's activity at `now`, from `screen` scrolled to the bottom.
    pub fn activity(&self, screen: &vt100::Screen, now: Instant) -> Activity {
        let quiet = now.saturating_duration_since(self.last_output.max(self.last_input));
        let waiting = self.running
            && (self.attention || (quiet >= INPUT_WAIT_QUIET && screen_awaits_input(screen)));
        Activity {
            shell: self.running,
            shell_waiting: waiting,
            ..Activity::default()
        }
    }

    /// What closing the tab now would cut off.
    pub fn work(&self) -> Vec<&'static str> {
        if self.running { vec![RUNNING_WORK] } else { Vec::new() }
    }
}

/// The folder in an OSC 7 report. pwsh reports `file://HOST/C:/dir`, so the
/// URL's slash before a drive letter goes; the path is then resolved, so
/// `C:/dir` and `C:\dir` are one target. `None` unless it is a folder here.
fn cwd_from_url(url: &str) -> Option<PathBuf> {
    let path = super::osc7::decode_file_url(url.as_bytes())?;
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        &path[1..]
    } else {
        &path[..]
    };
    let dir = gaviero_core::util::fs::canonicalize_simplified(Path::new(path)).ok()?;
    dir.is_dir().then_some(dir)
}

/// What a mouse event over a shell tab's screen does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellMouse {
    /// The program asked for mouse reports: forward it.
    Forward,
    /// Wheel over the shell's own screen: scroll its history.
    Scroll { up: bool },
    /// Wheel over a full-screen program that takes no mouse (a pager):
    /// these arrow keys, the way terminals' alternate scroll mode does.
    Keys(Vec<u8>),
    Drop,
}

/// Decide a mouse event over a shell tab. `wheel`: `Some(up)` for a
/// vertical wheel notch, `None` for anything else. A shell at its prompt
/// takes no mouse reports, so clicks there would arrive as typed garbage.
pub(crate) fn mouse(screen: &vt100::Screen, wheel: Option<bool>) -> ShellMouse {
    if !matches!(screen.mouse_protocol_mode(), vt100::MouseProtocolMode::None) {
        return ShellMouse::Forward;
    }
    let Some(up) = wheel else {
        return ShellMouse::Drop;
    };
    if !screen.alternate_screen() {
        return ShellMouse::Scroll { up };
    }
    let arrow: &[u8] = match (up, screen.application_cursor()) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    ShellMouse::Keys(arrow.repeat(WHEEL_LINES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn screen(bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(5, 40, 0);
        parser.process(bytes);
        parser
    }

    fn later(t: Instant) -> Instant {
        t + INPUT_WAIT_QUIET + Duration::from_secs(1)
    }

    #[test]
    fn integration_sequences_never_reach_the_screen() {
        let t = Instant::now();
        let mut w = ShellWatch::new(t);
        let clean = w.filter(b"\x1b]133;A\x07PS> \x1b]133;B\x07", t);
        assert_eq!(clean, b"PS> ");
        // Split across reads: kept until it ends.
        assert_eq!(w.filter(b"out\x1b]13", t), b"out");
        assert_eq!(w.filter(b"3;C\x07more", t), b"more");
        assert!(w.running);
    }

    #[test]
    fn a_command_runs_from_c_to_d() {
        let t = Instant::now();
        let mut w = ShellWatch::new(t);
        let idle = screen(b"PS> ");
        assert_eq!(w.activity(idle.screen(), t), Activity::default());
        assert!(w.work().is_empty());
        w.filter(b"\x1b]133;C\x07", t);
        let running = Activity { shell: true, ..Activity::default() };
        assert_eq!(w.activity(idle.screen(), t), running);
        assert_eq!(w.work(), vec![RUNNING_WORK]);
        w.filter(b"\x1b]133;D;0\x07\x1b]133;A\x07", t);
        assert_eq!(w.activity(idle.screen(), t), Activity::default());
        // A prompt alone also ends it (a lost `D`).
        w.filter(b"\x1b]133;C\x07", t);
        w.filter(b"\x1b]133;A\x07", t);
        assert!(!w.running);
    }

    #[test]
    fn a_bell_while_running_waits_until_the_user_types() {
        let t = Instant::now();
        let mut w = ShellWatch::new(t);
        let s = screen(b"working\r\n");
        // At the prompt a bell is a completion beep.
        w.filter(b"\x07", t);
        assert!(!w.activity(s.screen(), t).shell_waiting);
        w.filter(b"\x1b]133;C\x07", t);
        w.filter(b"\x1b]9;done?\x07", t);
        assert!(w.activity(s.screen(), t).shell_waiting, "at once, no quiet needed");
        w.note_input(t);
        assert!(!w.activity(s.screen(), t).shell_waiting);
    }

    #[test]
    fn a_quiet_prompt_line_waits_and_a_quiet_newline_does_not() {
        let t = Instant::now();
        let mut w = ShellWatch::new(t);
        w.filter(b"\x1b]133;C\x07", t);
        let asking = screen(b"Password: ");
        assert!(!w.activity(asking.screen(), t).shell_waiting, "not quiet yet");
        assert!(w.activity(asking.screen(), later(t)).shell_waiting);
        let silent = screen(b"building\r\n");
        assert!(!w.activity(silent.screen(), later(t)).shell_waiting);
        // Input restarts the quiet time.
        let typed = later(t);
        w.note_input(typed);
        assert!(!w.activity(asking.screen(), typed).shell_waiting);
    }

    #[test]
    fn the_working_directory_is_reported_once_per_change() {
        let dir = tempfile::tempdir().unwrap();
        let canon = gaviero_core::util::fs::canonicalize_simplified(dir.path()).unwrap();
        let forward = canon.to_string_lossy().replace('\\', "/");
        // A drive path gets the URL's leading slash, as pwsh sends it.
        let path = if forward.starts_with('/') { forward } else { format!("/{forward}") };
        let url = super::super::osc7::encode_file_url(&path);
        let report = format!("\x1b]7;{url}\x07");
        let t = Instant::now();
        let mut w = ShellWatch::new(t);
        w.filter(report.as_bytes(), t);
        assert_eq!(w.take_cwd(), Some(canon.clone()));
        assert_eq!(w.take_cwd(), None);
        // Every prompt repeats it: not a change.
        w.filter(report.as_bytes(), t);
        assert_eq!(w.take_cwd(), None);
        // A folder that is not here is ignored.
        w.filter(b"\x1b]7;file://h/definitely/not/here\x07", t);
        assert_eq!(w.take_cwd(), None);
    }

    #[test]
    fn pwsh_drive_paths_lose_the_urls_slash() {
        let dir = tempfile::tempdir().unwrap();
        let canon = gaviero_core::util::fs::canonicalize_simplified(dir.path()).unwrap();
        let forward = canon.to_string_lossy().replace('\\', "/");
        // pwsh: `file://HOST/` + `C:/…` (a Unix path already starts with /).
        let url = if forward.starts_with('/') {
            format!("file://HOST{forward}")
        } else {
            format!("file://HOST/{forward}")
        };
        assert_eq!(cwd_from_url(&url), Some(canon));
    }

    #[test]
    fn the_mouse_goes_to_programs_that_ask_and_scrolls_otherwise() {
        let prompt = screen(b"$ ");
        assert_eq!(mouse(prompt.screen(), None), ShellMouse::Drop);
        assert_eq!(mouse(prompt.screen(), Some(true)), ShellMouse::Scroll { up: true });
        assert_eq!(mouse(prompt.screen(), Some(false)), ShellMouse::Scroll { up: false });
        // A program with mouse reporting on gets everything.
        let tui = screen(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(mouse(tui.screen(), None), ShellMouse::Forward);
        assert_eq!(mouse(tui.screen(), Some(true)), ShellMouse::Forward);
        // A pager on the alternate screen without mouse: arrow keys.
        let pager = screen(b"\x1b[?1049h");
        assert_eq!(mouse(pager.screen(), Some(true)), ShellMouse::Keys(b"\x1b[A".repeat(WHEEL_LINES)));
        let app_cursor = screen(b"\x1b[?1049h\x1b[?1h");
        assert_eq!(mouse(app_cursor.screen(), Some(false)), ShellMouse::Keys(b"\x1bOB".repeat(WHEEL_LINES)));
        assert_eq!(mouse(pager.screen(), None), ShellMouse::Drop);
    }
}
