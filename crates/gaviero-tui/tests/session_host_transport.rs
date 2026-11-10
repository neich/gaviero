//! Transport proof for the session host: win32-input-mode key records and
//! SGR mouse reports written to a ConPTY reach a crossterm child (the
//! `GAVIERO_INPUT_PROBE` mode of the `gaviero` binary) as the right events.
#![cfg(windows)]

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "../src/session_host/encode.rs"]
mod encode;

use common::Pty;
use encode::{KeyRecord, LEFT_ALT, LEFT_CTRL, RIGHT_ALT, sgr_mouse, win32_key_seq};
use std::ffi::OsStr;
use std::time::Duration;

/// A key-down record followed by its key-up, as psmux sends them.
fn tap(vk: u16, scan: u16, uchar: u16, ctrl_state: u32) -> String {
    let mut r = KeyRecord {
        vk,
        scan,
        uchar,
        key_down: true,
        ctrl_state,
        repeat: 1,
    };
    let down = win32_key_seq(&r);
    r.key_down = false;
    down + &win32_key_seq(&r)
}

fn line_with(contents: &str, all: &[&str]) -> bool {
    contents.lines().any(|l| all.iter().all(|n| l.contains(n)))
}

#[test]
fn keys_and_mouse_reach_a_crossterm_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut pty = Pty::spawn(
        &[],
        &[("GAVIERO_INPUT_PROBE", OsStr::new("1"))],
        dir.path(),
    );

    // 1. The probe is up.
    pty.wait_for("title probe-ready", |_, t| t == Some("probe-ready"));

    // 2. A plain key.
    pty.send(&tap(0x58, 0x2D, 'x' as u16, 0));
    pty.wait_text("KEY Char('x')");

    // 3. Ctrl+Alt+Left keeps both modifiers.
    pty.send(&tap(0x25, 0x4B, 0, LEFT_CTRL | LEFT_ALT));
    pty.wait_for("Ctrl+Alt+Left", |s, _| {
        line_with(s, &["KEY Left", "CONTROL | ALT"])
    });

    // 4. AltGr resolves to its layout character.
    pty.send(&tap(0x32, 0x03, '@' as u16, RIGHT_ALT | LEFT_CTRL));
    pty.wait_text("Char('@')");

    // 5. SGR mouse press and release become mouse events.
    pty.send(&sgr_mouse(0, 9, 4, true));
    pty.send(&sgr_mouse(0, 9, 4, false));
    pty.wait_text("MOUSE Down(Left) 9 4");
    pty.wait_text("MOUSE Up(Left) 9 4");

    // 6. Wheel down.
    pty.send(&sgr_mouse(65, 3, 3, true));
    pty.wait_text("MOUSE ScrollDown 3 3");

    // 7. Ctrl+Q ends the probe.
    pty.send(&tap(0x51, 0x10, 17, LEFT_CTRL));
    let code = pty.wait_exit(Duration::from_secs(10));
    assert!(code.is_some(), "probe did not exit; screen:\n{}", pty.contents());
}
