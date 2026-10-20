//! A live code graph per workspace folder, with a single writer.
//!
//! [`GraphService`] owns one folder's on-disk graph
//! (`<folder>/.gaviero/code_graph.db` by default). A dedicated writer thread
//! is the only connection that writes it: file-change notifications are
//! coalesced (until [`COALESCE_QUIET`] of quiet, at most
//! [`COALESCE_MAX_WAIT`]), applied with [`graph_builder::update_paths`] in one
//! transaction (a full incremental scan above [`FULL_SCAN_THRESHOLD`] dirty
//! paths, on [`GraphService::rescan`], and at start), committed to disk, and
//! then published as a fresh [`RepoMap`] snapshot.
//!
//! Readers never take a lock across I/O: [`GraphService::current`] clones the
//! snapshot `Arc`, [`GraphService::with_reader`] opens a fresh WAL connection,
//! and [`GraphService::flush`] / [`GraphService::snapshot`] wait for the
//! pending batch so a reader never sees a state older than the changes it was
//! told about. The thread exits when the last handle is dropped.
//!
//! [`GraphServices`] holds one service per workspace folder (the first is the
//! primary) and routes absolute paths to the folder that contains them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use super::store::GraphStore;
use super::{RepoMap, SpecificityConfig, builder, graph_builder};

/// Quiet window that closes a batch of change notifications.
pub const COALESCE_QUIET: Duration = Duration::from_millis(300);
/// Longest a batch waits for quiet before it is applied anyway.
pub const COALESCE_MAX_WAIT: Duration = Duration::from_secs(2);
/// Above this many dirty paths a batch runs a full incremental scan instead
/// of a targeted update.
pub const FULL_SCAN_THRESHOLD: usize = 1000;

/// One workspace folder's graph configuration.
#[derive(Clone, Debug)]
pub struct GraphFolder {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub excludes: Vec<String>,
    pub specificity: SpecificityConfig,
}

/// Writer counters, for logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphServiceStats {
    /// Change batches applied (the startup scan is not counted).
    pub batches_applied: u64,
    /// Full incremental scans, the startup scan included.
    pub full_scans: u64,
    /// Wall time of the last applied batch.
    pub last_batch_ms: u64,
}

enum Msg {
    Changed(Vec<String>),
    Flush(tokio::sync::oneshot::Sender<()>),
    Rescan,
}

#[derive(Default)]
struct Shared {
    snapshot: RwLock<Option<Arc<RepoMap>>>,
    stats: RwLock<GraphServiceStats>,
}

/// Owned by the handles only. The writer thread holds the folder, the
/// shared state and the receiver — never this — so dropping the last handle
/// drops the sender and ends the thread.
struct Handle {
    folder: GraphFolder,
    shared: Arc<Shared>,
    tx: OnceLock<Sender<Msg>>,
}

/// The live graph of one workspace folder. Cheap to clone; all clones share
/// one writer thread.
#[derive(Clone)]
pub struct GraphService {
    inner: Arc<Handle>,
}

impl GraphService {
    /// A service for `folder`. Lazy: nothing runs until [`Self::start`] or
    /// the first notification / read.
    pub fn new(folder: GraphFolder) -> Self {
        Self {
            inner: Arc::new(Handle {
                folder,
                shared: Arc::new(Shared::default()),
                tx: OnceLock::new(),
            }),
        }
    }

    /// The folder root.
    pub fn root(&self) -> &Path {
        &self.inner.folder.root
    }

    /// The on-disk graph database.
    pub fn db_path(&self) -> &Path {
        &self.inner.folder.db_path
    }

    /// Start the writer thread (idempotent). Its first job is a full
    /// incremental scan of the folder.
    pub fn start(&self) {
        self.sender();
    }

    fn sender(&self) -> &Sender<Msg> {
        self.inner.tx.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let folder = self.inner.folder.clone();
            let shared = Arc::clone(&self.inner.shared);
            let spawned = std::thread::Builder::new()
                .name("gaviero-graph-writer".into())
                .spawn(move || writer_loop(folder, shared, rx));
            if let Err(e) = spawned {
                tracing::warn!(target: "repo_map", error = %e, "graph writer thread failed to start");
            }
            tx
        })
    }

    /// Queue changed paths (absolute) for the next batch. Paths outside the
    /// folder, under a dot directory or under [`builder::SKIP_DIRS`] are
    /// dropped here, so gaviero's own writes under `.gaviero/` never trigger
    /// a batch. Never blocks.
    pub fn notify_changed(&self, abs_paths: &[PathBuf]) {
        let rels: Vec<String> = abs_paths
            .iter()
            .filter_map(|p| relevant_rel(self.root(), p))
            .collect();
        if rels.is_empty() {
            return;
        }
        let _ = self.sender().send(Msg::Changed(rels));
    }

    /// Queue a full incremental scan of the folder.
    pub fn rescan(&self) {
        let _ = self.sender().send(Msg::Rescan);
    }

    /// Wait until every change notified before this call is on disk and
    /// published (the startup scan included).
    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.sender().send(Msg::Flush(tx)).is_err() {
            return;
        }
        let _ = rx.await;
    }

    /// [`Self::flush`], then the current snapshot.
    pub async fn snapshot(&self) -> Option<Arc<RepoMap>> {
        self.flush().await;
        self.current()
    }

    /// The last published snapshot, without waiting. `None` before the
    /// startup scan finishes.
    pub fn current(&self) -> Option<Arc<RepoMap>> {
        self.inner
            .shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Run `f` on a fresh read connection to the on-disk graph. Blocking:
    /// call it from `spawn_blocking`.
    pub fn with_reader<T>(&self, f: impl FnOnce(&GraphStore) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let store = GraphStore::open(self.db_path())?;
        f(&store)
    }

    /// Run `f` on a query-only connection to the on-disk graph. Blocking; the
    /// caller keeps it on one thread. Never runs migrations and never creates
    /// the database: `f` is not called when it does not exist.
    pub fn with_query_reader<T>(
        &self,
        f: impl FnOnce(&GraphStore) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let store = GraphStore::open_query_only(self.db_path())?;
        f(&store)
    }

    /// Writer counters.
    pub fn stats(&self) -> GraphServiceStats {
        *self.inner.shared.stats.read().unwrap_or_else(|e| e.into_inner())
    }
}

/// `abs` as a `/`-separated path relative to `root`, or `None` when it is
/// outside `root` or under a dot directory or a [`builder::SKIP_DIRS`] name.
fn relevant_rel(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let mut parts: Vec<&str> = Vec::new();
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            return None;
        };
        let name = name.to_str()?;
        if name.starts_with('.') || builder::SKIP_DIRS.contains(&name) {
            return None;
        }
        parts.push(name);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn writer_loop(folder: GraphFolder, shared: Arc<Shared>, rx: Receiver<Msg>) {
    let store = match GraphStore::open(&folder.db_path) {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!(
                target: "repo_map",
                db = %folder.db_path.display(),
                error = %e,
                "graph writer cannot open its database"
            );
            // Keep answering flushes so readers never hang.
            while let Ok(msg) = rx.recv() {
                if let Msg::Flush(waiter) = msg {
                    let _ = waiter.send(());
                }
            }
            return;
        }
    };

    // Startup: a full incremental scan against whatever the DB holds. If it
    // fails, the next batch is a full scan rather than a targeted update on
    // a possibly partial graph.
    let mut needs_full = match graph_builder::build_graph_into(&store, &folder.root, &folder.excludes) {
        Ok(_) => {
            shared.stats.write().unwrap_or_else(|e| e.into_inner()).full_scans += 1;
            publish(&store, &folder, &shared);
            false
        }
        Err(e) => {
            tracing::warn!(
                target: "repo_map",
                root = %folder.root.display(),
                error = %e,
                "graph startup scan failed; the next batch rescans"
            );
            true
        }
    };

    loop {
        let Ok(first) = rx.recv() else {
            return;
        };
        let batch_start = Instant::now();
        let mut dirty: BTreeSet<String> = BTreeSet::new();
        let mut full = false;
        let mut waiters: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();
        let mut disconnected = false;
        let mut flush_now = false;
        match first {
            Msg::Changed(paths) => dirty.extend(paths),
            Msg::Rescan => full = true,
            Msg::Flush(waiter) => {
                waiters.push(waiter);
                flush_now = true;
            }
        }
        // Coalesce until quiet, a flush, or the max wait.
        while !flush_now {
            let elapsed = batch_start.elapsed();
            if elapsed >= COALESCE_MAX_WAIT {
                break;
            }
            match rx.recv_timeout(COALESCE_QUIET.min(COALESCE_MAX_WAIT - elapsed)) {
                Ok(Msg::Changed(paths)) => dirty.extend(paths),
                Ok(Msg::Rescan) => full = true,
                Ok(Msg::Flush(waiter)) => {
                    waiters.push(waiter);
                    flush_now = true;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if full || needs_full || !dirty.is_empty() {
            needs_full = !apply(&store, &folder, &shared, dirty, full || needs_full);
        }
        for waiter in waiters {
            let _ = waiter.send(());
        }
        if disconnected {
            return;
        }
    }
}

/// Apply one batch, commit it, and publish a new snapshot if the graph
/// changed. Returns false when the update failed.
fn apply(store: &GraphStore, folder: &GraphFolder, shared: &Shared, dirty: BTreeSet<String>, full: bool) -> bool {
    let started = Instant::now();
    let dirty_len = dirty.len();
    let mut full_used = full || dirty_len > FULL_SCAN_THRESHOLD;
    let outcome = if full_used {
        graph_builder::build_graph_into(store, &folder.root, &folder.excludes)
    } else {
        let paths: Vec<String> = dirty.into_iter().collect();
        graph_builder::update_paths(store, &folder.root, &folder.excludes, &paths).or_else(|e| {
            tracing::warn!(
                target: "repo_map",
                error = %e,
                "targeted graph update failed; running a full scan"
            );
            full_used = true;
            graph_builder::build_graph_into(store, &folder.root, &folder.excludes)
        })
    };
    let changed = match &outcome {
        Ok(r) => full_used || r.files_changed + r.files_removed + r.files_reresolved > 0,
        Err(e) => {
            tracing::warn!(target: "repo_map", error = %e, "graph update failed");
            false
        }
    };
    if changed {
        publish(store, folder, shared);
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    {
        let mut stats = shared.stats.write().unwrap_or_else(|e| e.into_inner());
        stats.batches_applied += 1;
        if full_used {
            stats.full_scans += 1;
        }
        stats.last_batch_ms = elapsed_ms;
    }
    tracing::debug!(
        target: "repo_map",
        root = %folder.root.display(),
        dirty = dirty_len,
        full = full_used,
        changed,
        elapsed_ms,
        "graph batch applied"
    );
    outcome.is_ok()
}

/// Project the store and swap the snapshot. The projection runs outside the
/// lock; the lock only guards the `Arc` swap.
fn publish(store: &GraphStore, folder: &GraphFolder, shared: &Shared) {
    match RepoMap::from_store(store, folder.specificity) {
        Ok(map) => {
            let map = Arc::new(map);
            let previous = shared
                .snapshot
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .replace(map);
            // A large old map is freed here, after the lock is released.
            drop(previous);
        }
        Err(e) => tracing::warn!(target: "repo_map", error = %e, "repo map projection failed"),
    }
}

/// One [`GraphService`] per workspace folder; the first is the primary.
#[derive(Clone)]
pub struct GraphServices {
    services: Arc<Vec<GraphService>>,
}

impl GraphServices {
    /// Services for `folders`, in order. Lazy: nothing starts.
    pub fn new(folders: Vec<GraphFolder>) -> Self {
        Self {
            services: Arc::new(folders.into_iter().map(GraphService::new).collect()),
        }
    }

    /// The first folder's service.
    pub fn primary(&self) -> Option<&GraphService> {
        self.services.first()
    }

    /// Every service, in folder order.
    pub fn all(&self) -> &[GraphService] {
        &self.services
    }

    /// The service whose root is exactly `root`.
    pub fn for_root(&self, root: &Path) -> Option<&GraphService> {
        self.services.iter().find(|s| s.root() == root)
    }

    /// The service (and its index) whose root is the longest prefix of
    /// `abs`.
    pub fn containing(&self, abs: &Path) -> Option<(usize, &GraphService)> {
        self.services
            .iter()
            .enumerate()
            .filter(|(_, s)| abs.starts_with(s.root()))
            .max_by_key(|(_, s)| s.root().as_os_str().len())
    }

    /// Route changed absolute paths to the folders that contain them.
    pub fn notify_changed(&self, abs_paths: &[PathBuf]) {
        let mut by_folder: Vec<Vec<PathBuf>> = vec![Vec::new(); self.services.len()];
        for path in abs_paths {
            if let Some((idx, _)) = self.containing(path) {
                by_folder[idx].push(path.clone());
            }
        }
        for (service, paths) in self.services.iter().zip(by_folder) {
            if !paths.is_empty() {
                service.notify_changed(&paths);
            }
        }
    }

    /// Start every folder's writer.
    pub fn start_all(&self) {
        for service in self.services.iter() {
            service.start();
        }
    }

    /// Queue a full scan of every folder.
    pub fn rescan_all(&self) {
        for service in self.services.iter() {
            service.rescan();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked_folder(dir: &Path) -> GraphFolder {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("seed.rs"), "pub fn gv_seed_fn() { gv_target_fn(); }\n").unwrap();
        std::fs::write(src.join("zz_target.rs"), "pub fn gv_target_fn() {}\n").unwrap();
        GraphFolder {
            root: dir.to_path_buf(),
            db_path: dir.join(".gaviero").join("code_graph.db"),
            excludes: Vec::new(),
            specificity: SpecificityConfig::default(),
        }
    }

    fn has_node(map: &RepoMap, path: &str) -> bool {
        map.graph
            .node_indices()
            .any(|i| map.graph[i].path.to_string_lossy() == path)
    }

    #[tokio::test]
    async fn snapshot_builds_on_first_use() {
        let dir = tempfile::tempdir().unwrap();
        let service = GraphService::new(linked_folder(dir.path()));
        let map = service.snapshot().await.expect("snapshot");
        assert!(map.edge_count() >= 2, "edges: {}", map.edge_count());
        assert_eq!(service.stats().full_scans, 1);
    }

    #[tokio::test]
    async fn change_reaches_snapshot_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        let service = GraphService::new(linked_folder(dir.path()));
        service.snapshot().await.expect("snapshot");

        let new_file = dir.path().join("src").join("new_caller.rs");
        std::fs::write(&new_file, "pub fn gv_new_fn() { gv_target_fn(); }\n").unwrap();
        service.notify_changed(std::slice::from_ref(&new_file));
        let map = service.snapshot().await.expect("snapshot");
        assert!(has_node(&map, "src/new_caller.rs"));

        let on_disk = GraphStore::open(service.db_path()).unwrap();
        assert!(
            on_disk
                .file_edges()
                .unwrap()
                .iter()
                .any(|(s, t, _)| s == "src/new_caller.rs" && t == "src/zz_target.rs"),
            "the committed DB holds the new edge"
        );
    }

    #[tokio::test]
    async fn burst_is_one_batch() {
        let dir = tempfile::tempdir().unwrap();
        let service = GraphService::new(linked_folder(dir.path()));
        service.snapshot().await.expect("snapshot");
        let before = service.stats().batches_applied;

        let files: Vec<PathBuf> = (0..20)
            .map(|i| {
                let path = dir.path().join("src").join(format!("burst_{i:02}.rs"));
                std::fs::write(&path, format!("pub fn gv_burst_{i:02}() {{ gv_target_fn(); }}\n"))
                    .unwrap();
                path
            })
            .collect();
        for file in &files {
            service.notify_changed(std::slice::from_ref(file));
        }
        let map = service.snapshot().await.expect("snapshot");
        assert_eq!(service.stats().batches_applied, before + 1);
        for i in 0..20 {
            assert!(has_node(&map, &format!("src/burst_{i:02}.rs")));
        }
    }

    #[tokio::test]
    async fn dot_gaviero_paths_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let service = GraphService::new(linked_folder(dir.path()));
        service.snapshot().await.expect("snapshot");
        let before = service.stats().batches_applied;

        service.notify_changed(&[
            dir.path().join(".gaviero").join("code_graph.db-wal"),
            dir.path().join("target").join("x.rs"),
        ]);
        service.flush().await;
        assert_eq!(service.stats().batches_applied, before);
    }

    /// The writer holds the shared state; once the last handle is dropped
    /// the thread exits and releases it.
    #[tokio::test]
    async fn dropping_the_service_stops_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let service = GraphService::new(linked_folder(dir.path()));
        service.snapshot().await.expect("snapshot");
        let shared = Arc::downgrade(&service.inner.shared);
        drop(service);
        let deadline = Instant::now() + Duration::from_secs(2);
        while shared.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(shared.upgrade().is_none(), "the writer thread is still running");
    }

    #[tokio::test]
    async fn services_route_to_the_containing_folder() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let services = GraphServices::new(vec![linked_folder(a.path()), linked_folder(b.path())]);
        services.all()[0].snapshot().await.expect("snapshot a");
        services.all()[1].snapshot().await.expect("snapshot b");

        let new_file = b.path().join("src").join("only_in_b.rs");
        std::fs::write(&new_file, "pub fn gv_only_b() { gv_target_fn(); }\n").unwrap();
        services.notify_changed(std::slice::from_ref(&new_file));
        let map_b = services.all()[1].snapshot().await.expect("snapshot b");
        let map_a = services.all()[0].snapshot().await.expect("snapshot a");
        assert!(has_node(&map_b, "src/only_in_b.rs"));
        assert!(!has_node(&map_a, "src/only_in_b.rs"));
    }

    /// `with_query_reader` answers from the on-disk graph without starting the
    /// writer thread or running a migration.
    #[test]
    fn nav_query_reader_reads_a_built_graph() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn helper() {}\n").unwrap();
        graph_builder::build_graph(dir.path(), &[]).unwrap();

        let root = dir.path().to_path_buf();
        let service = GraphService::new(GraphFolder {
            db_path: graph_builder::graph_db_path(&root),
            root,
            excludes: Vec::new(),
            specificity: SpecificityConfig::default(),
        });
        assert!(service.current().is_none(), "the writer must not have started");

        let hits = service
            .with_query_reader(|store| store.definitions_by_name("helper", "src/lib.rs"))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].qualified_name, "src/lib.rs::helper");
        assert!(service.current().is_none(), "a read must not start the writer");
    }
}
