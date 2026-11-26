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
/// The badges' letters (`session_host/render.rs`).
const AGENT: &str = "A";
const SHELL: &str = "S";

/// The badges after `label` on the tab bar, as [agent, shell]: (letter, its
/// cell's background) each, a blank letter when not shown. Each shown badge
/// is a blank, then its letter on the badge colour, agent first, and the tab
/// is only as wide as the badges it shows — so a cell after the label is a
/// badge only when it holds a badge letter (otherwise the trailing pad, the
/// separator or the next tab).
fn circles(pty: &Pty, label: &str) -> [(String, vt100::Color); 2] {
    let bar = pty.row(0);
    let at = bar.find(label).expect("label on the tab bar");
    let end = (bar[..at].chars().count() + label.chars().count()) as u16;
    let mut shown = [(String::new(), vt100::Color::Default), (String::new(), vt100::Color::Default)];
    for col in [end + 1, end + 3] {
        let (letter, bg) = pty.cell(0, col);
        let slot = match letter.trim() {
            AGENT => 0,
            SHELL => 1,
            _ => break,
        };
        shown[slot] = (letter.trim().to_string(), bg);
    }
    shown
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

/// Ctrl+Alt+R opens the rename popup (in both legacy VT and kitty-protocol
/// terminals), Esc cancels without leaking keys, Enter locks the name so
/// another tab with the same folder name does not disturb it, and quitting
/// and reopening keeps the custom name.
#[test]
fn ctrl_alt_r_renames_a_tab_and_keeps_the_name_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    let clash = root.path().join("Backend");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&clash).unwrap();
    let state = root.path().join("state.json");

    let mut pty = Pty::spawn_answering(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &[
            ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
            ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ],
        root.path(),
        Answers::Legacy,
    );
    pty.wait_for("the tab", |_, _| pty.row(0).contains("alpha-a"));

    // Legacy VT Ctrl+Alt+R (ESC + 0x12): Esc cancels and leaks nothing.
    pty.send("\x1b\x12");
    pty.wait_text(" Rename tab ");
    pty.send("Discard");
    pty.send("\x1b");
    pty.wait_for("popup closed", |s, _| !s.contains(" Rename tab "));
    assert!(pty.row(0).contains("alpha-a"), "row 0: {}", pty.row(0));
    assert!(!pty.contents().contains("KEY"), "leaked to probe:\n{}", pty.contents());

    // Rename to "Backend" (initial label is pre-selected, so typing replaces it).
    pty.send("\x1b\x12");
    pty.wait_text(" Rename tab ");
    pty.send("Backend\r");
    pty.wait_for("renamed tab", |s, _| {
        !s.contains(" Rename tab ") && pty.row(0).contains("Backend") && !pty.row(0).contains("alpha-a")
    });
    assert!(!pty.contents().contains("KEY"), "leaked to probe:\n{}", pty.contents());

    // Opening a folder literally named "Backend" numbers the new tab, never the renamed one.
    pty.send("\x1bn");
    pty.wait_text(" New session ");
    pty.send(&clash.display().to_string());
    pty.send("\r");
    pty.wait_for("both tabs", |_, _| {
        let bar = pty.row(0);
        bar.contains(" x Backend ") && bar.contains("Backend (2)")
    });

    // Ctrl+Q quits and keeps both tabs plus the custom name.
    pty.send("\x11");
    assert_eq!(pty.wait_exit(Duration::from_secs(10)), Some(0));
    drop(pty);

    // Reopen under kitty protocol: "Backend" is restored and Ctrl+Alt+R (\x1b[114;7u) renames again.
    let mut pty = Pty::spawn_answering(
        &[OsStr::new("--sessions")],
        &[
            ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
            ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ],
        root.path(),
        Answers::Kitty,
    );
    pty.wait_for("reopened tabs", |_, _| {
        let bar = pty.row(0);
        bar.contains(" x Backend ") && bar.contains("Backend (2)")
    });

    // Switch back to the first tab ("Backend") and rename via kitty Ctrl+Alt+R.
    pty.send("\x1b[1;3D");
    pty.send("\x1b[114;7u");
    pty.wait_text(" Rename tab ");
    pty.send("Core\r");
    pty.wait_for("renamed via kitty", |s, _| {
        !s.contains(" Rename tab ") && pty.row(0).contains(" x Core ")
    });

    pty.send("\x1b[113;5u");
    assert_eq!(pty.wait_exit(Duration::from_secs(10)), Some(0));
}

/// A renamed shell tab never reverts when the shell changes folder (OSC 7)
/// and reopens with its custom name in the new folder.
#[test]
fn a_renamed_shell_tab_keeps_its_name_across_cd_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    let sub = a.join("sub-dir");
    std::fs::create_dir_all(&sub).unwrap();
    let state = root.path().join("state.json");
    let envs = [
        ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
        ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ("SHELL", OsStr::new("/bin/sh")),
        ("PS1", OsStr::new("$ ")),
    ];

    let mut pty = Pty::spawn_answering(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &envs,
        root.path(),
        Answers::Legacy,
    );
    pty.wait_for("alpha tab", |_, _| pty.row(0).contains("alpha-a"));

    // Open a shell tab (Alt+T) and rename it to "Logs".
    pty.send("\x1bt");
    pty.wait_for("shell tab", |_, _| pty.row(0).contains(">_ alpha-a"));
    pty.send("\x1b\x12");
    pty.wait_text(" Rename tab ");
    pty.send("Logs\r");
    pty.wait_for("renamed shell tab", |s, _| {
        !s.contains(" Rename tab ") && pty.row(0).contains("Logs") && !pty.row(0).contains(">_")
    });

    // Change directory and report OSC 7 as shell integration does.
    pty.send("cd sub-dir && printf '\\033]7;file://localhost%s\\007done-cd\\n' \"$PWD\"\r");
    pty.wait_text("done-cd");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(&state).unwrap_or_default();
        if text.contains("sub-dir") && text.contains("\"Logs\"") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "state.json:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(pty.row(0).contains("Logs"), "row 0: {}", pty.row(0));
    assert!(!pty.row(0).contains("sub-dir"), "row 0: {}", pty.row(0));

    // Quit and reopen: shell tab reopens in sub-dir, still named "Logs".
    pty.send("\x11");
    assert_eq!(pty.wait_exit(Duration::from_secs(10)), Some(0));
    drop(pty);

    let mut pty = Pty::spawn_answering(&[OsStr::new("--sessions")], &envs, root.path(), Answers::Legacy);
    pty.wait_for("reopened renamed shell", |_, _| {
        let bar = pty.row(0);
        bar.contains("alpha-a") && bar.contains("Logs") && !bar.contains("sub-dir")
    });
    pty.send("\x11");
    assert_eq!(pty.wait_exit(Duration::from_secs(10)), Some(0));
}

/// A renamed SSH tab keeps its name when the remote reports its picked folder
/// (OSC 7), when the connection drops and reconnects, and across reopen.
#[test]
fn a_renamed_ssh_tab_keeps_its_name_across_osc7_and_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    std::fs::create_dir(&a).unwrap();
    let state = root.path().join("state.json");
    let args_file = root.path().join("ssh-args.txt");
    let envs = [
        ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
        ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ("GAVIERO_SSH", OsStr::new(env!("CARGO_BIN_EXE_gaviero"))),
        ("GAVIERO_PROBE_ARGS", args_file.as_os_str()),
    ];

    let mut pty = Pty::spawn_answering(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &envs,
        root.path(),
        Answers::Legacy,
    );
    pty.wait_for("alpha tab", |_, _| pty.row(0).contains("alpha-a"));

    // Alt+N -> Ctrl+T (SSH) -> "testhost" -> Enter.
    pty.send("\x1bn");
    pty.wait_text("Ctrl+T: switch");
    pty.send("\x14");
    pty.wait_text("host or host:/path");
    pty.send("testhost\r");
    pty.wait_for("SSH tab", |_, _| pty.row(0).contains("testhost:~"));

    // Rename SSH tab to "Prod" before remote picks its folder.
    pty.send("\x1b\x12");
    pty.wait_text(" Rename tab ");
    pty.send("Prod\r");
    pty.wait_for("renamed SSH tab", |s, _| {
        !s.contains(" Rename tab ") && pty.row(0).contains("Prod") && !pty.row(0).contains("testhost")
    });

    // Probe reports OSC 7 /srv/my proj (Ctrl+O): target updates, name stays "Prod".
    pty.send("\x0f");
    pty.wait_text("STATUS osc7");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(&state).unwrap_or_default();
        if text.contains("ssh://testhost/srv/my proj") && text.contains("\"Prod\"") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "state.json:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(pty.row(0).contains("Prod"), "row 0: {}", pty.row(0));
    assert!(!pty.row(0).contains("my proj"), "row 0: {}", pty.row(0));

    // Connection drops (Ctrl+K -> exit 255) and Enter reconnects: still "Prod".
    let _ = std::fs::remove_file(&args_file);
    pty.send("\x0b");
    pty.wait_text("Enter reconnect");
    assert!(pty.row(0).contains("Prod"), "row 0: {}", pty.row(0));
    pty.send("\r");
    pty.wait_for("reconnected", |s, _| !s.contains("Enter reconnect"));
    // Another OSC 7 after reconnect still leaves "Prod" untouched.
    pty.send("\x0f");
    pty.wait_text("STATUS osc7");
    assert!(pty.row(0).contains("Prod"), "row 0: {}", pty.row(0));
    assert!(!pty.row(0).contains("my proj"), "row 0: {}", pty.row(0));

    pty.send("\x11");
    assert_eq!(pty.wait_exit(Duration::from_secs(10)), Some(0));
}
