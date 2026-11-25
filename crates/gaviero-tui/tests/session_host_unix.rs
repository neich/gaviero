//! The Unix session host in a real pseudo-terminal, with an input probe as
//! its one tab. In a terminal that speaks the kitty keyboard protocol the
//! host asks for it, forwards each key as the legacy bytes a gaviero tab
//! reads (Shift+Enter as Alt+Enter, its chat newline), and gives the flags
//! back when it quits. A terminal without the protocol gets nothing pushed.
#![cfg(unix)]

#[allow(dead_code)]
mod common;

use common::{Answers, Pty, count};
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

/// Push of disambiguate (1) + report alternate keys (4).
const PUSH: &[u8] = b"\x1b[>5u";
/// crossterm's pop of one entry.
const POP: &[u8] = b"\x1b[<1u";
/// Leaving the alternate screen.
const LEAVE_ALT: &[u8] = b"\x1b[?1049l";

/// A host on `dir` in a terminal answering `answers`, its tab a probe.
fn host(root: &Path, dir: &Path, answers: Answers) -> Pty {
    let state = root.join("state.json");
    let pty = Pty::spawn_answering(
        &[OsStr::new("--sessions"), dir.as_os_str()],
        &[
            ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
            ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ],
        root,
        answers,
    );
    pty.wait_for("the tab", |_, _| pty.row(0).contains("alpha-a"));
    pty
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

#[test]
fn kitty_keys_reach_a_gaviero_tab_as_legacy_vt() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    std::fs::create_dir(&a).unwrap();
    let mut pty = host(root.path(), &a, Answers::Kitty);
    assert_eq!(count(&pty.raw(), PUSH), 1, "the host pushes its flags once");

    // What the keyboard sends under the protocol, and what the tab reads.
    let keys = [
        // Shift+Enter: the chat newline, sent as Alt+Enter.
        ("\x1b[13;2u", "KEY Enter KeyModifiers(ALT) Press"),
        // Ctrl+[ is Esc.
        ("\x1b[91;5u", "KEY Esc KeyModifiers(0x0) Press"),
        // Ctrl+Space is NUL.
        ("\x1b[32;5u", "KEY Char(' ') KeyModifiers(CONTROL) Press"),
        // Alt+Shift+1 with the shifted key reported: Alt+!.
        ("\x1b[49:33;4u", "KEY Char('!') KeyModifiers(ALT) Press"),
        // Alt+Shift+A without it: still ESC A.
        ("\x1b[97;4u", "KEY Char('A') KeyModifiers(SHIFT | ALT) Press"),
        // Ctrl+R, and plain Enter keeps its legacy byte.
        ("\x1b[114;5u", "KEY Char('r') KeyModifiers(CONTROL) Press"),
        ("\r", "KEY Enter KeyModifiers(0x0) Press"),
    ];
    for (sent, seen) in keys {
        pty.send(sent);
        pty.wait_text(seen);
    }

    // Ctrl+Q is the host's; on the way out it pops the flags, before it
    // leaves the alternate screen.
    pty.send("\x1b[113;5u");
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
    let raw = pty.raw();
    assert_eq!(count(&raw, POP), 1);
    assert!(rfind(&raw, POP) < rfind(&raw, LEAVE_ALT), "popped after leaving the alternate screen");
}

/// The host's "finished" badge colour (`session_host/render.rs`).
const DONE: vt100::Color = vt100::Color::Rgb(51, 136, 255);
/// The badges' letters, small capitals (`session_host/render.rs`).
const AGENT: &str = "\u{1D00}";
const SHELL: &str = "\u{A731}";

/// The two badges after `label` on the tab bar, each a 2-cell slot (blank,
/// letter on the badge colour): (letter, its cell's background) each; a
/// blank letter when unset.
fn circles(pty: &Pty, label: &str) -> [(String, vt100::Color); 2] {
    let bar = pty.row(0);
    let at = bar.find(label).expect("label on the tab bar");
    let col = (bar[..at].chars().count() + label.chars().count() + 1) as u16;
    let badge = |at: u16| {
        let (letter, bg) = pty.cell(0, at);
        (letter.trim().to_string(), bg)
    };
    [badge(col), badge(col + 2)]
}

/// Work that ended out of view marks the tab finished. Coming back, the
/// agent's mark stays until the tab is left, but the shell's clears at once:
/// the command's output is in view and its prompt back.
#[test]
fn a_finished_shell_mark_clears_when_its_tab_is_made_current() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    let b = root.path().join("beta-b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut pty = host(root.path(), &a, Answers::Legacy);
    // Alt+N, B's path, Enter: B opens as the current tab.
    pty.send("\x1bn");
    pty.wait_text(" New session ");
    pty.send(&b.display().to_string());
    pty.send("\r");
    pty.wait_for("both tabs", |_, _| pty.row(0).contains("beta-b"));

    // B runs an agent and a shell command (the probe's Ctrl+E) that end
    // while A is current: both of B's circles turn to finished.
    pty.send("\x05");
    pty.wait_text("STATUS busy");
    pty.send("\x1b[1;3D");
    pty.wait_for("B's finished circles", |_, _| {
        circles(&pty, "beta-b") == [(AGENT.to_string(), DONE), (SHELL.to_string(), DONE)]
    });

    // Back to B: the agent's mark is kept, the shell's is gone.
    pty.send("\x1b[1;3C");
    pty.wait_for("B current, shell mark cleared", |s, _| {
        let [agent, shell] = circles(&pty, "beta-b");
        s.contains("STATUS idle (later)") && agent == (AGENT.to_string(), DONE) && shell.0.is_empty()
    });

    pty.send("\x11");
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
}

/// The selection background (`panels::terminal::selection_style`).
const SELECTED: vt100::Color = vt100::Color::Rgb(97, 175, 239);

/// A shell at its prompt takes no mouse: dragging over its output selects
/// that text (drawn in the selection colours) and the release copies it.
/// `/bin/sh` is an unknown shell, so it runs without integration and
/// without the user's profile.
#[test]
fn dragging_over_a_shell_tab_selects_and_copies_its_text() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    std::fs::create_dir(&a).unwrap();
    let clip = root.path().join("clipboard.txt");
    let state = root.path().join("state.json");
    let mut pty = Pty::spawn_answering(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &[
            ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
            ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
            ("GAVIERO_SESSION_HOST_CLIPBOARD", clip.as_os_str()),
            ("SHELL", OsStr::new("/bin/sh")),
            ("PS1", OsStr::new("$ ")),
        ],
        root.path(),
        Answers::Legacy,
    );
    pty.wait_for("the tab", |_, _| pty.row(0).contains("alpha-a"));

    // Alt+T: a shell tab; its command's output on a line of its own.
    pty.send("\x1bt");
    pty.wait_for("the shell tab", |_, _| pty.row(0).contains(">_"));
    pty.send("echo hello-select\r");
    let output_row = || (1..common::ROWS).find(|&r| pty.row(r) == "hello-select");
    pty.wait_for("the output", |_, _| output_row().is_some());
    let row = output_row().unwrap();

    // Press on its first cell, drag to just past its last, release (SGR,
    // 1-based): the twelve characters are copied.
    let y = row + 1;
    pty.send(&format!("\x1b[<0;1;{y}M\x1b[<32;13;{y}M"));
    pty.wait_for("the highlight", |_, _| {
        pty.cell(row, 0).1 == SELECTED && pty.cell(row, 11).1 == SELECTED && pty.cell(row, 12).1 != SELECTED
    });
    pty.send(&format!("\x1b[<0;13;{y}m"));
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::fs::read_to_string(&clip).unwrap_or_default() != "hello-select" {
        assert!(std::time::Instant::now() < deadline, "clipboard: {:?}", std::fs::read_to_string(&clip));
        std::thread::sleep(Duration::from_millis(50));
    }

    // Typing clears the highlight.
    pty.send("x");
    pty.wait_for("the highlight gone", |_, _| pty.cell(row, 0).1 != SELECTED);

    pty.send("\x11");
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
}

#[test]
fn a_terminal_without_the_protocol_gets_no_flags() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    std::fs::create_dir(&a).unwrap();
    let mut pty = host(root.path(), &a, Answers::Legacy);
    assert_eq!(count(&pty.raw(), b"\x1b[?u"), 1, "the host asks");

    pty.send("\x1bx");
    pty.wait_text("KEY Char('x') KeyModifiers(ALT) Press");
    pty.send("\x11");
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
    let raw = pty.raw();
    assert_eq!(count(&raw, b"\x1b[>"), 0, "nothing pushed");
    assert_eq!(count(&raw, POP), 0, "nothing popped");
}
