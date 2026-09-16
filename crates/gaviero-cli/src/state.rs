//! Where a headless run keeps its gaviero state.
//!
//! By default the CLI shares state with the TUI: it searches the run root
//! and its parents for the workspace `gaviero` would open there and reads
//! and writes that workspace's memory. `--isolated` — and a search that
//! finds nothing — runs against throwaway state in a temp directory
//! instead, reading only settings from the repo.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gaviero_core::workspace::Workspace;

const WORKSPACE_FILE_EXT: &str = "gaviero-workspace";

/// Config files `synthesize_for_worktree` writes under an agent root, plus
/// the endpoint descriptor the MCP host writes there. An isolated run puts
/// these back afterwards so a TUI on the same folder keeps its own server.
const SYNTHESIZED_FILES: &[&str] = &[
    ".mcp.json",
    ".claude/settings.json",
    ".cursor/mcp.json",
    ".cursor/cli.json",
    ".codex/config.toml",
    ".codex/rules/gaviero.rules",
    ".codex/agents/gaviero-worker.toml",
    ".gaviero/mcp-endpoint.json",
    ".gaviero/managed-permissions.json",
];

/// Directories synthesis may create, deepest first. Removed again when an
/// isolated run created them and left them empty.
const SYNTHESIZED_DIRS: &[&str] = &[
    ".codex/rules",
    ".codex/agents",
    ".codex",
    ".cursor",
    ".claude",
];

/// A folder counts as a gaviero workspace root once the TUI configured it
/// or wrote memory there. A bare `.gaviero/` (worktrees, run state,
/// spilled prompts — all an isolated run leaves behind) does not.
pub fn has_workspace_marker(dir: &Path) -> bool {
    let gaviero = dir.join(".gaviero");
    gaviero.join("settings.json").is_file() || gaviero.join("memory.db").is_file()
}

/// The workspace a TUI would open for a run root.
pub struct Discovered {
    /// Settings cascade and folder list, loaded the way `gaviero` loads it.
    pub workspace: Workspace,
    /// Folder holding the `.gaviero/` marker the search stopped at.
    pub marker_root: PathBuf,
    /// The `*.gaviero-workspace` file, in multi-folder mode.
    pub workspace_file: Option<PathBuf>,
    /// Where the TUI keeps workspace-level state: the workspace memory DB,
    /// MCP endpoint, telemetry, history and code graph (its first root).
    pub state_root: PathBuf,
    /// Workspace folder containing the run root; the run's repo-scoped
    /// memory is written here.
    pub memory_root: PathBuf,
    /// Remarks for the run banner.
    pub notes: Vec<String>,
}

impl Discovered {
    pub fn describe(&self) -> String {
        let mut out = match &self.workspace_file {
            Some(file) => format!("workspace {}", file.display()),
            None => format!("workspace {}", self.marker_root.display()),
        };
        if self.state_root != self.marker_root {
            out.push_str(&format!(" (state {})", self.state_root.display()));
        }
        if self.memory_root != self.state_root {
            out.push_str(&format!(" (repo scope {})", self.memory_root.display()));
        }
        out
    }
}

/// Search `start` and its parents for a gaviero workspace. The home
/// directory is skipped: `~/.gaviero/` holds user-wide settings, not a
/// workspace.
pub fn discover(start: &Path) -> Result<Option<Discovered>> {
    discover_with_home(start, dirs::home_dir().as_deref())
}

fn discover_with_home(start: &Path, home: Option<&Path>) -> Result<Option<Discovered>> {
    let start = canonical(start);
    let home = home.map(canonical);
    let Some(marker_root) = start
        .ancestors()
        .find(|dir| Some(*dir) != home.as_deref() && has_workspace_marker(dir))
        .map(Path::to_path_buf)
    else {
        return Ok(None);
    };

    let mut notes = Vec::new();
    if let Some((file, workspace)) = find_workspace_file(&marker_root)? {
        match workspace.folder_for_path(&start).map(Path::to_path_buf) {
            Some(member) => {
                let state_root = workspace
                    .roots()
                    .first()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| member.clone());
                return Ok(Some(Discovered {
                    workspace,
                    marker_root,
                    workspace_file: Some(file),
                    state_root,
                    memory_root: member,
                    notes,
                }));
            }
            None => notes.push(format!(
                "{} is not inside any folder of {}; using {} as a single-folder workspace",
                start.display(),
                file.display(),
                marker_root.display()
            )),
        }
    }

    Ok(Some(Discovered {
        workspace: Workspace::single_folder(marker_root.clone()),
        state_root: marker_root.clone(),
        memory_root: marker_root.clone(),
        marker_root,
        workspace_file: None,
        notes,
    }))
}

/// A `*.gaviero-workspace` file that sits in `marker_root` (what
/// `gaviero --workspace <marker_root>` opens) or, in a parent, lists
/// `marker_root` as a folder.
fn find_workspace_file(marker_root: &Path) -> Result<Option<(PathBuf, Workspace)>> {
    for dir in marker_root.ancestors() {
        for file in workspace_files_in(dir) {
            if dir == marker_root {
                let workspace = Workspace::load(&file)
                    .with_context(|| format!("loading workspace file {}", file.display()))?;
                return Ok(Some((file, workspace)));
            }
            let Ok(workspace) = Workspace::load(&file) else {
                continue;
            };
            if workspace
                .roots()
                .iter()
                .any(|root| canonical(root) == marker_root)
            {
                return Ok(Some((file, workspace)));
            }
        }
    }
    Ok(None)
}

/// Workspace files in `dir`, ordered as the TUI picks them:
/// `<dirname>.gaviero-workspace` first, then by name.
fn workspace_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == WORKSPACE_FILE_EXT))
        .collect();
    files.sort();
    if let Some(name) = dir.file_name() {
        let preferred = dir.join(format!("{}.{WORKSPACE_FILE_EXT}", name.to_string_lossy()));
        if let Some(i) = files.iter().position(|p| *p == preferred) {
            let p = files.remove(i);
            files.insert(0, p);
        }
    }
    files
}

fn canonical(path: &Path) -> PathBuf {
    gaviero_core::util::fs::canonicalize_simplified(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Throwaway state for an isolated run. [`Self::remove`] deletes it; drop
/// makes one silent attempt.
pub struct Scratch {
    root: PathBuf,
    removed: bool,
}

impl Scratch {
    pub fn create() -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("gaviero-isolated-")
            .tempdir()
            .context("creating isolated state directory")?
            .keep();
        std::fs::create_dir_all(root.join(".gaviero"))
            .context("creating isolated .gaviero directory")?;
        Ok(Self {
            root,
            removed: false,
        })
    }

    /// Delete the state directory. Retries for a while: the memory writer
    /// and MCP server release their SQLite handles asynchronously, and
    /// Windows refuses to delete open files.
    pub async fn remove(mut self) -> std::io::Result<()> {
        self.removed = true;
        let mut last = Ok(());
        for _ in 0..30 {
            match std::fs::remove_dir_all(&self.root) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => last = Err(e),
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        last
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn memory_db(&self) -> PathBuf {
        self.root().join(".gaviero").join("memory.db")
    }

    pub fn graph_db(&self) -> PathBuf {
        self.root().join(".gaviero").join("code_graph.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.removed {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// State a swarm run reads and writes.
pub enum RunState {
    /// The discovered workspace: its memory is read and written.
    Shared(Discovered),
    /// Throwaway memory, graph and telemetry. Settings still come from the
    /// discovered workspace when there is one.
    Isolated {
        scratch: Scratch,
        workspace: Workspace,
        settings_from: Option<PathBuf>,
    },
}

impl RunState {
    pub fn isolated(discovered: Option<Discovered>, run_root: &Path) -> Result<Self> {
        let scratch = Scratch::create()?;
        Ok(match discovered {
            Some(d) => Self::Isolated {
                scratch,
                settings_from: Some(d.marker_root),
                workspace: d.workspace,
            },
            None => Self::Isolated {
                scratch,
                settings_from: None,
                workspace: Workspace::single_folder(run_root.to_path_buf()),
            },
        })
    }

    pub fn workspace(&self) -> &Workspace {
        match self {
            Self::Shared(d) => &d.workspace,
            Self::Isolated { workspace, .. } => workspace,
        }
    }

    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::Isolated { .. })
    }

    /// The throwaway state directory, for explicit removal.
    pub fn into_scratch(self) -> Option<Scratch> {
        match self {
            Self::Shared(_) => None,
            Self::Isolated { scratch, .. } => Some(scratch),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Shared(d) => format!("shared — {}", d.describe()),
            Self::Isolated {
                scratch,
                settings_from,
                ..
            } => format!(
                "isolated — temporary memory at {} (settings from {})",
                scratch.root().display(),
                settings_from
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "defaults".to_string())
            ),
        }
    }

    /// Root the embedder and other memory settings resolve against.
    pub fn settings_root<'a>(&'a self, run_root: &'a Path) -> &'a Path {
        match self {
            Self::Shared(d) => &d.state_root,
            Self::Isolated { .. } => run_root,
        }
    }

    /// `SwarmConfig::memory_root`.
    pub fn swarm_memory_root(&self) -> Option<PathBuf> {
        match self {
            Self::Shared(d) => Some(d.memory_root.clone()),
            Self::Isolated { .. } => None,
        }
    }

    /// Code graph for `run_root`. A run root other than the TUI's state
    /// root gets its own database under that root — a graph build deletes
    /// every file it did not scan, so it must not share the TUI's.
    pub fn graph_db(&self, run_root: &Path) -> PathBuf {
        match self {
            Self::Shared(d) if canonical(run_root) == canonical(&d.state_root) => {
                gaviero_core::repo_map::graph_builder::graph_db_path(run_root)
            }
            Self::Shared(d) => d
                .state_root
                .join(".gaviero")
                .join("graphs")
                .join(gaviero_core::memory::hash_path(run_root))
                .join("code_graph.db"),
            Self::Isolated { scratch, .. } => scratch.graph_db(),
        }
    }

    /// `SwarmConfig::graph_db_path`: `None` keeps the default location.
    pub fn swarm_graph_db(&self, run_root: &Path) -> Option<PathBuf> {
        let db = self.graph_db(run_root);
        (db != gaviero_core::repo_map::graph_builder::graph_db_path(run_root)).then_some(db)
    }

    /// Where the MCP server's endpoint, HTTP port and token derive from.
    pub fn endpoint_root<'a>(&'a self, run_root: &'a Path) -> &'a Path {
        match self {
            Self::Shared(_) => run_root,
            Self::Isolated { scratch, .. } => scratch.root(),
        }
    }

    /// Directory whose `.gaviero/mcp_calls.ndjson` receives tool telemetry.
    pub fn telemetry_root(&self) -> &Path {
        match self {
            Self::Shared(d) => &d.state_root,
            Self::Isolated { scratch, .. } => scratch.root(),
        }
    }

    /// Folder whose repo-scoped memory the MCP server reaches.
    pub fn mcp_memory_root<'a>(&'a self, run_root: &'a Path) -> &'a Path {
        match self {
            Self::Shared(d) => &d.memory_root,
            Self::Isolated { .. } => run_root,
        }
    }
}

/// Snapshot of the synthesized agent config files under a root, put back
/// by [`Self::restore`] or on drop.
pub struct ConfigRestore {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
    new_dirs: Vec<PathBuf>,
    restored: bool,
}

impl ConfigRestore {
    pub fn capture(root: &Path) -> Result<Self> {
        let mut files = Vec::with_capacity(SYNTHESIZED_FILES.len());
        for rel in SYNTHESIZED_FILES {
            let path = root.join(rel);
            let previous = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(e).with_context(|| format!("snapshotting {}", path.display()));
                }
            };
            files.push((path, previous));
        }
        let new_dirs = SYNTHESIZED_DIRS
            .iter()
            .map(|rel| root.join(rel))
            .filter(|dir| !dir.exists())
            .collect();
        Ok(Self {
            files,
            new_dirs,
            restored: false,
        })
    }

    /// Put every snapshotted file back. Returns one message per file that
    /// could not be restored.
    pub fn restore(&mut self) -> Vec<String> {
        if std::mem::replace(&mut self.restored, true) {
            return Vec::new();
        }
        let mut errors = Vec::new();
        for (path, previous) in &self.files {
            let result = match previous {
                Some(bytes) if std::fs::read(path).ok().as_deref() == Some(bytes.as_slice()) => {
                    Ok(())
                }
                Some(bytes) => path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(path, bytes)),
                None => match std::fs::remove_file(path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            if let Err(e) = result {
                errors.push(format!("{}: {e}", path.display()));
            }
        }
        // `remove_dir` refuses non-empty directories, which is the point.
        for dir in &self.new_dirs {
            let _ = std::fs::remove_dir(dir);
        }
        errors
    }
}

impl Drop for ConfigRestore {
    fn drop(&mut self) {
        for error in self.restore() {
            eprintln!("[state] could not restore agent config {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(dir: &Path, file: &str) {
        std::fs::create_dir_all(dir.join(".gaviero")).unwrap();
        std::fs::write(dir.join(".gaviero").join(file), "{}").unwrap();
    }

    fn write_workspace_file(path: &Path, folders: &[&Path]) {
        let folders: Vec<_> = folders
            .iter()
            .map(|f| serde_json::json!({ "path": f }))
            .collect();
        std::fs::write(path, serde_json::json!({ "folders": folders }).to_string()).unwrap();
    }

    #[test]
    fn walks_up_to_the_nearest_marked_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical(tmp.path());
        mark(&root, "settings.json");
        let run = root.join("plans").join("x");
        std::fs::create_dir_all(&run).unwrap();

        let d = discover_with_home(&run, None).unwrap().expect("found");
        assert_eq!(d.marker_root, root);
        assert_eq!(d.state_root, root);
        assert_eq!(d.memory_root, root);
        assert!(d.workspace_file.is_none());
    }

    #[test]
    fn a_memory_db_alone_marks_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical(tmp.path());
        mark(&root, "memory.db");
        let d = discover_with_home(&root, None).unwrap().expect("found");
        assert_eq!(d.marker_root, root);
    }

    #[test]
    fn a_bare_gaviero_dir_is_not_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical(tmp.path());
        mark(&root, "settings.json");
        let run = root.join("sub");
        std::fs::create_dir_all(run.join(".gaviero").join("worktrees")).unwrap();

        let d = discover_with_home(&run, None).unwrap().expect("found");
        assert_eq!(
            d.marker_root, root,
            "leftover run files must not stop the search"
        );
    }

    #[test]
    fn nothing_marked_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&run).unwrap();
        // Treat the tempdir as home so an unrelated ancestor marker (a real
        // home or a developer's checkout above the temp dir) cannot leak in.
        assert!(
            discover_with_home(&run, Some(tmp.path()))
                .unwrap()
                .is_none_or(|d| !d.marker_root.starts_with(canonical(tmp.path())))
        );
    }

    #[test]
    fn the_home_directory_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = canonical(tmp.path());
        mark(&home, "settings.json");
        let run = home.join("project");
        std::fs::create_dir_all(&run).unwrap();

        let found = discover_with_home(&run, Some(&home)).unwrap();
        assert!(found.is_none_or(|d| d.marker_root != home));
    }

    #[test]
    fn a_workspace_file_in_the_marked_folder_selects_multi_folder_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = canonical(tmp.path());
        mark(&ws, "settings.json");
        let a = ws.join("a");
        let b = ws.join("b");
        std::fs::create_dir_all(b.join("src")).unwrap();
        std::fs::create_dir_all(&a).unwrap();
        let file = ws.join("ws.gaviero-workspace");
        write_workspace_file(&file, &[&a, &b]);

        let d = discover_with_home(&b.join("src"), None)
            .unwrap()
            .expect("found");
        assert_eq!(d.workspace_file.as_deref(), Some(file.as_path()));
        assert_eq!(
            d.state_root, a,
            "the TUI keeps workspace state in its first folder"
        );
        assert_eq!(canonical(&d.memory_root), b);
    }

    #[test]
    fn a_parent_workspace_file_listing_the_marked_folder_is_used() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = canonical(tmp.path());
        let a = ws.join("a");
        std::fs::create_dir_all(&a).unwrap();
        mark(&a, "memory.db");
        let file = ws.join("team.gaviero-workspace");
        write_workspace_file(&file, &[&a]);
        // An unrelated workspace file next to it must not win.
        write_workspace_file(&ws.join("aaa.gaviero-workspace"), &[&ws.join("other")]);

        let d = discover_with_home(&a, None).unwrap().expect("found");
        assert_eq!(d.workspace_file.as_deref(), Some(file.as_path()));
        assert_eq!(canonical(&d.memory_root), a);
    }

    #[test]
    fn a_run_root_outside_every_member_falls_back_to_single_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = canonical(tmp.path());
        mark(&ws, "settings.json");
        let a = ws.join("a");
        std::fs::create_dir_all(&a).unwrap();
        write_workspace_file(&ws.join("ws.gaviero-workspace"), &[&a]);
        let run = ws.join("plans");
        std::fs::create_dir_all(&run).unwrap();

        let d = discover_with_home(&run, None).unwrap().expect("found");
        assert!(d.workspace_file.is_none());
        assert_eq!(d.state_root, ws);
        assert_eq!(d.notes.len(), 1);
    }

    #[test]
    fn a_subfolder_run_gets_its_own_graph_under_the_state_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical(tmp.path());
        mark(&root, "settings.json");
        let run = root.join("plans");
        std::fs::create_dir_all(&run).unwrap();
        let state = RunState::Shared(discover_with_home(&run, None).unwrap().unwrap());

        let db = state.graph_db(&run);
        assert!(db.starts_with(root.join(".gaviero").join("graphs")));
        assert_eq!(state.swarm_graph_db(&run), Some(db));
        assert_eq!(state.swarm_graph_db(&root), None);
        assert_eq!(state.swarm_memory_root(), Some(root));
    }

    #[test]
    fn isolated_state_lives_outside_the_run_root() {
        let tmp = tempfile::tempdir().unwrap();
        let run = canonical(tmp.path());
        let state = RunState::isolated(None, &run).unwrap();
        assert!(state.is_isolated());
        assert!(!state.graph_db(&run).starts_with(&run));
        assert!(!state.endpoint_root(&run).starts_with(&run));
        assert!(!state.telemetry_root().starts_with(&run));
        assert_eq!(state.swarm_memory_root(), None);
        assert!(
            !run.join(".gaviero").exists(),
            "isolated setup writes nothing to the run root"
        );
    }

    #[tokio::test]
    async fn scratch_remove_deletes_the_directory() {
        let scratch = Scratch::create().unwrap();
        let root = scratch.root().to_path_buf();
        std::fs::write(scratch.memory_db(), "x").unwrap();
        scratch.remove().await.unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn restore_puts_back_changed_files_and_removes_new_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".mcp.json"), "before").unwrap();
        let mut restore = ConfigRestore::capture(root).unwrap();

        std::fs::write(root.join(".mcp.json"), "after").unwrap();
        std::fs::create_dir_all(root.join(".cursor")).unwrap();
        std::fs::write(root.join(".cursor/mcp.json"), "new").unwrap();
        std::fs::create_dir_all(root.join(".gaviero")).unwrap();
        std::fs::write(root.join(".gaviero/mcp-endpoint.json"), "new").unwrap();

        assert!(restore.restore().is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join(".mcp.json")).unwrap(),
            "before"
        );
        assert!(
            !root.join(".cursor").exists(),
            "a directory the run created is removed"
        );
        assert!(!root.join(".gaviero/mcp-endpoint.json").exists());
        assert!(
            root.join(".gaviero").exists(),
            "run files stay under .gaviero/"
        );
    }

    #[test]
    fn restore_runs_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        {
            let _restore = ConfigRestore::capture(root).unwrap();
            std::fs::write(root.join(".mcp.json"), "synthesized").unwrap();
        }
        assert!(!root.join(".mcp.json").exists());
    }

    /// The snapshot list must cover everything synthesis writes, or an
    /// isolated run would leave a repointed config behind.
    #[test]
    fn restore_covers_every_synthesized_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical(tmp.path());
        let workspace = Workspace::single_folder(root.clone());
        let overrides = gaviero_core::mcp::McpConfigOverrides {
            codex_trust: Some(gaviero_core::mcp::TrustConsent::Granted),
            extra_urls: vec![("remote".into(), "https://example.com/mcp/".into())],
            ..Default::default()
        };
        let mut synth = gaviero_core::mcp::resolve_mcp_config_synth(
            &workspace,
            &root,
            gaviero_core::mcp::McpEndpoint::for_workspace(&root),
            &overrides,
        );
        synth.explicit_ref_required = true;

        let mut restore = ConfigRestore::capture(&root).unwrap();
        let written = gaviero_core::mcp::synthesize_for_worktree(&synth).unwrap();
        assert!(!written.is_empty());
        for path in &written {
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            assert!(
                SYNTHESIZED_FILES.contains(&rel.as_str()),
                "{rel} is synthesized but not restored"
            );
        }
        assert!(restore.restore().is_empty());
        for path in &written {
            assert!(!path.exists(), "{} survived the restore", path.display());
        }
    }
}
