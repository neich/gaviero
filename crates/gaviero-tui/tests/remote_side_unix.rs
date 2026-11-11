//! What an SSH tab's remote gaviero does, run locally on Unix in a real
//! pseudo-terminal: `gaviero --pick` browses to a folder and opens it,
//! reports that folder to the session host with OSC 7, and a hang-up
//! (SIGHUP — the SSH connection or tab went away) still saves the session.
#![cfg(unix)]

#[allow(dead_code)]
mod common;

use common::Pty;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

/// `file://` + the path with every byte outside `A-Z a-z 0-9 - . _ ~ /`
/// percent-encoded, as gaviero writes it (`session_host::osc7`).
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &b in path.to_str().unwrap().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

#[test]
fn pick_reports_the_folder_and_hangup_saves() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let dir = |name: &str| {
        let p = root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let (home, cache, data, config, groot) =
        (dir("home"), dir("cache"), dir("data"), dir("config"), dir("groot"));
    let project = home.join("proj a");
    std::fs::create_dir(&project).unwrap();

    let mut pty = Pty::spawn(
        &[OsStr::new("--pick"), home.as_os_str()],
        &[
            ("HOME", home.as_os_str()),
            ("XDG_CACHE_HOME", cache.as_os_str()),
            ("XDG_DATA_HOME", data.as_os_str()),
            ("XDG_CONFIG_HOME", config.as_os_str()),
            ("GAVIERO_ROOT", groot.as_os_str()),
            ("GAVIERO_SESSION_HOST", OsStr::new("1")),
        ],
        &home,
    );

    // 1. The folder browser comes first; type the folder and open it.
    pty.wait_text("New session");
    pty.send(&format!("{}/", project.display()));
    pty.wait_text("proj a/");
    pty.send("\r");

    // 2. The folder is new to gaviero: the first-run screen; editor only.
    pty.wait_text("Editor only");
    pty.send("\r");

    // 3. Under a session host it reports its status and the opened folder.
    let want = file_url(&project);
    pty.wait_for("status title and folder report", |_, title| {
        title.is_some_and(|t| t.starts_with("gaviero-session:"))
            && pty.osc7().as_deref() == Some(want.as_str())
    });

    // 4. The terminal hangs up: gaviero saves its session and exits.
    let pid = pty.pid().expect("gaviero's pid");
    let status = std::process::Command::new("kill")
        .args(["-HUP", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let code = pty.wait_exit(Duration::from_secs(10));
    assert!(code.is_some(), "gaviero kept running after SIGHUP");
    let log = std::fs::read_to_string(cache.join("gaviero").join("gaviero.log")).unwrap();
    assert!(
        log.contains("exit: session state saved"),
        "the session was not saved on hang-up; log tail:\n{}",
        log.lines().rev().take(20).collect::<Vec<_>>().join("\n")
    );
}
