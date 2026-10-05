//! The baseline: every tracked file's size, mtime, and content hash.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub root: PathBuf,
    pub rel: String,
    pub size: u64,
    pub mtime_ns: i64,
    /// `None` past the first-baseline cap: the file is tracked by stat only.
    pub sha256: Option<String>,
    /// Content is in the blob store.
    pub stored: bool,
    /// The mtime fell inside the racy window of the scan that captured this
    /// entry, so an equal (size, mtime) later does not prove equal content.
    #[serde(default)]
    pub racy: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: BTreeMap<PathBuf, Entry>,
}

impl Manifest {
    /// Entries under any of `roots`.
    pub fn in_scope<'a>(&'a self, roots: &'a [PathBuf]) -> impl Iterator<Item = (&'a PathBuf, &'a Entry)> {
        self.entries
            .iter()
            .filter(move |(p, _)| under_any(p, roots))
    }
}

pub(crate) fn under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}
