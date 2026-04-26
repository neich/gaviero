//! gaviero-mcp-shim (Tier A / A5).
//!
//! A tiny stdio↔Unix-socket bridge. Subprocess coding agents (Claude
//! Code, Codex) spawn this binary as their MCP "server"; all it does
//! is open a connection to Gaviero's workspace socket and pipe bytes
//! in both directions. Gaviero's in-process rmcp server on the other
//! end handles the actual MCP protocol.
//!
//! Decoupling the shim from Gaviero itself has three benefits:
//! * subprocess agents don't have to know about Gaviero's internals;
//! * Gaviero restarts don't require the subprocess to restart — the
//!   shim retries the socket connect with a short backoff;
//! * the shim binary is a few KB and pure-stdlib-ish, so
//!   `.mcp.json`'s `command` field resolves cleanly everywhere.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(
    name = "gaviero-mcp-shim",
    about = "stdio↔socket bridge for Gaviero's MCP server"
)]
struct Cli {
    /// Absolute path to the workspace MCP socket
    /// (`<workspace>/.gaviero/mcp.sock`).
    #[arg(long)]
    socket: PathBuf,

    /// Seconds to retry the initial socket connect. Useful when the
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

    let stream = connect_with_backoff(&cli.socket, cli.connect_timeout_secs).await?;
    bridge(stream).await
}

/// Connect to the Unix socket, retrying briefly in case Gaviero is
/// still starting up.
async fn connect_with_backoff(path: &std::path::Path, timeout_secs: u64) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut backoff = std::time::Duration::from_millis(50);
    loop {
        match UnixStream::connect(path).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(e).with_context(|| {
                        format!("connecting to {} after {}s", path.display(), timeout_secs)
                    });
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(std::time::Duration::from_millis(400));
            }
        }
    }
}

/// Bidirectional pipe: stdin→socket, socket→stdout. Exits when either
/// side closes. MCP over stdio is line-delimited JSON-RPC 2.0 —
/// `tokio::io::copy` is byte-faithful which is what rmcp expects.
async fn bridge(stream: UnixStream) -> Result<()> {
    let (mut sock_rx, mut sock_tx) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let to_sock = async move {
        let mut buf = [0u8; 8192];
        loop {
            let n = stdin.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            sock_tx.write_all(&buf[..n]).await?;
            sock_tx.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    };
    let from_sock = async move {
        let mut buf = [0u8; 8192];
        loop {
            let n = sock_rx.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            stdout.write_all(&buf[..n]).await?;
            stdout.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    };

    tokio::select! {
        r = to_sock => r.context("piping stdin → socket")?,
        r = from_sock => r.context("piping socket → stdout")?,
    }
    Ok(())
}
