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
