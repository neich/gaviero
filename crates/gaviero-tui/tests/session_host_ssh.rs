//! SSH tabs end to end: `gaviero --sessions` in a real pseudo console, with
//! the gaviero binary itself standing in for `ssh` (`GAVIERO_SSH`). Every
//! child — local tabs and the fake `ssh` alike — is an input probe
//! (`GAVIERO_INPUT_PROBE` is inherited), which records the arguments `ssh`
//! was given and can act like a remote gaviero (folder report, clipboard
//! copy, a dropped connection).
#![cfg(windows)]

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "../src/session_host/encode.rs"]
mod encode;

use common::Pty;
use encode::{KeyRecord, LEFT_ALT, LEFT_CTRL, SHIFT, win32_key_seq};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

fn alt_n() -> String {
    tap(0x4E, 0x31, 'n' as u16, LEFT_ALT)
}

fn ctrl_t() -> String {
    tap(0x54, 0x14, 0x14, LEFT_CTRL)
}

fn enter() -> String {
    tap(0x0D, 0x1C, 13, 0)
}

/// Type `text` as key records, each with the virtual-key code and Shift
/// state the current keyboard layout gives that character, paced like
/// typing (see `session_host_e2e.rs`).
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

/// The files one test uses, in its own temporary folder.
struct Fixture {
    root: tempfile::TempDir,
    local: PathBuf,
    state: PathBuf,
    probe_args: PathBuf,
    clipboard: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join("local-a");
        std::fs::create_dir(&local).unwrap();
        Fixture {
            state: root.path().join("state.json"),
            probe_args: root.path().join("probe-args.txt"),
            clipboard: root.path().join("clipboard.txt"),
            local,
            root,
        }
    }

    /// A host on the local folder, with the gaviero binary as its `ssh`.
    fn host(&self) -> Pty {
        let pty = Pty::spawn(
            &[OsStr::new("--sessions"), self.local.as_os_str()],
            &[
                ("GAVIERO_INPUT_PROBE", OsStr::new("1")),
                ("GAVIERO_SESSION_HOST_STATE", self.state.as_os_str()),
                ("GAVIERO_SSH", OsStr::new(env!("CARGO_BIN_EXE_gaviero"))),
                ("GAVIERO_PROBE_ARGS", self.probe_args.as_os_str()),
                ("GAVIERO_SESSION_HOST_CLIPBOARD", self.clipboard.as_os_str()),
            ],
            self.root.path(),
        );
        pty.wait_for("the local tab", |_, _| pty.row(0).contains("local-a"));
        pty
    }

    /// The arguments the fake `ssh` was started with, once it has started.
    fn ssh_args(&self) -> Vec<String> {
        wait_until("the fake ssh to start", || self.probe_args.exists());
        std::fs::read_to_string(&self.probe_args)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// Poll `cond` every 50 ms; panic after 15 s.
fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Alt+N, switch the picker to SSH, type `destination`, Enter.
fn open_ssh(pty: &mut Pty, destination: &str) {
    pty.send(&alt_n());
    pty.wait_text("Ctrl+T: switch");
    pty.send(&ctrl_t());
    pty.wait_text("host or host:/path");
    type_text(pty, destination);
    pty.send(&enter());
}

#[test]
fn an_ssh_tab_runs_the_remote_command() {
    let fx = Fixture::new();
    let mut pty = fx.host();

    open_ssh(&mut pty, "testhost:/srv/proj");
    pty.wait_for("the SSH tab", |_, _| pty.row(0).contains("testhost:proj"));
    assert!(pty.row(0).contains("local-a"), "row 0: {}", pty.row(0));
    assert_eq!(
        fx.ssh_args(),
        vec![
            "-t",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "testhost",
            "exec $SHELL -l -c 'exec env GAVIERO_SESSION_HOST=1 gaviero \"/srv/proj\"'",
        ]
    );

    // Keys reach the SSH tab like any other.
    pty.send(&tap(0x58, 0x2D, 'x' as u16, 0));
    pty.wait_text("KEY Char('x')");

    pty.send(&ctrl_q());
    assert_eq!(pty.wait_exit(Duration::from_secs(15)), Some(0));
}

#[test]
fn shift_enter_reaches_an_ssh_tab_as_alt_enter() {
    let fx = Fixture::new();
    let mut pty = fx.host();

    open_ssh(&mut pty, "testhost:/srv/proj");
    pty.wait_for("the SSH tab", |_, _| pty.row(0).contains("testhost:proj"));
    fx.ssh_args();

    // `ssh.exe` reads VT, where conhost writes Shift+Enter as a bare CR; the
    // host sends Alt+Enter instead, which crosses as `ESC CR`.
    pty.send(&tap(0x0D, 0x1C, 13, SHIFT));
    pty.wait_text("KEY Enter KeyModifiers(ALT) Press");
    let screen = pty.contents();
    assert!(!screen.contains("SHIFT"), "screen:\n{screen}");

    pty.send(&ctrl_q());
    assert_eq!(pty.wait_exit(Duration::from_secs(15)), Some(0));
}

#[test]
fn a_picked_remote_folder_is_adopted_and_saved() {
    let fx = Fixture::new();
    let mut pty = fx.host();

    // No path: the remote gaviero is asked to let the user pick one.
    open_ssh(&mut pty, "testhost");
    pty.wait_for("the SSH tab", |_, _| pty.row(0).contains("testhost:~"));
    let args = fx.ssh_args();
    assert!(
        args.last().unwrap().ends_with("gaviero --pick'"),
        "args: {args:?}"
    );

    // The remote gaviero reports the folder it opened.
    pty.send(&tap(0x4F, 0x18, 15, LEFT_CTRL));
    pty.wait_for("the reported folder", |_, _| {
        pty.row(0).contains("testhost:my proj")
    });

    // Quit: the SSH tab is kept as the folder it reported.
    pty.send(&ctrl_q());
    assert_eq!(pty.wait_exit(Duration::from_secs(15)), Some(0));
    let text = std::fs::read_to_string(&fx.state).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let open: Vec<&str> = json["open"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(open.contains(&"ssh://testhost/srv/my proj"), "open: {open:?}");
    assert!(!open.contains(&"ssh://testhost"), "open: {open:?}");
}

#[test]
fn a_lost_connection_keeps_the_tab_until_enter_reconnects() {
    let fx = Fixture::new();
    let mut pty = fx.host();

    open_ssh(&mut pty, "testhost:/srv/proj");
    pty.wait_for("the SSH tab", |_, _| pty.row(0).contains("testhost:proj"));
    fx.ssh_args();
    std::fs::remove_file(&fx.probe_args).unwrap();

    // ssh exits with 255: the tab stays, with the reconnect box.
    pty.send(&tap(0x4B, 0x25, 11, LEFT_CTRL));
    pty.wait_text("Enter reconnect");
    assert!(pty.contents().contains("Disconnected"), "screen:\n{}", pty.contents());
    assert!(pty.row(0).contains("testhost:proj"), "row 0: {}", pty.row(0));

    // Other keys do nothing; Enter connects again in the same tab.
    pty.send(&tap(0x58, 0x2D, 'x' as u16, 0));
    assert!(!fx.probe_args.exists(), "a key other than Enter reconnected");
    pty.send(&enter());
    fx.ssh_args();
    pty.wait_for("the reconnected tab", |s, _| !s.contains("Enter reconnect"));
    assert!(pty.row(0).contains("testhost:proj"), "row 0: {}", pty.row(0));

    pty.send(&ctrl_q());
    assert_eq!(pty.wait_exit(Duration::from_secs(15)), Some(0));
}

#[test]
fn clipboard_crosses_to_and_from_an_ssh_tab() {
    let fx = Fixture::new();
    let mut pty = fx.host();

    open_ssh(&mut pty, "testhost:/srv/proj");
    pty.wait_for("the SSH tab", |_, _| pty.row(0).contains("testhost:proj"));
    fx.ssh_args();

    // The remote copies (OSC 52): the text lands on this machine's clipboard.
    pty.send(&tap(0x59, 0x15, 25, LEFT_CTRL));
    wait_until("the remote copy on the local clipboard", || {
        std::fs::read_to_string(&fx.clipboard).is_ok_and(|t| t == "probe-copy")
    });

    // Ctrl+V pastes this machine's clipboard into the SSH tab.
    std::fs::write(&fx.clipboard, "Zq").unwrap();
    pty.send(&tap(0x56, 0x2F, 22, LEFT_CTRL));
    pty.wait_text("KEY Char('Z')");
    pty.wait_text("KEY Char('q')");
    assert!(!pty.contents().contains("Char('v')"), "screen:\n{}", pty.contents());

    pty.send(&ctrl_q());
    assert_eq!(pty.wait_exit(Duration::from_secs(15)), Some(0));
}
