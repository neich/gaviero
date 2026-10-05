//! Going back: restore a file (or some of its hunks) to its pre-turn state.
//!
//! Restores are byte-exact — the stored pre-turn blob is written back as is —
//! and every write is tmp-file + rename and recorded in the
//! [`HostWriteLedger`](super::HostWriteLedger), so a concurrent turn's end
//! scan does not attribute the revert to its agent.
//!
//! A revert refuses to overwrite a file that changed after the turn ended
//! ([`RevertOutcome::Drifted`]) unless the caller passes `force` — the user's
//! confirmation that later edits may be lost.

use std::path::Path;

use anyhow::{Context, Result};

use crate::diff_engine::compute_hunks;
use crate::types::DiffHunk;

use super::changeset::FileChange;
use super::store::{atomic_write, hash_file};
use super::TurnCapture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertOutcome {
    Reverted,
    /// The file already holds its pre-turn state.
    AlreadyAtBefore,
    /// The file changed after the turn ended; not reverted (pass `force`).
    Drifted,
    NotRevertible(String),
}

fn current_hash(path: &Path) -> Result<Option<String>> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => hash_file(path).map(Some),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("stat of {}", path.display())),
    }
}

/// The file no longer holds what the turn left (edited since, or reverted).
/// Unknown hashes (stat-only tracking) count as not drifted.
pub fn has_drifted(change: &FileChange) -> bool {
    let Ok(current) = current_hash(&change.path) else {
        return true;
    };
    match &change.after {
        None => current.is_some(),
        Some(after) => match &after.sha256 {
            Some(h) => current.as_deref() != Some(h.as_str()),
            None => false,
        },
    }
}

/// Restore the whole file to its pre-turn state (delete it if it was added).
pub fn revert_file(cap: &TurnCapture, change: &FileChange, force: bool) -> Result<RevertOutcome> {
    let current = current_hash(&change.path)?;
    let at_before = match &change.before {
        None => current.is_none(),
        Some(b) => b.sha256.is_some() && current.as_deref() == b.sha256.as_deref(),
    };
    if at_before {
        return Ok(RevertOutcome::AlreadyAtBefore);
    }
    if !change.revertible {
        return Ok(RevertOutcome::NotRevertible(
            "pre-turn content was not stored (file over 8 MiB or past the baseline cap)".into(),
        ));
    }
    if !force && has_drifted(change) {
        return Ok(RevertOutcome::Drifted);
    }
    match &change.before {
        None => {
            match std::fs::remove_file(&change.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e).with_context(|| format!("removing {}", change.path.display()));
                }
            }
            cap.ledger().record(&change.path, None);
        }
        Some(b) => {
            let hash = b.sha256.as_deref().expect("revertible implies a hash");
            let bytes = cap.store().get(hash)?;
            write_back(cap, &change.path, &bytes)?;
        }
    }
    Ok(RevertOutcome::Reverted)
}

fn write_back(cap: &TurnCapture, path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    atomic_write(path, bytes)?;
    cap.ledger().record(path, Some(bytes));
    Ok(())
}

fn stored_text(cap: &TurnCapture, side: Option<&super::BlobRef>) -> Option<String> {
    match side {
        None => Some(String::new()),
        Some(b) if b.stored => {
            let bytes = cap.store().get(b.sha256.as_deref()?).ok()?;
            String::from_utf8(bytes).ok()
        }
        Some(_) => None,
    }
}

/// Hunks from pre-turn to post-turn content, or `None` when the change only
/// supports whole-file decisions (binary, deleted, or content not stored).
pub fn file_hunks(cap: &TurnCapture, change: &FileChange) -> Option<Vec<DiffHunk>> {
    if change.binary || change.after.is_none() {
        return None;
    }
    let before = stored_text(cap, change.before.as_ref())?;
    let after = stored_text(cap, change.after.as_ref())?;
    Some(compute_hunks(&before, &after))
}

/// Revert only the hunks at `revert` (indices into [`file_hunks`]).
pub fn revert_hunks(
    cap: &TurnCapture,
    change: &FileChange,
    revert: &[usize],
    force: bool,
) -> Result<RevertOutcome> {
    let Some(hunks) = file_hunks(cap, change) else {
        return Ok(RevertOutcome::NotRevertible(
            "per-hunk revert needs stored text on both sides".into(),
        ));
    };
    if hunks.is_empty() || (0..hunks.len()).all(|i| revert.contains(&i)) {
        return revert_file(cap, change, force);
    }
    if !force && has_drifted(change) {
        return Ok(RevertOutcome::Drifted);
    }
    let after = stored_text(cap, change.after.as_ref()).expect("file_hunks checked");
    let content = splice(&after, &hunks, revert);
    write_back(cap, &change.path, content.as_bytes())?;
    Ok(RevertOutcome::Reverted)
}

/// Rebuild `after` with the hunks at `revert` replaced by their pre-turn text.
/// Works on `\n`-inclusive lines, so bytes outside reverted hunks are exact.
fn splice(after: &str, hunks: &[DiffHunk], revert: &[usize]) -> String {
    let lines: Vec<&str> = after.split_inclusive('\n').collect();
    let mut out = String::with_capacity(after.len());
    let mut cursor = 0;
    for (i, h) in hunks.iter().enumerate() {
        let (start, end) = h.proposed_range;
        for line in &lines[cursor.min(lines.len())..start.min(lines.len())] {
            out.push_str(line);
        }
        if revert.contains(&i) {
            out.push_str(&h.original_text);
        } else {
            for line in &lines[start.min(lines.len())..end.min(lines.len())] {
                out.push_str(line);
            }
        }
        cursor = end;
    }
    for line in &lines[cursor.min(lines.len())..] {
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_reverts_selected_hunks_exactly() {
        let before = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
        let after = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl";
        let hunks = compute_hunks(before, after);
        assert_eq!(hunks.len(), 2, "{hunks:?}");
        assert_eq!(splice(after, &hunks, &[]), after);
        assert_eq!(splice(after, &hunks, &[0]), "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl");
        assert_eq!(splice(after, &hunks, &[1]), "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n");
        assert_eq!(splice(after, &hunks, &[0, 1]), before);
    }
}
