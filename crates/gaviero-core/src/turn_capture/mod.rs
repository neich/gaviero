//! Turn change capture — a host-side, git-free record of every file a chat
//! turn changed, whatever wrote it (edit tools, Bash, a formatter the agent
//! ran, a provider's own process).
//!
//! The host brackets a turn with [`TurnCapture::begin`] / [`TurnCapture::end`]:
//!
//! 1. `begin` refreshes the **baseline** — every file in the capture scope
//!    with its size, mtime and SHA-256, plus a stored copy of its content
//!    (≤ [`MAX_BLOB_BYTES`]). Only files whose (size, mtime) moved since the
//!    last scan are re-read, so a warm `begin` is a stat walk.
//! 2. The agent edits the real tree freely during the turn.
//! 3. `end` walks again and diffs against the turn's starting baseline,
//!    storing the post-turn content of every changed file. The result is a
//!    [`TurnChangeSet`]: added / modified / deleted, with both sides
//!    addressable in the blob store.
//!
//! A pre-turn copy is the price of being generic: once a shell command
//! overwrites a file, its old content is gone unless it was stored first.
//!
//! The capture never decides anything. Review (keep / revert per file or
//! hunk) is the host's, via [`revert`]; pending reviews persist under
//! `<workspace>/.gaviero/turns/pending/` so a restart cannot skip one.
//!
//! Lock discipline: the inner `Mutex` guards small in-memory state only; every
//! walk, hash and file write happens outside it.

pub mod changeset;
pub mod ledger;
pub mod manifest;
pub mod revert;
pub mod store;
pub mod walk;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

pub use changeset::{
    BlobRef, ChangeKind, FileChange, FileDecision, PendingReview, ResolvedDecision, ReviewedTurn,
    TurnChangeSet, TurnOutcome,
};
pub use ledger::HostWriteLedger;
pub use revert::{RevertOutcome, file_hunks, has_drifted, revert_file, revert_hunks};
pub use walk::CaptureScope;

use manifest::{Entry, Manifest, under_any};
use store::{BlobStore, atomic_write, hash_file};
use walk::Observed;

/// Largest file whose content is stored (and therefore revertible).
pub const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024;
/// First-baseline caps: past either, files are tracked by stat only.
pub const BASELINE_MAX_FILES: usize = 50_000;
pub const BASELINE_MAX_BYTES: u64 = 1 << 30;
/// Reviewed turns whose contents are kept for quick recovery.
pub const RETAINED_TURNS: usize = 5;
/// An mtime this close to the scan cannot prove content is unchanged
/// (coarse timestamps, same-tick rewrites).
const RACY_WINDOW_NS: i64 = 2_000_000_000;

pub(crate) fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Per-workspace capture state, shared by every conversation.
pub struct TurnCapture {
    dir: PathBuf,
    store: BlobStore,
    ledger: HostWriteLedger,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    baseline: Option<Manifest>,
    /// Running turns: id → start.
    active: HashMap<String, i64>,
    /// Ended turns that a still-running turn may overlap.
    recent: Vec<RecentTurn>,
}

struct RecentTurn {
    turn_id: String,
    ended_ns: i64,
    keys: HashSet<String>,
}

/// A turn in progress. Hand it back to [`TurnCapture::end`].
pub struct TurnHandle {
    turn_id: String,
    conv_id: Option<String>,
    scope: CaptureScope,
    pre: Manifest,
    started_ns: i64,
    warnings: Vec<String>,
    /// Root-relative paths that changed since the previous capture, outside
    /// any turn (user edits, `git checkout`, formatters). Logged, not
    /// reviewed. Empty when another turn was running (its writes would be
    /// misattributed).
    pub between_turns: Vec<String>,
}

impl TurnHandle {
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }
}

/// Result of [`TurnCapture::end`].
#[derive(Debug)]
pub struct TurnEnd {
    pub set: TurnChangeSet,
    /// Other turns whose pending review gained an overlap mark.
    pub overlapped_reviews: Vec<String>,
}

#[derive(Default)]
struct Budget {
    files: usize,
    stored_bytes: u64,
    capped: bool,
}

impl TurnCapture {
    /// Capture state at `<root>/.gaviero/turns`.
    pub fn for_workspace(root: &Path) -> Arc<Self> {
        Self::with_dir(root.join(".gaviero").join("turns"))
    }

    pub fn with_dir(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            store: BlobStore::new(dir.join("objects")),
            dir,
            ledger: HostWriteLedger::default(),
            state: Mutex::new(State::default()),
        })
    }

    pub fn store(&self) -> &BlobStore {
        &self.store
    }

    /// Host writes (editor saves, reverts) to exclude from attribution.
    pub fn ledger(&self) -> &HostWriteLedger {
        &self.ledger
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ── Turn bracket ─────────────────────────────────────────────

    /// Refresh the baseline and open a turn. Blocking (walks + hashes): call
    /// from `spawn_blocking`.
    pub fn begin(&self, turn_id: &str, conv_id: Option<&str>, scope: CaptureScope) -> Result<TurnHandle> {
        let started_ns = now_ns();
        let (cached, others_active) = {
            let st = self.lock();
            (st.baseline.clone(), !st.active.is_empty())
        };
        let prev = match cached {
            Some(m) => m,
            None => self.load_baseline(),
        };

        let observed = walk::walk(&scope);
        let mut budget = Budget::default();
        let mut next = Manifest::default();
        let mut between = Vec::new();

        for (path, o) in &observed {
            let old = prev.entries.get(path);
            let entry = match old.filter(|e| self.reusable(e, o)) {
                Some(e) => {
                    let mut e = e.clone();
                    e.root = o.root.clone();
                    e.rel = o.rel.clone();
                    e
                }
                None => match self.capture(o, started_ns, &mut budget) {
                    Some(e) => e,
                    None => continue,
                },
            };
            budget.files += 1;
            if entry.stored {
                budget.stored_bytes += entry.size;
            }
            // A first-ever baseline has nothing to compare against.
            if !prev.entries.is_empty() && old.map(|e| &e.sha256) != Some(&entry.sha256) {
                between.push(entry.rel.clone());
            }
            next.entries.insert(path.clone(), entry);
        }
        for (path, e) in prev.in_scope(&scope.roots) {
            if !observed.contains_key(path) {
                between.push(e.rel.clone());
            }
        }
        // Keep other scopes' entries (another conversation's roots).
        for (path, e) in &prev.entries {
            if !under_any(path, &scope.roots) {
                next.entries.insert(path.clone(), e.clone());
            }
        }

        let mut warnings = Vec::new();
        if budget.capped {
            warnings.push(format!(
                "turn capture baseline capped at {BASELINE_MAX_FILES} files / {} MiB; \
                 files past the cap are tracked without content and cannot be reverted",
                BASELINE_MAX_BYTES >> 20
            ));
        }

        {
            let mut st = self.lock();
            st.baseline = Some(next.clone());
            st.active.insert(turn_id.to_string(), started_ns);
        }
        self.save_baseline(&next);

        Ok(TurnHandle {
            turn_id: turn_id.to_string(),
            conv_id: conv_id.map(str::to_string),
            scope,
            pre: next,
            started_ns,
            warnings,
            between_turns: if others_active { Vec::new() } else { between },
        })
    }

    /// Close a turn and compute what it changed. Blocking: `spawn_blocking`.
    pub fn end(&self, handle: TurnHandle, outcome: TurnOutcome) -> Result<TurnEnd> {
        let TurnHandle {
            turn_id,
            conv_id,
            scope,
            pre,
            started_ns,
            mut warnings,
            ..
        } = handle;
        let scan_ns = now_ns();
        let observed = walk::walk(&scope);
        let mut budget = Budget::default();
        let mut scoped = Manifest::default();
        let mut files = Vec::new();

        for (path, o) in &observed {
            let before = pre.entries.get(path);
            if let Some(e) = before.filter(|e| self.reusable(e, o)) {
                scoped.entries.insert(path.clone(), e.clone());
                budget.files += 1;
                continue;
            }
            let Some(after) = self.capture(o, scan_ns, &mut budget) else {
                continue;
            };
            budget.files += 1;
            if after.stored {
                budget.stored_bytes += after.size;
            }
            match before {
                Some(b) if b.sha256.is_some() && b.sha256 == after.sha256 => {}
                Some(b) => files.push(change(path, ChangeKind::Modified, Some(b), Some(&after))),
                None => files.push(change(path, ChangeKind::Added, None, Some(&after))),
            }
            scoped.entries.insert(path.clone(), after);
        }
        for (path, b) in pre.in_scope(&scope.roots) {
            if !observed.contains_key(path) {
                files.push(change(path, ChangeKind::Deleted, Some(b), None));
            }
        }

        files.retain(|c| !self.ledger.attributes(&c.path, c.after_hash(), started_ns));
        for c in &mut files {
            c.binary = self.is_binary_side(c.before.as_ref()) || self.is_binary_side(c.after.as_ref());
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let keys: HashSet<String> = files.iter().map(|c| c.key()).collect();
        let mut overlapped: Vec<(String, HashSet<String>)> = Vec::new();
        {
            let mut st = self.lock();
            st.active.remove(&turn_id);
            for r in &st.recent {
                if r.ended_ns >= started_ns {
                    let common: HashSet<String> = r.keys.intersection(&keys).cloned().collect();
                    if !common.is_empty() {
                        overlapped.push((r.turn_id.clone(), common));
                    }
                }
            }
            st.recent.push(RecentTurn {
                turn_id: turn_id.clone(),
                ended_ns: scan_ns,
                keys: keys.clone(),
            });
            let horizon = st.active.values().copied().min();
            match horizon {
                Some(h) => st.recent.retain(|r| r.ended_ns >= h),
                None => st.recent.clear(),
            }

            let mut base = st.baseline.take().unwrap_or_default();
            base.entries.retain(|p, _| !under_any(p, &scope.roots));
            base.entries.extend(scoped.entries);
            st.baseline = Some(base);
        }
        let base = self.lock().baseline.clone().unwrap_or_default();
        self.save_baseline(&base);

        for c in &mut files {
            for (other, common) in &overlapped {
                if common.contains(&c.key()) {
                    c.overlap_with.push(other.clone());
                }
            }
        }
        let mut touched = Vec::new();
        for (other, common) in &overlapped {
            if self.mark_overlap(other, &turn_id, common) {
                touched.push(other.clone());
            }
        }

        if budget.capped {
            warnings.push("files past the baseline cap were tracked without content".into());
        }
        Ok(TurnEnd {
            set: TurnChangeSet {
                turn_id,
                conv_id,
                started_at_ms: started_ns / 1_000_000,
                ended_at_ms: scan_ns / 1_000_000,
                outcome,
                files,
                warnings,
                auto_reverted: Vec::new(),
            },
            overlapped_reviews: touched,
        })
    }

    /// Restore every sensitive path in `set` (`.env`, keys, …) to its pre-turn
    /// state and drop it from the review. Exemptions follow
    /// `agent.permissions.sensitivePaths.allow` of the path's own root.
    pub fn auto_revert_sensitive(&self, set: &mut TurnChangeSet) {
        let mut kept = Vec::with_capacity(set.files.len());
        for c in std::mem::take(&mut set.files) {
            let policy = crate::scope_enforcer::SensitivePolicy::resolve(&c.root);
            if policy.refusal(Path::new(&c.rel)).is_none() {
                kept.push(c);
                continue;
            }
            match revert_file(self, &c, true) {
                Ok(RevertOutcome::Reverted | RevertOutcome::AlreadyAtBefore) => {
                    set.warnings
                        .push(format!("{}: sensitive path changed by the agent — reverted", c.rel));
                    set.auto_reverted.push(c.rel.clone());
                }
                Ok(other) => {
                    set.warnings.push(format!(
                        "{}: sensitive path changed by the agent and could not be reverted ({other:?})",
                        c.rel
                    ));
                    kept.push(c);
                }
                Err(e) => {
                    set.warnings.push(format!(
                        "{}: sensitive path changed by the agent; revert failed: {e:#}",
                        c.rel
                    ));
                    kept.push(c);
                }
            }
        }
        set.files = kept;
    }

    fn reusable(&self, e: &Entry, o: &Observed) -> bool {
        e.size == o.size
            && e.mtime_ns == o.mtime_ns
            && !e.racy
            && match (&e.sha256, e.stored) {
                (Some(h), true) => self.store.contains(h),
                _ => true,
            }
    }

    /// Hash (and store, under the caps) one observed file.
    fn capture(&self, o: &Observed, scan_ns: i64, budget: &mut Budget) -> Option<Entry> {
        let racy = o.mtime_ns >= scan_ns - RACY_WINDOW_NS;
        if budget.files >= BASELINE_MAX_FILES || budget.stored_bytes >= BASELINE_MAX_BYTES {
            budget.capped = true;
            return Some(Entry {
                root: o.root.clone(),
                rel: o.rel.clone(),
                size: o.size,
                mtime_ns: o.mtime_ns,
                sha256: None,
                stored: false,
                racy,
            });
        }
        if o.size <= MAX_BLOB_BYTES {
            let bytes = std::fs::read(&o.path).ok()?;
            let hash = match self.store.put(&bytes) {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("turn capture: storing {} failed: {e:#}", o.path.display());
                    return None;
                }
            };
            Some(Entry {
                root: o.root.clone(),
                rel: o.rel.clone(),
                size: bytes.len() as u64,
                mtime_ns: o.mtime_ns,
                sha256: Some(hash),
                stored: true,
                racy,
            })
        } else {
            let hash = hash_file(&o.path).ok()?;
            Some(Entry {
                root: o.root.clone(),
                rel: o.rel.clone(),
                size: o.size,
                mtime_ns: o.mtime_ns,
                sha256: Some(hash),
                stored: false,
                racy,
            })
        }
    }

    fn is_binary_side(&self, side: Option<&BlobRef>) -> bool {
        let Some(b) = side else {
            return false;
        };
        let (Some(hash), true) = (&b.sha256, b.stored) else {
            return true;
        };
        match self.store.get(hash) {
            Ok(bytes) => bytes.contains(&0) || std::str::from_utf8(&bytes).is_err(),
            Err(_) => true,
        }
    }

    /// Add `other_turn` to the overlap list of `turn_id`'s pending review.
    fn mark_overlap(&self, turn_id: &str, other_turn: &str, keys: &HashSet<String>) -> bool {
        let Some(mut review) = self.load_pending_one(turn_id) else {
            return false;
        };
        let mut changed = false;
        for c in &mut review.set.files {
            if keys.contains(&c.key()) && !c.overlap_with.iter().any(|t| t == other_turn) {
                c.overlap_with.push(other_turn.to_string());
                changed = true;
            }
        }
        if changed && let Err(e) = self.save_pending(&review) {
            tracing::warn!("turn capture: saving overlap mark failed: {e:#}");
        }
        changed
    }

    // ── Persistence ──────────────────────────────────────────────

    fn baseline_path(&self) -> PathBuf {
        self.dir.join("baseline.json")
    }

    fn pending_dir(&self) -> PathBuf {
        self.dir.join("pending")
    }

    fn sets_dir(&self) -> PathBuf {
        self.dir.join("sets")
    }

    fn load_baseline(&self) -> Manifest {
        std::fs::read(self.baseline_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_baseline(&self, m: &Manifest) {
        let result = (|| -> Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            atomic_write(&self.baseline_path(), &serde_json::to_vec(m)?)
        })();
        if let Err(e) = result {
            tracing::warn!("turn capture: saving baseline failed: {e:#}");
        }
    }

    /// Persist a pending review (create or update).
    pub fn save_pending(&self, review: &PendingReview) -> Result<()> {
        let dir = self.pending_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("{}.json", file_stem(&review.set.turn_id)));
        atomic_write(&path, &serde_json::to_vec_pretty(review)?)
    }

    fn load_pending_one(&self, turn_id: &str) -> Option<PendingReview> {
        let path = self.pending_dir().join(format!("{}.json", file_stem(turn_id)));
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Every pending review on disk, oldest first.
    pub fn load_pending(&self) -> Vec<PendingReview> {
        let mut out: Vec<PendingReview> = read_json_dir(&self.pending_dir());
        out.sort_by_key(|r| r.set.ended_at_ms);
        out
    }

    pub fn remove_pending(&self, turn_id: &str) {
        let path = self.pending_dir().join(format!("{}.json", file_stem(turn_id)));
        let _ = std::fs::remove_file(path);
    }

    /// Archive a reviewed turn, drop its pending file, and collect garbage.
    pub fn archive(&self, reviewed: &ReviewedTurn) -> Result<()> {
        let dir = self.sets_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("{}.json", file_stem(&reviewed.set.turn_id)));
        atomic_write(&path, &serde_json::to_vec_pretty(reviewed)?)?;
        self.remove_pending(&reviewed.set.turn_id);
        self.gc();
        Ok(())
    }

    /// Reviewed turns still within retention, newest first.
    pub fn recent_reviewed(&self) -> Vec<ReviewedTurn> {
        let mut out: Vec<ReviewedTurn> = read_json_dir(&self.sets_dir());
        out.sort_by_key(|r| std::cmp::Reverse(r.set.ended_at_ms));
        out.truncate(RETAINED_TURNS);
        out
    }

    /// Apply a review's decisions and archive it. Files without a decision
    /// are kept. `force` lists [`FileChange::key`]s whose revert may overwrite
    /// changes made after the turn ended (the user confirmed).
    pub fn resolve(&self, review: &PendingReview, force: &HashSet<String>) -> ReviewedTurn {
        let mut resolved = std::collections::BTreeMap::new();
        for c in &review.set.files {
            let key = c.key();
            let forced = force.contains(&key);
            let result = match review.decisions.get(&key).unwrap_or(&FileDecision::Keep) {
                FileDecision::Keep => ResolvedDecision::Kept,
                FileDecision::Revert => outcome_of(revert_file(self, c, forced), None),
                FileDecision::RevertHunks(h) if h.is_empty() => ResolvedDecision::Kept,
                FileDecision::RevertHunks(h) => {
                    outcome_of(revert_hunks(self, c, h, forced), Some(h.len()))
                }
            };
            resolved.insert(key, result);
        }
        let reviewed = ReviewedTurn {
            set: review.set.clone(),
            resolved,
        };
        if let Err(e) = self.archive(&reviewed) {
            tracing::warn!("turn capture: archiving {} failed: {e:#}", review.set.turn_id);
        }
        reviewed
    }

    /// Keep blobs referenced by the baseline, pending reviews, and the last
    /// [`RETAINED_TURNS`] reviewed turns; delete older archives and the rest
    /// (blobs touched within [`store::GC_GRACE`] survive — a concurrent turn
    /// may not have persisted the manifest that references them yet).
    pub fn gc(&self) -> usize {
        self.gc_with_grace(store::GC_GRACE)
    }

    pub(crate) fn gc_with_grace(&self, grace: std::time::Duration) -> usize {
        let mut keep: HashSet<String> = HashSet::new();
        let baseline = {
            let cached = self.lock().baseline.clone();
            cached.unwrap_or_else(|| self.load_baseline())
        };
        keep.extend(
            baseline
                .entries
                .values()
                .filter(|e| e.stored)
                .filter_map(|e| e.sha256.clone()),
        );
        for review in self.load_pending() {
            pin_set(&mut keep, &review.set);
        }
        let mut archived: Vec<(PathBuf, ReviewedTurn)> = read_json_dir_with_paths(&self.sets_dir());
        archived.sort_by_key(|(_, r)| std::cmp::Reverse(r.set.ended_at_ms));
        for (i, (path, reviewed)) in archived.iter().enumerate() {
            if i < RETAINED_TURNS {
                pin_set(&mut keep, &reviewed.set);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
        self.store.gc_with_grace(&keep, grace)
    }
}

fn outcome_of(r: Result<RevertOutcome>, hunks: Option<usize>) -> ResolvedDecision {
    match r {
        Ok(RevertOutcome::Reverted) => match hunks {
            Some(n) => ResolvedDecision::RevertedHunks(n),
            None => ResolvedDecision::Reverted,
        },
        Ok(RevertOutcome::AlreadyAtBefore) => ResolvedDecision::Reverted,
        Ok(RevertOutcome::Drifted) => {
            ResolvedDecision::Failed("file changed after the turn; revert not confirmed".into())
        }
        Ok(RevertOutcome::NotRevertible(why)) => ResolvedDecision::Failed(why),
        Err(e) => ResolvedDecision::Failed(format!("{e:#}")),
    }
}

fn pin_set(keep: &mut HashSet<String>, set: &TurnChangeSet) {
    for c in &set.files {
        for side in [&c.before, &c.after].into_iter().flatten() {
            if let Some(h) = &side.sha256 {
                keep.insert(h.clone());
            }
        }
    }
}

fn change(path: &Path, kind: ChangeKind, before: Option<&Entry>, after: Option<&Entry>) -> FileChange {
    let side = |e: &Entry| BlobRef {
        sha256: e.sha256.clone(),
        size: e.size,
        stored: e.stored,
    };
    let any = before.or(after).expect("a change has at least one side");
    let before_ref = before.map(side);
    let revertible = before_ref.as_ref().is_none_or(|b| b.stored && b.sha256.is_some());
    FileChange {
        path: path.to_path_buf(),
        root: any.root.clone(),
        rel: any.rel.clone(),
        kind,
        before: before_ref,
        after: after.map(side),
        revertible,
        binary: false,
        overlap_with: Vec::new(),
    }
}

/// File-name-safe form of a turn id.
fn file_stem(turn_id: &str) -> String {
    turn_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn read_json_dir<T: serde::de::DeserializeOwned>(dir: &Path) -> Vec<T> {
    read_json_dir_with_paths(dir).into_iter().map(|(_, v)| v).collect()
}

fn read_json_dir_with_paths<T: serde::de::DeserializeOwned>(dir: &Path) -> Vec<(PathBuf, T)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let v = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            Some((p, v))
        })
        .collect()
}

#[cfg(test)]
mod tests;
