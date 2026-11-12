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
use encode::{KeyEncoder, KeyRecord, LEFT_ALT, LEFT_CTRL, RIGHT_ALT, SHIFT, sgr_mouse, win32_key_seq};
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

/// The records the console hands the host for pasted `text`: each
/// character down + up, capitals wrapped in Shift down/up, like conhost
/// generates them.
fn paste_records(text: &str) -> Vec<KeyRecord> {
    let rec = |vk: u16, uchar: u16, key_down: bool, ctrl_state: u32| KeyRecord {
        vk,
        scan: 0,
        uchar,
        key_down,
        ctrl_state,
        repeat: 1,
    };
    let mut out = Vec::new();
    for c in text.encode_utf16() {
        let upper = (b'A' as u16..=b'Z' as u16).contains(&c);
        let vk = match c {
            0x61..=0x7A => c - 0x20,
            0x0D => 0x0D,
            _ if upper => c,
            _ => 0,
        };
        let shift = if upper { SHIFT } else { 0 };
        if upper {
            out.push(rec(0x10, 0, true, SHIFT));
        }
        out.push(rec(vk, c, true, shift));
        out.push(rec(vk, c, false, shift));
        if upper {
            out.push(rec(0x10, 0, false, 0));
        }
    }
    out
}

/// The key code the probe printed for each key press on screen, oldest first.
fn pressed(screen: &str) -> Vec<String> {
    screen
        .lines()
        .filter(|l| l.starts_with("KEY ") && l.ends_with("Press"))
        .map(|l| {
            let code = &l["KEY ".len()..];
            match code.find("') ") {
                Some(end) => code[..end + 2].to_string(),
                None => code.split(' ').next().unwrap_or_default().to_string(),
            }
        })
        .collect()
}

/// A long Spanish paste (no digits, `;` or `_`, so any sequence fragment
/// that leaks out as text is easy to tell apart).
fn paste_text() -> String {
    let para = "La propuesta se ancla en los modelos consolidados de educación \
                diabetológica estructurada en DAFNE y BERTIE en Reino Unido.\r";
    para.repeat(25)
}

/// Feed the records of a paste to the probe one write per record (as the
/// host's writer thread does), end with a Ctrl+D marker, and return the
/// key presses the probe printed on its last screen together with the ones
/// the paste should have produced.
fn paste_through(encode: impl Fn(&KeyRecord) -> Vec<u8>) -> (Vec<String>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let mut pty = Pty::spawn(
        &[],
        &[("GAVIERO_INPUT_PROBE", OsStr::new("1"))],
        dir.path(),
    );
    pty.wait_for("title probe-ready", |_, t| t == Some("probe-ready"));
    let text = paste_text();
    for r in paste_records(&text) {
        let bytes = encode(&r);
        if !bytes.is_empty() {
            pty.send(std::str::from_utf8(&bytes).unwrap());
        }
    }
    pty.send(&tap(0x44, 0x20, 4, LEFT_CTRL));
    pty.wait_text_within("STATUS idle", Duration::from_secs(60));
    let got = pressed(&pty.contents());
    let want: Vec<String> = text
        .chars()
        .map(|c| match c {
            '\r' => "Enter".to_string(),
            c => format!("Char({c:?})"),
        })
        .collect();
    pty.send(&tap(0x51, 0x10, 17, LEFT_CTRL));
    let _ = pty.wait_exit(Duration::from_secs(10));
    (got, want)
}

/// A paste is a burst that fills ConPTY's input pipe. Sent as text, every
/// character arrives in order however the pipe is read. (Sent as one
/// win32-input-mode sequence per record, the same burst ends with fragments
/// like `3;0;13;1;0;1_` typed into the child — the bug `KeyEncoder` fixes.)
#[test]
fn a_paste_burst_reaches_a_crossterm_child_intact() {
    let enc = std::cell::RefCell::new(KeyEncoder::default());
    let (got, want) = paste_through(|r| enc.borrow_mut().encode(r));
    assert!(got.len() >= 10, "too few key presses on screen: {got:?}");
    assert_eq!(got[..], want[want.len() - got.len()..]);
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

    // 6b. Ctrl+wheel up (SGR 64 + Ctrl 16): ConPTY keeps the modifier, so
    // when a live Windows Terminal wheel arrives without Ctrl the loss is
    // upstream (`platform::ctrl_key_held` covers it).
    pty.send(&sgr_mouse(64 + 16, 4, 3, true));
    pty.wait_for("Ctrl+wheel", |s, _| {
        line_with(s, &["MOUSE ScrollUp 4 3", "CONTROL"])
    });

    // 7. Ctrl+Q ends the probe.
    pty.send(&tap(0x51, 0x10, 17, LEFT_CTRL));
    let code = pty.wait_exit(Duration::from_secs(10));
    assert!(code.is_some(), "probe did not exit; screen:\n{}", pty.contents());
}
