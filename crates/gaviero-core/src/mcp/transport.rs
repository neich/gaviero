//! MCP transport endpoint (Tier W1 / PR-1).
//!
//! The in-process MCP server listens on a Unix domain socket on Unix
//! and a named pipe on Windows. `McpEndpoint` is the platform-neutral
//! description of that listening point, shared by the server accept
//! loop ([`super::server::spawn_mcp_server`]), config synthesis
//! ([`super::config_synth`]), and every host call site that previously
//! built `<workspace>/.gaviero/mcp.sock` by hand.
//!
//! Both variants exist on both platforms so configs and tests are
//! portable; only the accept/connect arms are platform-gated.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where the in-process MCP server listens for shim connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpEndpoint {
    /// Unix domain socket at this filesystem path.
    Unix(PathBuf),
    /// Windows named pipe (`\\.\pipe\gaviero-…`).
    Pipe(String),
}

impl McpEndpoint {
    /// The endpoint for a workspace (or worktree) root.
    ///
    /// Unix: `<root>/.gaviero/mcp.sock`. Windows: a named pipe derived
    /// from the canonicalized root path (W-D1), so distinct
    /// workspaces — and distinct worktrees of the same repo — get
    /// distinct pipes, while every process referring to the same root
    /// agrees on the name.
    pub fn for_workspace(root: &Path) -> Self {
        #[cfg(unix)]
        {
            McpEndpoint::Unix(root.join(".gaviero/mcp.sock"))
        }
        #[cfg(not(unix))]
        {
            // Shared workspace identity (Plan A §3.3): canonicalize so
            // `C:\ws`, `C:/ws/.` and relative spellings hash identically,
            // falling back to the raw path when the root doesn't exist yet
            // (tests, races at startup). The resulting pipe name is
            // byte-identical to the pre-refactor derivation — pinned below.
            let hex = crate::workspace::identity::workspace_id_hex16(root);
            McpEndpoint::Pipe(format!(r"\\.\pipe\gaviero-{hex}"))
        }
    }

    /// The `gaviero-mcp-shim` argv for this endpoint (W-D2: hard flag
    /// split — `--socket <path>` on Unix, `--pipe <name>` on Windows).
    pub fn shim_args(&self) -> [String; 2] {
        match self {
            McpEndpoint::Unix(path) => {
                ["--socket".to_string(), path.to_string_lossy().into_owned()]
            }
            McpEndpoint::Pipe(name) => ["--pipe".to_string(), name.clone()],
        }
    }

    /// Whether a live MCP server is already accepting connections here.
    ///
    /// Lets a host (CLI or a second TUI) reuse the server another
    /// gaviero process — typically the first TUI — runs for the same
    /// workspace instead of trying to rebind: the Windows accept loop
    /// holds the name with `first_pipe_instance(true)`, so a second
    /// [`super::server::spawn_mcp_server`] dies with `Access is denied`,
    /// and the Unix arm would unlink a *live* socket and orphan the other
    /// process's listener. Synthesized agent configs address the
    /// endpoint, not a process, so shims reach whichever host owns it.
    ///
    /// The probe is a transport-level connect closed immediately — no MCP
    /// handshake. A stale Unix socket file (connection refused) reports
    /// `false` so the spawn path can unlink and rebind as before. The
    /// foreign-platform variant (Unix socket on Windows, pipe on Unix)
    /// reports `false`.
    pub fn has_live_server(&self) -> bool {
        match self {
            McpEndpoint::Unix(path) => unix_socket_has_live_server(path),
            McpEndpoint::Pipe(name) => pipe_has_live_server(name),
        }
    }
}

#[cfg(unix)]
fn unix_socket_has_live_server(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn unix_socket_has_live_server(_path: &Path) -> bool {
    false
}

#[cfg(windows)]
fn pipe_has_live_server(name: &str) -> bool {
    // ERROR_PIPE_BUSY: every instance is mid-handshake right now — a
    // server exists even though this connect attempt couldn't land.
    const ERROR_PIPE_BUSY: i32 = 231;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
    {
        Ok(_) => true,
        Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
    }
}

#[cfg(not(windows))]
fn pipe_has_live_server(_name: &str) -> bool {
    false
}

impl fmt::Display for McpEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpEndpoint::Unix(path) => write!(f, "{}", path.display()),
            McpEndpoint::Pipe(name) => f.write_str(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_args_split_by_variant() {
        let unix = McpEndpoint::Unix(PathBuf::from("/ws/.gaviero/mcp.sock"));
        assert_eq!(unix.shim_args()[0], "--socket");
        let pipe = McpEndpoint::Pipe(r"\\.\pipe\gaviero-abc".to_string());
        assert_eq!(
            pipe.shim_args(),
            ["--pipe".to_string(), r"\\.\pipe\gaviero-abc".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn for_workspace_is_socket_under_gaviero_dir() {
        let ep = McpEndpoint::for_workspace(Path::new("/ws"));
        assert_eq!(
            ep,
            McpEndpoint::Unix(PathBuf::from("/ws/.gaviero/mcp.sock"))
        );
    }

    #[test]
    fn has_live_server_is_false_for_foreign_platform_variant() {
        #[cfg(windows)]
        assert!(!McpEndpoint::Unix(PathBuf::from("/tmp/nope.sock")).has_live_server());
        #[cfg(unix)]
        assert!(!McpEndpoint::Pipe(r"\\.\pipe\gaviero-nope".to_string()).has_live_server());
    }

    #[cfg(unix)]
    #[test]
    fn has_live_server_tracks_unix_socket_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("mcp.sock");
        let ep = McpEndpoint::Unix(sock.clone());
        assert!(
            !ep.has_live_server(),
            "missing socket file must probe false"
        );
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert!(ep.has_live_server(), "bound listener must probe true");
        drop(listener);
        // The socket file outlives the listener; a stale file must not
        // count as live or the spawn path would never reclaim it.
        assert!(sock.exists(), "socket file expected to persist after drop");
        assert!(!ep.has_live_server(), "stale socket file must probe false");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn has_live_server_tracks_named_pipe_lifecycle() {
        let name = format!(r"\\.\pipe\gaviero-test-probe-{}", std::process::id());
        let ep = McpEndpoint::Pipe(name.clone());
        assert!(!ep.has_live_server(), "unbound pipe name must probe false");
        let instance = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        assert!(ep.has_live_server(), "pending instance must probe true");
        // A second probe hits the now client-consumed single instance —
        // ERROR_PIPE_BUSY, which still proves a server owns the name.
        assert!(ep.has_live_server(), "busy instance must still probe true");
        drop(instance);
        assert!(
            !ep.has_live_server(),
            "dropping the last instance must free the name"
        );
    }

    /// Pin (Plan A A6): the pipe name must be byte-identical before and
    /// after the identity refactor, or the refactor silently orphans
    /// running shims. Expected value computed with the ORIGINAL inline
    /// algorithm over a nonexistent (fallback-path) root.
    #[cfg(windows)]
    #[test]
    fn for_workspace_pipe_name_is_byte_identical_after_identity_refactor() {
        use sha2::{Digest, Sha256};
        let root = Path::new(r"C:\nonexistent\gaviero-pipe-pin");
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let expected = McpEndpoint::Pipe(format!(r"\\.\pipe\gaviero-{}", &hex[..16]));
        assert_eq!(McpEndpoint::for_workspace(root), expected);
    }

    #[cfg(windows)]
    #[test]
    fn for_workspace_pipe_is_stable_and_per_root() {
        let dir = tempfile::tempdir().unwrap();
        let a = McpEndpoint::for_workspace(dir.path());
        let b = McpEndpoint::for_workspace(dir.path());
        assert_eq!(a, b, "same root must map to the same pipe name");
        let McpEndpoint::Pipe(name) = &a else {
            panic!("windows endpoint must be a pipe");
        };
        assert!(name.starts_with(r"\\.\pipe\gaviero-"), "got {name}");
        let other = tempfile::tempdir().unwrap();
        assert_ne!(
            a,
            McpEndpoint::for_workspace(other.path()),
            "distinct roots must map to distinct pipes"
        );
    }
}
