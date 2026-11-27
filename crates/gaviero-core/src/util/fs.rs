//! Filesystem helpers (Tier W1 / PR-6).

use std::path::Path;
use std::time::Duration;

/// `std::fs::remove_dir_all` with a Windows retry.
///
/// Deleting a tree that contained a just-closed SQLite DB (per-folder
/// `.gaviero/memory.db` in worktrees) or a just-killed child's cwd can
/// hit `ERROR_SHARING_VIOLATION` while the other handle lingers. Retry
/// 3 times with 100/300/900 ms backoff, logging every retry at `warn`
/// so genuine handle leaks stay visible. Unix fails immediately — the
/// races this masks don't exist there.
pub fn remove_dir_all_retry(path: &Path) -> std::io::Result<()> {
    let first = match std::fs::remove_dir_all(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => e,
    };
    if !cfg!(windows) {
        return Err(first);
    }
    let mut last = first;
    for delay_ms in [100u64, 300, 900] {
        tracing::warn!(
            target: "util_fs",
            path = %path.display(),
            error = %last,
            retry_in_ms = delay_ms,
            "remove_dir_all failed — retrying (lingering handle?)"
        );
        std::thread::sleep(Duration::from_millis(delay_ms));
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Give `replacement` the permission bits of the file it is about to replace.
///
/// A save through a temp file + rename swaps in a new inode created with the
/// creator's default mode (`File::create`: `0666 & !umask`; `tempfile`:
/// `0600`), so without this every save drops a script's executable bit and
/// changes who may read the file. Call it after writing `replacement` and
/// before the rename. A missing `target` (a new file) leaves `replacement` as
/// created. Unix only: Windows has no mode bits, and copying its read-only
/// attribute would make the replacing rename itself fail.
pub fn copy_permissions_from_target(target: &Path, replacement: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        match std::fs::metadata(target) {
            Ok(meta) => std::fs::set_permissions(replacement, meta.permissions()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (target, replacement);
        Ok(())
    }
}

/// A file's Unix permission bits (`0o7777`: rwx for all three classes plus
/// setuid/setgid/sticky), for recording and restoring later with
/// [`set_mode`]. `None` off Unix.
pub fn file_mode(meta: &std::fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

/// Set a file's Unix permission bits to `mode` (as read by [`file_mode`]).
/// No-op off Unix.
pub fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Strip Windows' verbatim prefix from a canonicalized path:
/// `\\?\C:\ws` → `C:\ws`, `\\?\UNC\srv\share` → `\\srv\share`.
///
/// `std::fs::canonicalize` returns verbatim (`\\?\`) paths on Windows.
/// Handing those to child processes breaks user-facing rendering —
/// pwsh shows a `Microsoft.PowerShell.Core\FileSystem::\\?\…`
/// provider-qualified prompt — and some tools reject them outright.
/// Paths at/over the classic MAX_PATH limit keep the prefix (they need
/// it). Identity on non-Windows.
/// `std::fs::canonicalize` followed by [`simplify_path`] — the only way the
/// workspace should canonicalize a path. A bare `canonicalize` yields a
/// verbatim `\\?\` path on Windows whose prefix components never
/// `strip_prefix`-match against a simplified workspace root.
pub fn canonicalize_simplified(p: &Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(p).map(|p| simplify_path(&p))
}

pub fn simplify_path(p: &Path) -> std::path::PathBuf {
    if !cfg!(windows) {
        return p.to_path_buf();
    }
    const MAX_PATH: usize = 260;
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        let unc = format!(r"\\{rest}");
        if unc.len() < MAX_PATH {
            return std::path::PathBuf::from(unc);
        }
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && rest.len() < MAX_PATH {
            return std::path::PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_existing_tree() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("tree");
        std::fs::create_dir_all(target.join("nested")).unwrap();
        std::fs::write(target.join("nested/file.txt"), "x").unwrap();
        remove_dir_all_retry(&target).unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn missing_path_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        remove_dir_all_retry(&dir.path().join("nope")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn copies_mode_bits_onto_the_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run.sh");
        let replacement = dir.path().join("tmp");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::write(&replacement, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();

        copy_permissions_from_target(&target, &replacement).unwrap();

        let mode = std::fs::metadata(&replacement).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o750);
    }

    #[test]
    fn missing_target_leaves_the_replacement_alone() {
        let dir = tempfile::tempdir().unwrap();
        let replacement = dir.path().join("tmp");
        std::fs::write(&replacement, "x").unwrap();
        let before = std::fs::metadata(&replacement).unwrap().permissions();
        copy_permissions_from_target(&dir.path().join("new.sh"), &replacement).unwrap();
        assert_eq!(std::fs::metadata(&replacement).unwrap().permissions(), before);
    }

    #[cfg(windows)]
    #[test]
    fn simplify_strips_verbatim_drive_prefix() {
        assert_eq!(
            simplify_path(Path::new(r"\\?\C:\Users\dev\ws")),
            std::path::PathBuf::from(r"C:\Users\dev\ws")
        );
        // UNC verbatim → plain UNC.
        assert_eq!(
            simplify_path(Path::new(r"\\?\UNC\server\share\dir")),
            std::path::PathBuf::from(r"\\server\share\dir")
        );
        // Non-verbatim paths pass through.
        assert_eq!(
            simplify_path(Path::new(r"C:\plain")),
            std::path::PathBuf::from(r"C:\plain")
        );
        // Over-MAX_PATH keeps the verbatim prefix (still needed).
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(
            simplify_path(Path::new(&long)),
            std::path::PathBuf::from(&long)
        );
    }

    #[cfg(windows)]
    #[test]
    fn simplify_round_trips_canonicalize() {
        let dir = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(dir.path()).unwrap();
        let simplified = simplify_path(&canon);
        assert!(
            !simplified.to_string_lossy().starts_with(r"\\?\"),
            "got {}",
            simplified.display()
        );
        assert!(simplified.exists());
    }
}
