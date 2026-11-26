//! MCP transport endpoint (Tier W1 / PR-1).
//!
//! The in-process MCP server listens on a Unix domain socket on Unix
//! and a named pipe on Windows. `McpEndpoint` is the platform-neutral
//! description of that listening point, shared by the server accept
//! loop ([`super::server::spawn_mcp_server`]), config synthesis
//! ([`super::config_synth`]), and every host call site that previously
//! built `<workspace>/.gaviero/mcp.sock` by hand.
//!
//! The Unix socket lives in the per-user runtime directory
//! ([`unix_socket_dir`]), never inside the workspace: file-sync clients
//! (Dropbox, OneDrive, …) cannot sync a socket and keep re-indexing it
//! for as long as the server runs. Only the regular-file descriptor
//! (`.gaviero/mcp-endpoint.json`) stays in the workspace.
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
    /// Both platforms key the endpoint by the canonicalized root path
    /// (W-D1), so distinct workspaces — and distinct worktrees of the
    /// same repo — get distinct endpoints, while every process referring
    /// to the same root agrees on it. Unix: `<unix_socket_dir>/mcp-<hash>.sock`
    /// (outside the workspace — see the module docs). Windows:
    /// `\\.\pipe\gaviero-<hash>`.
    pub fn for_workspace(root: &Path) -> Self {
        // Shared workspace identity (Plan A §3.3): canonicalize so
        // `C:\ws`, `C:/ws/.` and relative spellings hash identically,
        // falling back to the raw path when the root doesn't exist yet
        // (tests, races at startup). The Windows pipe name is
        // byte-identical to the pre-refactor derivation — pinned below.
        let hex = crate::workspace::identity::workspace_id_hex16(root);
        #[cfg(unix)]
        {
            McpEndpoint::Unix(unix_socket_dir().join(format!("mcp-{hex}.sock")))
        }
        #[cfg(not(unix))]
        {
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

/// Per-user directory that holds the MCP Unix sockets:
/// `$XDG_RUNTIME_DIR/gaviero`, else `/run/user/<uid>/gaviero` when that
/// runtime directory exists (so a CLI over SSH without `XDG_RUNTIME_DIR`
/// still meets the desktop TUI's socket), else `<temp dir>/gaviero-<uid>`
/// (macOS: `$TMPDIR` is already per-user). Short on purpose — `sun_path`
/// caps a socket path at 104–108 bytes. Pure: the server's bind creates
/// the directory `0700` and refuses one another user owns.
#[cfg(unix)]
pub fn unix_socket_dir() -> PathBuf {
    if let Some(runtime) = dirs::runtime_dir() {
        return runtime.join("gaviero");
    }
    let uid = current_uid();
    let run_user = PathBuf::from(format!("/run/user/{uid}"));
    if run_user.is_dir() {
        return run_user.join("gaviero");
    }
    std::env::temp_dir().join(format!("gaviero-{uid}"))
}

#[cfg(unix)]
pub(crate) fn current_uid() -> u32 {
    // SAFETY: getuid(2) takes no arguments, cannot fail and touches no memory.
    unsafe { libc::getuid() }
}

/// Create `dir` (`0700`) if missing, then refuse it unless it is a real
/// directory owned by this user and not writable by everyone — in a shared
/// `/tmp` another user could otherwise pre-create it and own our socket.
#[cfg(unix)]
pub(crate) fn prepare_unix_socket_dir(dir: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating MCP socket directory {}", dir.display()))?;
    let meta = std::fs::symlink_metadata(dir)
        .with_context(|| format!("reading MCP socket directory {}", dir.display()))?;
    let uid = current_uid();
    if !meta.is_dir() || meta.uid() != uid {
        anyhow::bail!(
            "MCP socket directory {} is not a directory owned by this user (uid {uid})",
            dir.display()
        );
    }
    if meta.mode() & 0o002 != 0 {
        anyhow::bail!(
            "MCP socket directory {} is writable by every user",
            dir.display()
        );
    }
    Ok(())
}

/// Unlink a dead `<root>/.gaviero/mcp.sock` left by a build that kept the
/// socket inside the workspace (a crash skipped its cleanup). A live one —
/// an older gaviero still serving this root — is left alone.
#[cfg(unix)]
pub(crate) fn remove_stale_legacy_socket(root: &Path) {
    use std::os::unix::fs::FileTypeExt as _;

    let legacy = root.join(".gaviero").join("mcp.sock");
    let Ok(meta) = std::fs::symlink_metadata(&legacy) else {
        return;
    };
    if meta.file_type().is_socket() && !unix_socket_has_live_server(&legacy) {
        let _ = std::fs::remove_file(&legacy);
    }
}

#[cfg(not(unix))]
pub(crate) fn remove_stale_legacy_socket(_root: &Path) {}

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

    /// The socket must never sit inside the workspace: sync clients
    /// (Dropbox) re-index an unsyncable socket for as long as it exists.
    #[cfg(unix)]
    #[test]
    fn for_workspace_socket_is_per_root_and_outside_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let a = McpEndpoint::for_workspace(dir.path());
        assert_eq!(
            a,
            McpEndpoint::for_workspace(dir.path()),
            "same root must map to the same socket"
        );
        let McpEndpoint::Unix(path) = &a else {
            panic!("unix endpoint must be a socket");
        };
        assert!(!path.starts_with(dir.path()), "{}", path.display());
        assert_eq!(path.parent(), Some(unix_socket_dir().as_path()));
        let hex = crate::workspace::identity::workspace_id_hex16(dir.path());
        assert_eq!(
            path.file_name().unwrap().to_str(),
            Some(format!("mcp-{hex}.sock").as_str())
        );
        // sun_path is 104 bytes on macOS, 108 on Linux.
        assert!(path.as_os_str().len() < 104, "{}", path.display());
        let other = tempfile::tempdir().unwrap();
        assert_ne!(
            a,
            McpEndpoint::for_workspace(other.path()),
            "distinct roots must map to distinct sockets"
        );
    }

    /// Linux: with `XDG_RUNTIME_DIR` set, sockets go to
    /// `$XDG_RUNTIME_DIR/gaviero`. Reads the variable, never sets it —
    /// env writes race the other tests in this process.
    #[cfg(target_os = "linux")]
    #[test]
    fn unix_socket_dir_follows_xdg_runtime_dir() {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            assert_eq!(unix_socket_dir(), runtime.join("gaviero"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn prepare_unix_socket_dir_creates_a_private_dir_and_refuses_a_shared_one() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("gaviero-sock");
        prepare_unix_socket_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "mode {mode:o}");
        prepare_unix_socket_dir(&dir).expect("an existing private dir is reused");

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(prepare_unix_socket_dir(&dir).is_err(), "world-writable dir");

        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, "").unwrap();
        assert!(prepare_unix_socket_dir(&file).is_err(), "regular file");
    }

    #[cfg(unix)]
    #[test]
    fn remove_stale_legacy_socket_only_unlinks_a_dead_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let gaviero = tmp.path().join(".gaviero");
        std::fs::create_dir_all(&gaviero).unwrap();
        let legacy = gaviero.join("mcp.sock");

        let listener = std::os::unix::net::UnixListener::bind(&legacy).unwrap();
        remove_stale_legacy_socket(tmp.path());
        assert!(legacy.exists(), "a live legacy socket must survive");

        drop(listener);
        remove_stale_legacy_socket(tmp.path());
        assert!(!legacy.exists(), "a dead legacy socket must be unlinked");

        std::fs::write(&legacy, "not a socket").unwrap();
        remove_stale_legacy_socket(tmp.path());
        assert!(legacy.exists(), "a regular file is never touched");
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
