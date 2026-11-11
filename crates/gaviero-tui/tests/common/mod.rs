//! Shared helpers for the session-host integration tests: run the `gaviero`
//! binary in a real pseudo console and read its screen through `vt100`.

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ROWS: u16 = 30;
pub const COLS: u16 = 100;
const WAIT: Duration = Duration::from_secs(15);

/// Keeps the last window title (OSC 0 / OSC 2) and the last folder report
/// (OSC 7 URL) the child sent.
#[derive(Default)]
pub struct TitleCb {
    pub title: Option<String>,
    pub osc7: Option<String>,
}

impl vt100::Callbacks for TitleCb {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() == Some(&&b"7"[..]) {
            self.osc7 = Some(String::from_utf8_lossy(&params[1..].join(&b";"[..])).into_owned());
        }
    }
}

pub struct Pty {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Arc<Mutex<vt100::Parser<TitleCb>>>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

impl Pty {
    /// Start the `gaviero` binary with `args` and extra `envs` in a
    /// `ROWS` × `COLS` pseudo console.
    pub fn spawn(args: &[&OsStr], envs: &[(&str, &OsStr)], cwd: &Path) -> Pty {
        // Never let the binary under test become a supervisor.
        let mut envs = envs.to_vec();
        envs.push(("GAVIERO_NO_SUPERVISOR", OsStr::new("1")));
        Pty::spawn_exe(Path::new(env!("CARGO_BIN_EXE_gaviero")), args, &envs, cwd)
    }

    /// Start `exe` (a copy of the binary, say) as is: it supervises itself
    /// unless `envs` opt out.
    pub fn spawn_exe(exe: &Path, args: &[&OsStr], envs: &[(&str, &OsStr)], cwd: &Path) -> Pty {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");
        let mut cmd = CommandBuilder::new(exe);
        for arg in args {
            cmd.arg(arg);
        }
        for (key, value) in envs {
            cmd.env(key, value);
        }
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd).expect("spawn gaviero");
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            ROWS,
            COLS,
            0,
            TitleCb::default(),
        )));
        let mut reader = pair.master.try_clone_reader().expect("pty reader");
        let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("pty writer")));
        let feed = Arc::clone(&parser);
        let answer = Arc::clone(&writer);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                let cursor = {
                    let mut parser = feed.lock().unwrap();
                    parser.process(chunk);
                    parser.screen().cursor_position()
                };
                // We are the terminal: ConPTY itself asks for the cursor
                // position (DSR) at startup and waits for the answer.
                let dsr = chunk.windows(4).filter(|w| *w == b"\x1b[6n").count();
                if dsr > 0 {
                    let report = format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1);
                    let mut w = answer.lock().unwrap();
                    for _ in 0..dsr {
                        let _ = w.write_all(report.as_bytes());
                    }
                    let _ = w.flush();
                }
            }
        });
        Pty {
            writer,
            parser,
            child,
            _master: pair.master,
        }
    }

    pub fn send(&mut self, bytes: &str) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes.as_bytes()).expect("write pty");
        w.flush().expect("flush pty");
    }

    /// The child's process id, while known.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// The last OSC 7 URL the child sent.
    pub fn osc7(&self) -> Option<String> {
        self.parser.lock().unwrap().callbacks().osc7.clone()
    }

    pub fn contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Text of screen row `row` (0-based), trailing blanks trimmed.
    pub fn row(&self, row: u16) -> String {
        let parser = self.parser.lock().unwrap();
        parser
            .screen()
            .rows(0, COLS)
            .nth(row as usize)
            .unwrap_or_default()
            .trim_end()
            .to_string()
    }

    /// Symbol and foreground colour of the cell at `row`, `col`.
    pub fn cell(&self, row: u16, col: u16) -> (String, vt100::Color) {
        let parser = self.parser.lock().unwrap();
        parser
            .screen()
            .cell(row, col)
            .map(|c| (c.contents().to_string(), c.fgcolor()))
            .unwrap_or_else(|| (String::new(), vt100::Color::Default))
    }

    /// Poll every 50 ms until `pred(screen contents, title)` holds; panic
    /// with the screen after 15 s.
    pub fn wait_for(&self, what: &str, pred: impl Fn(&str, Option<&str>) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            // Snapshot, then release the lock: `pred` may read the screen again.
            let (contents, title) = {
                let parser = self.parser.lock().unwrap();
                (parser.screen().contents(), parser.callbacks().title.clone())
            };
            if pred(&contents, title.as_deref()) {
                return;
            }
            if Instant::now() > deadline {
                let (title, contents) = {
                    let parser = self.parser.lock().unwrap();
                    (
                        parser.callbacks().title.clone(),
                        parser.screen().contents(),
                    )
                };
                panic!("timed out waiting for {what}; title {title:?}; screen:\n{contents}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait until the screen contains `needle`.
    pub fn wait_text(&self, needle: &str) {
        self.wait_for(needle, |s, _| s.contains(needle));
    }

    /// [`Pty::wait_text`] with its own time limit.
    pub fn wait_text_within(&self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.contents().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {needle}; screen:\n{}", self.contents());
    }

    /// Wait up to `timeout` for the child to exit; its exit code.
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status.exit_code());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
    }
}
