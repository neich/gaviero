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

    /// Seconds to retry the initial connect. Useful when the
    /// subprocess agent spawns before Gaviero has finished `Workspace::open`.
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
    let socket = cli
        .socket
        .context("gaviero-mcp-shim: --socket <path> is required on this platform")?;
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
    let pipe = cli
        .pipe
        .context("gaviero-mcp-shim: --pipe <name> is required on Windows")?;
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
