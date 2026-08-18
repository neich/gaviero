//! PTY smoke test (Tier W1 / Gate C): spawn the default shell with the
//! same integration wiring `TerminalManager::create_tab` uses and
//! assert output actually flows. Catches platform spawn regressions
//! (ConPTY arg quoting, init-script injection) that unit tests can't.

use gaviero_core::terminal::config::ShellConfig;
use gaviero_core::terminal::shell_integration;
use gaviero_core::terminal::types::TerminalId;

#[test]
fn default_shell_emits_output_through_pty() {
    let mut shell_config = ShellConfig::default_for_user();
    let histdir = tempfile::tempdir().unwrap();
    let histfile = histdir.path().join("hist.txt");

    if shell_config.enable_integration {
        match shell_integration::create_init_file(
            &shell_config.shell_type,
            &TerminalId::next(),
            &histfile,
        ) {
            Ok(init_path) => shell_integration::build_shell_args(&mut shell_config, &init_path),
            Err(e) => eprintln!("no init file ({e}); spawning bare shell"),
        }
    }
    eprintln!(
        "spawning {:?} ({:?}) args {:?}",
        shell_config.shell_path, shell_config.shell_type, shell_config.shell_args
    );

    let handle = gaviero_core::terminal::pty::spawn_pty(
        &shell_config,
        std::env::temp_dir().as_path(),
        24,
        80,
    )
    .expect("spawn_pty");

    let mut reader = handle.reader;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Act like the embedded terminal: answer cursor-position queries
    // (DSR `ESC[6n`) or PSReadLine never renders its prompt, and wait
    // for the init script's OSC 133;A prompt-start marker.
    let mut writer = handle.writer;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut collected: Vec<u8> = Vec::new();
    let marker: &[u8] = b"\x1b]133;A";
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(chunk) => {
                if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                    use std::io::Write as _;
                    let _ = writer.write_all(b"\x1b[1;1R");
                    let _ = writer.flush();
                }
                collected.extend_from_slice(&chunk);
                if collected.windows(marker.len()).any(|w| w == marker) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut child = handle.child;
    let _ = child.kill();
    let _ = child.wait();

    eprintln!(
        "collected {} bytes: {:?}",
        collected.len(),
        String::from_utf8_lossy(&collected)
    );
    assert!(
        collected.windows(marker.len()).any(|w| w == marker),
        "integration prompt marker (OSC 133;A) never arrived — shell \
         failed to start or stalled before the prompt"
    );
}
