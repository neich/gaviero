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
            use sha2::{Digest, Sha256};
            // Canonicalize so `C:\ws`, `C:/ws/.` and relative spellings
            // hash identically; fall back to the raw path when the root
            // doesn't exist yet (tests, races at startup).
            let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            let digest = Sha256::digest(canon.to_string_lossy().as_bytes());
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            McpEndpoint::Pipe(format!(r"\\.\pipe\gaviero-{}", &hex[..16]))
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
        assert_eq!(ep, McpEndpoint::Unix(PathBuf::from("/ws/.gaviero/mcp.sock")));
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
