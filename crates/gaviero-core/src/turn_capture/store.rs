//! Content-addressed blob store for turn capture.
//!
//! Blobs live at `<turns>/objects/<aa>/<sha256-hex>`. Writes are tmp-file +
//! rename so a crash never leaves a truncated blob under a valid name, and
//! `put` is idempotent: the name *is* the content.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Blobs younger than this survive [`BlobStore::gc`] even when unreferenced.
pub const GC_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Content-addressed store rooted at an `objects/` directory.
#[derive(Debug, Clone)]
pub struct BlobStore {
    dir: PathBuf,
}

impl BlobStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, hash: &str) -> PathBuf {
        let (head, _) = hash.split_at(2.min(hash.len()));
        self.dir.join(head).join(hash)
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.path_for(hash).is_file()
    }

    /// Store `bytes`, returning their SHA-256 hex. An existing blob is only
    /// touched, so [`Self::gc`]'s grace window covers it while the caller has
    /// not yet persisted the manifest or review that references it.
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let hash = hash_bytes(bytes);
        let path = self.path_for(&hash);
        if path.is_file() {
            if let Ok(f) = std::fs::File::options().write(true).open(&path) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
            return Ok(hash);
        }
        let parent = path.parent().expect("blob path has a parent");
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        atomic_write(&path, bytes)?;
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.path_for(hash);
        std::fs::read(&path).with_context(|| format!("reading blob {hash}"))
    }

    /// Delete every blob whose hash is not in `keep` and that was not written
    /// or touched within [`GC_GRACE`] — a turn running in another
    /// conversation may hold blobs its manifest does not reference yet.
    /// Returns how many went.
    pub fn gc(&self, keep: &HashSet<String>) -> usize {
        self.gc_with_grace(keep, GC_GRACE)
    }

    pub(crate) fn gc_with_grace(
        &self,
        keep: &HashSet<String>,
        grace: std::time::Duration,
    ) -> usize {
        let mut removed = 0;
        let Ok(shards) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        for shard in shards.flatten() {
            let Ok(entries) = std::fs::read_dir(shard.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if keep.contains(&name) {
                    continue;
                }
                let recent = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_none_or(|age| age < grace);
                if recent {
                    continue;
                }
                if std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Stream-hash a file without loading it whole (used past the blob cap).
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `bytes` to `path` through a sibling temp file and a rename, so a
/// reader never observes a half-written file. A replaced file keeps its
/// permission bits; a new one is private (`0600` on Unix) — right for the
/// capture's own records and blobs, which may hold sensitive file contents.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_via_temp(path, bytes, false, None)
}

/// [`atomic_write`] for a workspace file being restored. Its permission bits
/// become `mode` when known (the pre-turn mode the baseline recorded), else
/// those of the file it replaces, else the usual new-file mode
/// (`0666 & !umask`) — never the private `0600`.
pub fn restore_write(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    write_via_temp(path, bytes, true, mode)
}

fn write_via_temp(path: &Path, bytes: &[u8], public: bool, mode: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent", path.display()))?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".gaviero-tmp-");
    #[cfg(unix)]
    if public {
        use std::os::unix::fs::PermissionsExt;
        // Created through `open(2)`, so the umask applies as for any new file.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    #[cfg(not(unix))]
    let _ = public;
    let mut tmp = builder
        .tempfile_in(parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    std::io::Write::write_all(&mut tmp, bytes)
        .with_context(|| format!("writing temp file for {}", path.display()))?;
    // The rename replaces the inode, so the mode must be set on the new one.
    match mode {
        Some(mode) => crate::util::fs::set_mode(tmp.path(), mode),
        None => crate::util::fs::copy_permissions_from_target(path, tmp.path()),
    }
    .with_context(|| format!("setting permissions of {}", path.display()))?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_gc() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("objects"));
        let a = store.put(b"alpha").unwrap();
        let b = store.put(b"beta").unwrap();
        assert_eq!(store.put(b"alpha").unwrap(), a, "idempotent");
        assert_eq!(store.get(&a).unwrap(), b"alpha");
        let keep: HashSet<String> = [a.clone()].into_iter().collect();
        assert_eq!(
            store.gc(&keep),
            0,
            "fresh blobs are inside the grace window"
        );
        assert_eq!(store.gc_with_grace(&keep, std::time::Duration::ZERO), 1);
        assert!(store.contains(&a));
        assert!(!store.contains(&b));
    }

    #[test]
    fn stream_hash_matches_bytes_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(hash_file(&p).unwrap(), hash_bytes(b"hello world"));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_the_replaced_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("run.sh");
        std::fs::write(&p, "#!/bin/sh\necho agent\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();

        atomic_write(&p, b"#!/bin/sh\n").unwrap();

        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o755);
        assert_eq!(std::fs::read(&p).unwrap(), b"#!/bin/sh\n");
    }
}
