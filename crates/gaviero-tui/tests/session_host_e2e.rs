//! End-to-end session host tests: `gaviero --sessions` in a real pseudo
//! console, with every session child an input probe (`GAVIERO_INPUT_PROBE`
//! is inherited by the children; the host itself checks `--sessions` first
//! and stays a host).
#![cfg(windows)]

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "../src/session_host/encode.rs"]
mod encode;

use common::Pty;
use encode::{KeyRecord, LEFT_ALT, LEFT_CTRL, SHIFT, sgr_mouse, win32_key_seq};
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

/// A key-down record followed by its key-up.
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

fn ctrl_q() -> String {
    tap(0x51, 0x10, 17, LEFT_CTRL)
}

/// The probe reports work in progress (an agent and a shell command).
fn ctrl_b() -> String {
    tap(0x42, 0x30, 2, LEFT_CTRL)
}

/// The probe reports an agent and a shell command that end 2 s later.
fn ctrl_e() -> String {
    tap(0x45, 0x12, 5, LEFT_CTRL)
}

fn alt_left() -> String {
    tap(0x25, 0x4B, 0, LEFT_ALT)
}

fn alt_right() -> String {
    tap(0x27, 0x4D, 0, LEFT_ALT)
}

fn alt_x() -> String {
    tap(0x58, 0x2D, 'x' as u16, LEFT_ALT)
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Start a host on `args` with probe children and `state` as its state file.
fn host(args: &[&OsStr], state: &Path, cwd: &Path) -> Pty {
    Pty::spawn(
        args,
        &[
            ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
            ("GAVIERO_SESSION_HOST_STATE", state.as_os_str()),
        ],
        cwd,
    )
}

#[test]
fn host_forwards_input_and_switches_tabs() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    std::fs::create_dir(&a).unwrap();
    let state = root.path().join("state.json");
    let mut pty = host(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &state,
        root.path(),
    );

    // 1. The tab bar shows A's folder; nothing went wrong.
    let label = name_of(&a);
    pty.wait_for("A's tab", |_, _| pty.row(0).contains(&label));
    assert!(
        !pty.contents().to_lowercase().contains("error"),
        "screen:\n{}",
        pty.contents()
    );

    // 2. A key reaches the probe and its output is drawn by the host.
    pty.send(&tap(0x58, 0x2D, 'x' as u16, 0));
    pty.wait_text("KEY Char('x')");

    // 3. A click below the tab bar arrives one row up.
    pty.send(&sgr_mouse(0, 9, 5, true));
    pty.send(&sgr_mouse(0, 9, 5, false));
    pty.wait_text("MOUSE Down(Left) 9 4");

    // 4. Alt+Right with a single session: still one tab, still alive.
    pty.send(&tap(0x27, 0x4D, 0, LEFT_ALT));
    pty.send(&tap(0x59, 0x15, 'y' as u16, 0));
    pty.wait_text("KEY Char('y')");
    assert_eq!(pty.row(0).matches(&label).count(), 1, "row 0: {}", pty.row(0));

    // 5. Alt+X is the host's: it never reaches the session. Nothing is in
    // progress, so it closes the tab without asking.
    assert!(pty.row(0).starts_with(" x "), "row 0: {}", pty.row(0));
    pty.send(&alt_x());

    // 6. That closed the last tab, so the host ends. The probe ignores the
    // exit request: the host kills it after the grace period.
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
    assert!(!pty.contents().contains("ALT"), "screen:\n{}", pty.contents());
    assert!(open_names(&state).is_empty());
}

/// Type `text` as key records, each with the virtual-key code and Shift
/// state the current keyboard layout gives that character, paced like
/// typing: an unpaced burst merges in the pipe and ConPTY's input parser
/// splits sequences at its read boundaries, turning them into literal text.
fn type_text(pty: &mut Pty, text: &str) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;
    for c in text.chars() {
        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            // SAFETY: a pure keyboard-layout lookup.
            let scan = unsafe { VkKeyScanW(*unit) } as u16;
            let (vk, shift) = (scan & 0xFF, (scan >> 8) & 1 != 0);
            let ctrl_state = if shift { SHIFT } else { 0 };
            pty.send(&tap(vk, 0, *unit, ctrl_state));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The file names of the `open` list in the host state file.
fn open_names(state: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(state).expect("state file");
    let json: serde_json::Value = serde_json::from_str(&text).expect("state json");
    json["open"]
        .as_array()
        .expect("open list")
        .iter()
        .map(|p| name_of(Path::new(p.as_str().unwrap())))
        .collect()
}

#[test]
fn two_sessions_switch_close_and_quit() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    let b = root.path().join("beta-b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let (la, lb) = (name_of(&a), name_of(&b));
    let state = root.path().join("state.json");

    let mut pty = host(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &state,
        root.path(),
    );
    pty.wait_for("A's tab", |_, _| pty.row(0).contains(&la));

    // Alt+N opens the picker; typing B's full path + Enter opens B.
    open_in_picker(&mut pty, &b);
    pty.wait_for("both tabs", |_, _| {
        let bar = pty.row(0);
        bar.contains(&la) && bar.contains(&lb)
    });

    // B is active.
    pty.send(&tap(0x4B, 0x25, 'k' as u16, 0));
    pty.wait_text("KEY Char('k')");

    // Alt+Left shows A's screen.
    pty.send(&tap(0x25, 0x4B, 0, LEFT_ALT));
    pty.send(&tap(0x5A, 0x2C, 'z' as u16, 0));
    pty.wait_for("A's screen", |s, _| {
        s.contains("KEY Char('z')") && !s.contains("KEY Char('k')")
    });

    // Alt+Right shows B's again.
    pty.send(&tap(0x27, 0x4D, 0, LEFT_ALT));
    pty.wait_for("B's screen", |s, _| {
        s.contains("KEY Char('k')") && !s.contains("KEY Char('z')")
    });

    // B has work in progress: Alt+X asks first; any key but y cancels and
    // never reaches the session.
    pty.send(&ctrl_b());
    pty.wait_text("STATUS busy");
    pty.send(&alt_x());
    let close_b = format!(" Close {lb}? ");
    pty.wait_text(&close_b);
    let screen = pty.contents();
    assert!(screen.contains("an agent is running"), "screen:\n{screen}");
    assert!(screen.contains("a shell is running a command"), "screen:\n{screen}");
    pty.send(&tap(0x4E, 0x31, 'n' as u16, 0));
    pty.wait_for("guard cancelled", |s, _| !s.contains(&close_b));
    assert!(pty.row(0).contains(&lb), "row 0: {}", pty.row(0));
    assert!(!pty.contents().contains("Char('n')"), "screen:\n{}", pty.contents());

    // Asked again, y closes B: its tab goes and A, the one before, is
    // current.
    pty.send(&alt_x());
    pty.wait_text(&close_b);
    pty.send(&tap(0x59, 0x15, 'y' as u16, 0));
    pty.wait_for("A's screen after closing B", |s, _| {
        s.contains("KEY Char('z')") && !pty.row(0).contains(&lb)
    });
    assert_eq!(open_names(&state), vec![la.clone()]);

    // Open B again.
    open_in_picker(&mut pty, &b);
    pty.wait_for("both tabs", |_, _| {
        let bar = pty.row(0);
        bar.contains(&la) && bar.contains(&lb)
    });
    assert_eq!(open_names(&state), vec![la.clone(), lb.clone()]);

    // B has work in progress, so Alt+Q asks, naming it; 'y' quits the
    // host and keeps both tabs for next time. The probes ignore the exit
    // request and are killed after the grace.
    pty.send(&ctrl_b());
    pty.wait_text("STATUS busy");
    pty.send(&tap(0x51, 0x10, 'q' as u16, LEFT_ALT));
    pty.wait_text(" Quit gaviero? ");
    let named = format!("{lb}: an agent is running");
    assert!(pty.contents().contains(&named), "screen:\n{}", pty.contents());
    assert!(!pty.contents().contains(&format!("{la}:")), "screen:\n{}", pty.contents());
    pty.send(&tap(0x59, 0x15, 'y' as u16, 0));
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
    assert_eq!(open_names(&state), vec![la.clone(), lb.clone()]);
    drop(pty);

    // `--sessions` with no path reopens both; B, opened last, is current.
    let mut pty = host(&[OsStr::new("--sessions")], &state, root.path());
    pty.wait_for("both tabs reopened", |_, _| {
        let bar = pty.row(0);
        bar.contains(&la) && bar.contains(&lb)
    });

    // A click on A's close button closes A; B stays current.
    pty.send(&sgr_mouse(0, 1, 0, true));
    pty.send(&sgr_mouse(0, 1, 0, false));
    pty.wait_for("A closed", |_, _| !pty.row(0).contains(&la));
    pty.send(&tap(0x57, 0x11, 'w' as u16, 0));
    pty.wait_text("KEY Char('w')");
    assert!(!pty.contents().contains("MOUSE"), "screen:\n{}", pty.contents());

    // Ctrl+Q in the session ends it: the last tab, so the host ends too.
    pty.send(&ctrl_q());
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
    assert!(open_names(&state).is_empty());
}

/// The host's circle colours (`session_host/render.rs`).
const RUNNING: vt100::Color = vt100::Color::Rgb(152, 195, 121);
const DONE: vt100::Color = vt100::Color::Rgb(97, 175, 239);

/// The two circle cells after `label` on the tab bar: (symbol, colour) each.
fn circles(pty: &Pty, label: &str) -> [(String, vt100::Color); 2] {
    let bar = pty.row(0);
    let at = bar.find(label).expect("label on the tab bar");
    let col = (bar[..at].chars().count() + label.chars().count() + 1) as u16;
    [pty.cell(0, col), pty.cell(0, col + 1)]
}

#[test]
fn background_tabs_show_agent_and_shell_activity() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("alpha-a");
    let b = root.path().join("beta-b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let (la, lb) = (name_of(&a), name_of(&b));
    let state = root.path().join("state.json");
    let mut pty = host(
        &[OsStr::new("--sessions"), a.as_os_str()],
        &state,
        root.path(),
    );
    pty.wait_for("A's tab", |_, _| pty.row(0).contains(&la));
    open_in_picker(&mut pty, &b);
    pty.wait_for("both tabs", |_, _| {
        let bar = pty.row(0);
        bar.contains(&la) && bar.contains(&lb)
    });

    // B, current, starts an agent and a shell command: no circles on it.
    pty.send(&ctrl_e());
    pty.wait_text("STATUS busy");
    assert!(!pty.row(0).contains('●'), "row 0: {}", pty.row(0));

    // Seen from A, B shows both running.
    pty.send(&alt_left());
    pty.wait_for("B's running circles", |_, _| {
        let [agent, shell] = circles(&pty, &lb);
        agent == ("●".to_string(), RUNNING) && shell == ("●".to_string(), RUNNING)
    });
    let [a_agent, a_shell] = circles(&pty, &la);
    assert_eq!((a_agent.0.trim(), a_shell.0.trim()), ("", ""), "A has none");

    // The work ends while B is out of view: both turn to finished.
    pty.wait_for("B's finished circles", |_, _| {
        let [agent, shell] = circles(&pty, &lb);
        agent == ("●".to_string(), DONE) && shell == ("●".to_string(), DONE)
    });

    // Making B current clears them, and they stay clear when B is left.
    pty.send(&alt_right());
    pty.wait_for("B current, circles cleared", |s, _| {
        s.contains("STATUS idle (later)") && !pty.row(0).contains('●')
    });

    // Work that ends while B is current leaves no finished mark.
    pty.send(&ctrl_e());
    pty.wait_for("B's second run ended", |s, _| {
        s.matches("STATUS idle (later)").count() == 2
    });
    pty.send(&alt_left());
    pty.send(&tap(0x5A, 0x2C, 'z' as u16, 0));
    pty.wait_text("KEY Char('z')");
    assert!(!pty.row(0).contains('●'), "row 0: {}", pty.row(0));

    // Nothing in progress: Alt+Q quits at once.
    pty.send(&tap(0x51, 0x10, 'q' as u16, LEFT_ALT));
    let code = pty.wait_exit(Duration::from_secs(10));
    assert_eq!(code, Some(0), "screen:\n{}", pty.contents());
}

/// Alt+N, the full path of `dir`, Enter.
fn open_in_picker(pty: &mut Pty, dir: &Path) {
    pty.send(&tap(0x4E, 0x31, 'n' as u16, LEFT_ALT));
    pty.wait_text(" New session ");
    type_text(pty, &dir.display().to_string());
    pty.send(&tap(0x0D, 0x1C, 13, 0));
}
