//! Per-turn file snapshots for Option-B writes (DeepSeek plan Unit 10).
//!
//! On the first write to a path in a turn, capture `{ path → Option<bytes> }`
//! where `None` means the file did not exist. `revert` restores/removes paths
//! idempotently — used on cancel, stream error, and when the user rejects an
//! external-change review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Pre-turn on-disk state for paths touched by write tools this turn.
#[derive(Debug, Default)]
pub struct TurnSnapshot {
    /// `None` = file did not exist at first-write time.
    originals: HashMap<PathBuf, Option<String>>,
}

impl TurnSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.originals.is_empty()
    }

    pub fn touched_paths(&self) -> Vec<PathBuf> {
        self.originals.keys().cloned().collect()
    }

    /// Export `(path, pre-turn content)` pairs for the TUI revert path.
    pub fn edits(&self) -> Vec<(PathBuf, Option<String>)> {
        self.originals.iter().map(|(p, c)| (p.clone(), c.clone())).collect()
    }

    /// Record the pre-write state for `path` if not already captured.
    pub async fn capture_before_write(&mut self, path: &Path) -> Result<()> {
        let key = path.to_path_buf();
        if self.originals.contains_key(&key) {
            return Ok(());
        }
        let content = match tokio::fs::read_to_string(path).await {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Err(e).with_context(|| format!("snapshot read of {}", path.display()))?,
        };
        self.originals.insert(key, content);
        Ok(())
    }

    /// Restore every snapshotted path to its pre-turn state.
    pub async fn revert_all(&self) -> Result<()> {
        for (path, original) in &self.originals {
            restore_path(path, original.as_deref()).await?;
        }
        Ok(())
    }

    /// Restore one path. No-op when `path` was not snapshotted.
    pub async fn revert_path(&self, path: &Path) -> Result<()> {
        match self.originals.get(path) {
            Some(original) => restore_path(path, original.as_deref()).await,
            None => Ok(()),
        }
    }
}

async fn restore_path(path: &Path, original: Option<&str>) -> Result<()> {
    match original {
        Some(content) => {
            tokio::fs::write(path, content)
                .await
                .with_context(|| format!("revert write to {}", path.display()))?;
        }
        None => match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("revert unlink {}", path.display()));
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn revert_restores_modified_and_removes_new_file() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("a.txt");
        let created = dir.path().join("b.txt");
        std::fs::write(&existing, "orig\n").unwrap();

        let mut snap = TurnSnapshot::new();
        snap.capture_before_write(&existing).await.unwrap();
        // Snapshot before first write — file does not exist yet.
        snap.capture_before_write(&created).await.unwrap();

        std::fs::write(&existing, "changed\n").unwrap();
        std::fs::write(&created, "new\n").unwrap();
        snap.revert_all().await.unwrap();

        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "orig\n");
        assert!(!created.exists());
    }
}
