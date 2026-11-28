//! The on-disk journal: one `writer.json` + one append-only `claims.ndjson`
//! per writer, in a machine-local directory.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use super::attribution::{JournalSnapshot, WriterRecord};
use super::liveness::pid_alive;
use crate::turn_capture::now_ns;
use crate::turn_capture::store::{atomic_write, hash_file};
use crate::util::fs::simplify_path;

/// Overrides the journal directory (tests, sandboxes).
pub const CLAIMS_DIR_ENV: &str = "GAVIERO_CLAIMS_DIR";
/// A writer whose end is older than this is garbage-collected.
pub const CLAIM_TTL_NS: i64 = 6 * 60 * 60 * 1_000_000_000;

const WRITER_SUFFIX: &str = ".writer.json";
const CLAIMS_SUFFIX: &str = ".claims.ndjson";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterKind {
    /// One TUI chat turn.
    ChatTurn,
    /// One swarm agent invocation.
    SwarmAgent,
    /// A swarm run's own pipeline writes (merges, verdict files).
    SwarmRun,
    /// The TUI's own writes: editor saves, review reverts, config synthesis.
    Host,
}

/// A writer's `writer.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterInfo {
    pub id: String,
    pub kind: WriterKind,
    /// Shown to the user ("gaviero-cli 'fix' · u1", "chat 'Refactor'").
    pub label: String,
    pub pid: u32,
    /// Folders the writer may write in (canonical).
    pub roots: Vec<PathBuf>,
    pub started_ns: i64,
    #[serde(default)]
    pub ended_ns: Option<i64>,
}

/// A file's state before a claimed write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "sha256", rename_all = "snake_case")]
pub enum Before {
    /// Not observed.
    Unknown,
    /// The file did not exist.
    Absent,
    /// SHA-256 hex of its content.
    Hash(String),
}

/// One line of a `claims.ndjson`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// [`claim_key`] of the path.
    pub key: String,
    pub before: Before,
    /// SHA-256 of the content after the write; `None` = the file is absent.
    #[serde(default)]
    pub after: Option<String>,
    pub at_ns: i64,
}

/// A write an agent or the host just made, as reported by its emitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    /// Absolute path.
    pub path: PathBuf,
    pub before: Before,
}

impl FileWrite {
    /// A write whose prior state was not observed.
    pub fn unknown(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            before: Before::Unknown,
        }
    }

    /// Hash the file now, before the write happens.
    pub fn before_now(path: impl Into<PathBuf>) -> Self {
        let p = path.into();
        let before = match hash_file(&p) {
            Ok(h) => Before::Hash(h),
            Err(_) if !p.exists() => Before::Absent,
            Err(_) => Before::Unknown,
        };
        Self { path: p, before }
    }
}

/// Comparison key of a path: lexically normalized, its parent resolved
/// through symlinks when it exists, `/`-separated, case-folded on Windows.
pub fn claim_key(path: &Path) -> String {
    ClaimKeyCache::default().key(path)
}

/// [`claim_key`] for many paths (a whole baseline), canonicalizing each
/// parent directory once.
#[derive(Debug, Default)]
pub struct ClaimKeyCache {
    parents: HashMap<PathBuf, Option<PathBuf>>,
}

impl ClaimKeyCache {
    pub fn key(&mut self, path: &Path) -> String {
        let lexical = crate::agent_session::tool_agent::tools::normalize_lexically(path);
        let resolved = match (lexical.parent(), lexical.file_name()) {
            (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                let canon = self
                    .parents
                    .entry(parent.to_path_buf())
                    .or_insert_with(|| std::fs::canonicalize(parent).ok().map(|c| simplify_path(&c)));
                match canon {
                    Some(c) => c.join(name),
                    None => lexical.clone(),
                }
            }
            _ => lexical.clone(),
        };
        crate::turn_capture::ledger::path_key(&resolved)
    }
}

/// File-name-safe form of a writer id.
fn stem(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn canonical_root(r: PathBuf) -> PathBuf {
    let canon = std::fs::canonicalize(&r).unwrap_or(r);
    simplify_path(&canon)
}

/// The journal directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimJournal {
    dir: PathBuf,
}

impl ClaimJournal {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `$GAVIERO_CLAIMS_DIR`, else the machine runtime dir's `claims` folder
    /// (Unix: beside the MCP sockets; Windows: `%LOCALAPPDATA%\gaviero\claims`).
    /// Never inside a workspace. Pure: nothing is created until [`Self::register`].
    pub fn machine_default() -> Option<Self> {
        if let Some(v) = std::env::var_os(CLAIMS_DIR_ENV)
            && !v.is_empty()
        {
            return Some(Self::at(PathBuf::from(v)));
        }
        #[cfg(unix)]
        {
            Some(Self::at(
                crate::mcp::transport::unix_socket_dir().join("claims"),
            ))
        }
        #[cfg(not(unix))]
        {
            Some(Self::at(
                dirs::data_local_dir()?.join("gaviero").join("claims"),
            ))
        }
    }

    fn writer_path(&self, stem: &str) -> PathBuf {
        self.dir.join(format!("{stem}{WRITER_SUFFIX}"))
    }

    fn claims_path(&self, stem: &str) -> PathBuf {
        self.dir.join(format!("{stem}{CLAIMS_SUFFIX}"))
    }

    fn ensure_dir(&self) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            crate::mcp::transport::prepare_unix_socket_dir(&self.dir)
        }
        #[cfg(not(unix))]
        {
            use anyhow::Context as _;
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating {}", self.dir.display()))
        }
    }

    /// Register a writer: create the directory, collect expired writers and
    /// write `<id>.writer.json`.
    pub fn register(
        &self,
        id: &str,
        kind: WriterKind,
        label: &str,
        roots: Vec<PathBuf>,
    ) -> anyhow::Result<Arc<ClaimWriter>> {
        self.ensure_dir()?;
        self.gc(now_ns());
        let info = WriterInfo {
            id: id.to_string(),
            kind,
            label: label.to_string(),
            pid: std::process::id(),
            roots: roots.into_iter().map(canonical_root).collect(),
            started_ns: now_ns(),
            ended_ns: None,
        };
        atomic_write(&self.writer_path(&stem(id)), &serde_json::to_vec(&info)?)?;
        Ok(Arc::new(ClaimWriter {
            journal: self.clone(),
            info,
            baseline: OnceLock::new(),
            finished: AtomicBool::new(false),
        }))
    }

    /// Every writer still relevant at `since_ns`, with its claims made at or
    /// after it. Unreadable files and half-written lines are skipped.
    pub fn snapshot(&self, since_ns: i64) -> JournalSnapshot {
        let mut writers = Vec::new();
        for (stem, info, writer_path) in self.writer_files() {
            let claims_path = self.claims_path(&stem);
            let end = effective_end(&info, &writer_path, &claims_path);
            if end.is_some_and(|e| e < since_ns) {
                continue;
            }
            let claims = read_claims(&claims_path, since_ns);
            writers.push(WriterRecord {
                info,
                effective_end_ns: end,
                claims,
            });
        }
        JournalSnapshot { writers }
    }

    /// Remove both files of every writer whose effective end is older than
    /// [`CLAIM_TTL_NS`]. Returns how many writers were removed.
    pub fn gc(&self, now_ns: i64) -> usize {
        let cutoff = now_ns - CLAIM_TTL_NS;
        let mut removed = 0;
        for (stem, info, writer_path) in self.writer_files() {
            let claims_path = self.claims_path(&stem);
            if effective_end(&info, &writer_path, &claims_path).is_some_and(|e| e < cutoff) {
                let _ = std::fs::remove_file(&claims_path);
                if std::fs::remove_file(&writer_path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// `(stem, info, path)` of every parsable `*.writer.json`.
    fn writer_files(&self) -> Vec<(String, WriterInfo, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let stem = path
                    .file_name()?
                    .to_str()?
                    .strip_suffix(WRITER_SUFFIX)?
                    .to_string();
                let info: WriterInfo = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
                Some((stem, info, path))
            })
            .collect()
    }
}

/// `ended_ns` when recorded; `None` while the writer's process lives; else
/// the last time either of its files was written (it died without finishing).
fn effective_end(info: &WriterInfo, writer_path: &Path, claims_path: &Path) -> Option<i64> {
    if let Some(e) = info.ended_ns {
        return Some(e);
    }
    if pid_alive(info.pid) {
        return None;
    }
    let mtime = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| crate::turn_capture::walk::mtime_ns(&m))
            .unwrap_or(0)
    };
    Some(mtime(writer_path).max(mtime(claims_path)))
}

fn read_claims(path: &Path, since_ns: i64) -> Vec<Claim> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Claim>(l).ok())
        .filter(|c| c.at_ns >= since_ns)
        .collect()
}

/// A registered writer. Holds no lock: every claim is one appended line.
pub struct ClaimWriter {
    journal: ClaimJournal,
    info: WriterInfo,
    /// Pre-write state of the writer's scope (chat turns: the capture
    /// baseline), for claims whose emitter could not hash before writing.
    baseline: OnceLock<(Vec<PathBuf>, HashMap<String, Before>)>,
    finished: AtomicBool,
}

impl std::fmt::Debug for ClaimWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimWriter")
            .field("id", &self.info.id)
            .field("kind", &self.info.kind)
            .finish()
    }
}

impl ClaimWriter {
    pub fn info(&self) -> &WriterInfo {
        &self.info
    }

    pub fn id(&self) -> &str {
        &self.info.id
    }

    pub fn journal(&self) -> &ClaimJournal {
        &self.journal
    }

    /// The pre-write state of `roots`, keyed by [`claim_key`]. A second call
    /// is ignored.
    pub fn set_baseline(&self, roots: Vec<PathBuf>, entries: HashMap<String, Before>) {
        let roots = roots.iter().map(|r| simplify_path(r)).collect();
        let _ = self.baseline.set((roots, entries));
    }

    fn baseline_before(&self, path: &Path, key: &str) -> Before {
        let Some((roots, entries)) = self.baseline.get() else {
            return Before::Unknown;
        };
        if let Some(b) = entries.get(key) {
            return b.clone();
        }
        let p = simplify_path(path);
        if roots.iter().any(|r| p.starts_with(r)) {
            Before::Absent
        } else {
            Before::Unknown
        }
    }

    /// Append a claim for `write`, hashing the file's current content.
    /// Errors are logged, never raised.
    pub fn record(&self, write: &FileWrite) {
        let after = hash_file(&write.path).ok();
        let key = claim_key(&write.path);
        let before = match &write.before {
            Before::Unknown => self.baseline_before(&write.path, &key),
            b => b.clone(),
        };
        let claim = Claim {
            key,
            before,
            after,
            at_ns: now_ns(),
        };
        if let Err(e) = self.append(&claim) {
            tracing::warn!(
                target: "write_claims",
                "recording a write claim for {} failed: {e:#}",
                write.path.display()
            );
        }
    }

    pub fn record_all(&self, writes: &[FileWrite]) {
        for w in writes {
            self.record(w);
        }
    }

    fn append(&self, claim: &Claim) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(claim)?;
        line.push('\n');
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut file = opts.open(self.journal.claims_path(&stem(&self.info.id)))?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Mark the writer ended. Only the first call writes.
    pub fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut info = self.info.clone();
        info.ended_ns = Some(now_ns());
        let result = serde_json::to_vec(&info)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                atomic_write(&self.journal.writer_path(&stem(&self.info.id)), &bytes)
            });
        if let Err(e) = result {
            tracing::warn!(
                target: "write_claims",
                "finishing write-claim writer {} failed: {e:#}",
                self.info.id
            );
        }
    }
}

impl Drop for ClaimWriter {
    fn drop(&mut self) {
        self.finish()
    }
}

/// A swarm agent's writer, carried on its request.
#[derive(Clone)]
pub struct ClaimHandle {
    pub writer: Arc<ClaimWriter>,
    /// Filter other writers' claimed files out of this agent's reconcile
    /// (`turnReview.attribution`).
    pub consume: bool,
}

impl std::fmt::Debug for ClaimHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimHandle")
            .field("id", &self.writer.id())
            .field("consume", &self.consume)
            .finish()
    }
}
