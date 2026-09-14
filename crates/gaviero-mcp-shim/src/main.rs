//! gaviero-mcp-shim (Tier A / A5).
//!
//! A tiny stdio↔socket bridge. Subprocess coding agents (Claude
//! Code, Codex) spawn this binary as their MCP "server"; all it does
//! is open a connection to Gaviero's workspace endpoint and pipe
//! bytes in both directions. Gaviero's in-process rmcp server on the
//! other end handles the actual MCP protocol.
//!
//! The endpoint is a Unix domain socket on Unix (`--socket <path>`)
//! and a named pipe on Windows (`--pipe <name>`, Tier W1 / PR-1).
//!
//! Decoupling the shim from Gaviero itself has three benefits:
//! * subprocess agents don't have to know about Gaviero's internals;
//! * Gaviero restarts don't require the subprocess to restart — the
//!   shim retries the connect with a short backoff;
//! * the shim binary is a few KB and pure-stdlib-ish, so
//!   `.mcp.json`'s `command` field resolves cleanly everywhere.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Parser)]
#[command(
    name = "gaviero-mcp-shim",
    about = "stdio↔socket bridge for Gaviero's MCP server"
)]
struct Cli {
    /// Absolute path to the workspace MCP Unix socket
    /// (`<workspace>/.gaviero/mcp.sock`). Unix only.
    #[arg(long, conflicts_with = "pipe")]
    socket: Option<PathBuf>,

    /// Windows named-pipe name (`\\.\pipe\gaviero-…`). Windows only.
    #[arg(long, conflicts_with = "socket")]
    pipe: Option<String>,

    /// Walk up from cwd for `.gaviero/mcp-endpoint.json` and connect
    /// to that endpoint. Mutually exclusive with `--socket` / `--pipe`.
    /// Exits 2 immediately when the file is missing or its `pid` is not
    /// alive (no connect retry).
    #[arg(long, conflicts_with_all = ["socket", "pipe"])]
    resolve: bool,

    /// Seconds to retry the initial connect. Useful when the
    /// subprocess agent spawns before Gaviero has finished `Workspace::open`.
    /// Ignored for `--resolve` when the descriptor is missing or dead.
    #[arg(long, default_value = "5")]
    connect_timeout_secs: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .init();

    run(cli).await
}

#[cfg(unix)]
async fn run(cli: Cli) -> Result<()> {
    if cli.pipe.is_some() {
        anyhow::bail!("gaviero-mcp-shim: --pipe is Windows-only; use --socket on this platform");
    }
    let socket = if cli.resolve {
        resolve::socket_from_descriptor()?
    } else {
        cli.socket
            .context("gaviero-mcp-shim: --socket <path> is required on this platform")?
    };
    let stream = unix::connect_with_backoff(&socket, cli.connect_timeout_secs).await?;
    let (rx, tx) = stream.into_split();
    bridge(rx, tx).await
}

#[cfg(windows)]
async fn run(cli: Cli) -> Result<()> {
    if cli.socket.is_some() {
        anyhow::bail!(
            "gaviero-mcp-shim: --socket is Unix-only; use --pipe <name> on Windows"
        );
    }
    let pipe = if cli.resolve {
        resolve::pipe_from_descriptor()?
    } else {
        cli.pipe
            .context("gaviero-mcp-shim: --pipe <name> is required on Windows")?
    };
    let client = windows::connect_with_backoff(&pipe, cli.connect_timeout_secs).await?;
    let (rx, tx) = tokio::io::split(client);
    bridge(rx, tx).await
}

#[cfg(not(any(unix, windows)))]
async fn run(_cli: Cli) -> Result<()> {
    anyhow::bail!("gaviero-mcp-shim: unsupported platform")
}

/// Bidirectional pipe: stdin→endpoint, endpoint→stdout. Exits when
/// either side closes. MCP over stdio is line-delimited JSON-RPC 2.0 —
/// the byte-faithful copy loops are what rmcp expects.
async fn bridge<R, W>(mut endpoint_rx: R, mut endpoint_tx: W) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let to_endpoint = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = stdin.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            endpoint_tx.write_all(&buf[..n]).await?;
            endpoint_tx.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    };
    let from_endpoint = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = endpoint_rx.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            stdout.write_all(&buf[..n]).await?;
            stdout.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    };

    tokio::select! {
        r = to_endpoint => r.context("piping stdin → endpoint")?,
        r = from_endpoint => r.context("piping endpoint → stdout")?,
    }
    Ok(())
}

/// Shared connect-retry pacing: exponential backoff 50 ms → 400 ms
/// until the deadline, so the shim survives Gaviero restarting
/// `Workspace::open` after the subprocess is already spawned.
async fn backoff_or_fail(
    deadline: Instant,
    backoff: &mut Duration,
    err: std::io::Error,
    what: &str,
    timeout_secs: u64,
) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(err).with_context(|| format!("connecting to {what} after {timeout_secs}s"));
    }
    tokio::time::sleep(*backoff).await;
    *backoff = (*backoff * 2).min(Duration::from_millis(400));
    Ok(())
}

#[cfg(unix)]
mod unix {
    use super::*;
    use tokio::net::UnixStream;

    pub(crate) async fn connect_with_backoff(
        path: &std::path::Path,
        timeout_secs: u64,
    ) -> Result<UnixStream> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut backoff = Duration::from_millis(50);
        loop {
            match UnixStream::connect(path).await {
                Ok(s) => return Ok(s),
                Err(e) => {
                    backoff_or_fail(
                        deadline,
                        &mut backoff,
                        e,
                        &path.display().to_string(),
                        timeout_secs,
                    )
                    .await?;
                }
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    pub(crate) async fn connect_with_backoff(
        name: &str,
        timeout_secs: u64,
    ) -> Result<NamedPipeClient> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut backoff = Duration::from_millis(50);
        loop {
            // Retry every failure until the deadline, matching the Unix
            // arm: NotFound = server not up yet, `ERROR_PIPE_BUSY` (231)
            // = all instances mid-handshake — both transient during
            // Gaviero startup.
            match ClientOptions::new().open(name) {
                Ok(c) => return Ok(c),
                Err(e) => {
                    backoff_or_fail(deadline, &mut backoff, e, name, timeout_secs).await?;
                }
            }
        }
    }
}

/// Cwd-walk `--resolve` (no gaviero-core: parse the JSON ourselves).
mod resolve {
    use super::*;
    use std::path::{Path, PathBuf};

    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // `socket` is Unix-only; `pipe` is Windows-only.
    struct Descriptor {
        pid: u32,
        #[serde(default)]
        pipe: Option<String>,
        #[serde(default)]
        socket: Option<PathBuf>,
    }

    fn find_descriptor_upwards(cwd: &Path) -> Option<PathBuf> {
        let mut cur = cwd.to_path_buf();
        for _ in 0..64 {
            let candidate = cur.join(".gaviero").join("mcp-endpoint.json");
            if candidate.is_file() {
                return Some(candidate);
            }
            if !cur.pop() {
                break;
            }
        }
        None
    }

    fn load_live_descriptor() -> Descriptor {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let Some(path) = find_descriptor_upwards(&cwd) else {
            tracing::warn!("gaviero-mcp-shim: no .gaviero/mcp-endpoint.json above {cwd:?}");
            std::process::exit(2);
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("gaviero-mcp-shim: reading {}: {e}", path.display());
                std::process::exit(2);
            }
        };
        let desc: Descriptor = match serde_json::from_str(&text) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("gaviero-mcp-shim: parsing {}: {e}", path.display());
                std::process::exit(2);
            }
        };
        if !pid_alive(desc.pid) {
            if !transport_live(&desc) {
                tracing::warn!(
                    "gaviero-mcp-shim: descriptor pid {} is not alive ({})",
                    desc.pid,
                    path.display()
                );
                std::process::exit(2);
            }
            tracing::warn!(
                "gaviero-mcp-shim: descriptor pid {} is not alive but the endpoint still accepts a connect ({})",
                desc.pid,
                path.display()
            );
        }
        desc
    }

    #[cfg(unix)]
    pub(crate) fn socket_from_descriptor() -> Result<PathBuf> {
        let desc = load_live_descriptor();
        desc.socket.context("mcp-endpoint.json has no socket field")
    }

    #[cfg(windows)]
    pub(crate) fn pipe_from_descriptor() -> Result<String> {
        let desc = load_live_descriptor();
        desc.pipe.context("mcp-endpoint.json has no pipe field")
    }

    #[cfg(windows)]
    fn transport_live(desc: &Descriptor) -> bool {
        let Some(name) = desc.pipe.as_deref() else {
            return false;
        };
        const ERROR_PIPE_BUSY: i32 = 231;
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Ok(_) => true,
            Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
        }
    }

    #[cfg(unix)]
    fn transport_live(desc: &Descriptor) -> bool {
        let Some(socket) = desc.socket.as_ref() else {
            return false;
        };
        std::os::unix::net::UnixStream::connect(socket).is_ok()
    }

    #[cfg(not(any(unix, windows)))]
    fn transport_live(_desc: &Descriptor) -> bool {
        false
    }

    #[cfg(unix)]
    fn pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let rc = unsafe { kill(pid as i32, 0) };
        if rc == 0 {
            return true;
        }
        // EPERM: process exists but we cannot signal it — still alive.
        std::io::Error::last_os_error().raw_os_error() == Some(1)
    }

    #[cfg(unix)]
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    #[cfg(windows)]
    fn pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE
        }
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        fn GetExitCodeProcess(handle: *mut std::ffi::c_void, exit_code: *mut u32) -> i32;
    }
}
