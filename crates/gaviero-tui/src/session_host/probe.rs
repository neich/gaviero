//! Input probe: a tiny crossterm app that prints one line per input event.
//!
//! Test-only plumbing for `tests/session_host_*.rs`, reached when
//! `GAVIERO_INPUT_PROBE` is set. It sets the terminal up the way the real
//! TUI does (raw mode, alternate screen, mouse capture, VT mouse
//! passthrough), so what it prints is what gaviero itself would receive
//! through a pseudo console. Ctrl+Q exits; Ctrl+B / Ctrl+D report work in
//! progress / none in the status title; Ctrl+E reports work in progress
//! that ends by itself [`WORK_LASTS`] later.
//!
//! As a fake `ssh` for SSH tabs (`GAVIERO_SSH` pointing at this binary):
//! `GAVIERO_PROBE_ARGS` names a file that receives its arguments, one per
//! line; Ctrl+K exits with 255 (a lost connection), Ctrl+O reports the
//! folder `/srv/my proj` (OSC 7) and Ctrl+Y copies `probe-copy` (OSC 52).

use crossterm::event::{Event, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use std::io::Write;
use std::time::Duration;

/// How long the work Ctrl+E reports lasts.
const WORK_LASTS: Duration = Duration::from_secs(2);

pub(crate) fn run_probe() -> anyhow::Result<()> {
    if let Some(file) = std::env::var_os("GAVIERO_PROBE_ARGS") {
        let args: Vec<String> = std::env::args().skip(1).collect();
        std::fs::write(file, args.join("\n"))?;
    }
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    crate::platform::enable_vt_mouse_passthrough(&mut stdout)?;
    stdout.write_all(b"\x1b]2;probe-ready\x07")?;
    stdout.flush()?;

    let result = probe_loop(&mut stdout);
    let restored = crate::restore_terminal();
    result?;
    restored?;
    Ok(())
}

fn probe_loop(stdout: &mut std::io::Stdout) -> anyhow::Result<()> {
    loop {
        let line = match crossterm::event::read()? {
            Event::Key(k) => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                if k.code == KeyCode::Char('q') && ctrl {
                    return Ok(());
                }
                // Ctrl+B / Ctrl+D: report work in progress / none, the way
                // a session reports it to the host (its status title).
                if ctrl && k.code == KeyCode::Char('b') {
                    stdout.write_all(b"\x1b]2;gaviero-session:busy:agent,terminal\x07")?;
                    "STATUS busy".to_string()
                } else if ctrl && k.code == KeyCode::Char('e') && k.kind == KeyEventKind::Press {
                    // Press only: one timer per tap.
                    stdout.write_all(b"\x1b]2;gaviero-session:busy:agent,terminal\x07")?;
                    std::thread::spawn(|| {
                        std::thread::sleep(WORK_LASTS);
                        let mut out = std::io::stdout().lock();
                        let _ = out.write_all(
                            b"\x1b]2;gaviero-session:idle\x07STATUS idle (later)\r\n",
                        );
                        let _ = out.flush();
                    });
                    "STATUS busy".to_string()
                } else if ctrl && k.code == KeyCode::Char('k') {
                    // A dropped SSH connection: ssh exits with 255.
                    let _ = crate::restore_terminal();
                    std::process::exit(255);
                } else if ctrl && k.code == KeyCode::Char('o') {
                    stdout.write_all(b"\x1b]7;file:///srv/my%20proj\x07")?;
                    "STATUS osc7".to_string()
                } else if ctrl && k.code == KeyCode::Char('y') {
                    use base64::Engine;
                    let data = base64::engine::general_purpose::STANDARD.encode("probe-copy");
                    write!(stdout, "\x1b]52;c;{data}\x07")?;
                    "STATUS osc52".to_string()
                } else if ctrl && k.code == KeyCode::Char('d') {
                    stdout.write_all(b"\x1b]2;gaviero-session:idle\x07")?;
                    "STATUS idle".to_string()
                } else {
                    format!("KEY {:?} {:?} {:?}", k.code, k.modifiers, k.kind)
                }
            }
            Event::Mouse(m) => {
                format!("MOUSE {:?} {} {} {:?}", m.kind, m.column, m.row, m.modifiers)
            }
            Event::Resize(cols, rows) => format!("RESIZE {cols} {rows}"),
            Event::Paste(text) => format!("PASTE {text:?}"),
            Event::FocusGained | Event::FocusLost => continue,
        };
        stdout.write_all(line.as_bytes())?;
        stdout.write_all(b"\r\n")?;
        stdout.flush()?;
    }
}
