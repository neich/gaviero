//! SQLite-backed code knowledge graph.
//!
//! Stores code structure as nodes (File, Function, Struct, Trait, Enum, Test)
//! and typed edges (Calls, Imports, Implements, Defines, TestOf, doc references,
//! and future contract declarations).
//!
//! Supports incremental updates via file-hash diffing and blast-radius queries
//! via recursive CTE.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::rust_walk::RustFileFacts;

// ── Schema ───────────────────────────────────────────────────────

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS nodes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    qualified_name TEXT NOT NULL UNIQUE,
    file_path   TEXT NOT NULL,
    line_start  INTEGER,
    line_end    INTEGER,
    language    TEXT,
    file_hash   TEXT,
    updated_at  REAL NOT NULL DEFAULT (julianday('now'))
);

CREATE TABLE IF NOT EXISTS edges (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,
    source_qn   TEXT NOT NULL,
    target_qn   TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    line        INTEGER DEFAULT 0,
    updated_at  REAL NOT NULL DEFAULT (julianday('now'))
);

CREATE TABLE IF NOT EXISTS file_hashes (
    file_path   TEXT PRIMARY KEY,
    hash        TEXT NOT NULL,
    updated_at  REAL NOT NULL DEFAULT (julianday('now'))
);

CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file_path);
CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
CREATE INDEX IF NOT EXISTS idx_nodes_qn   ON nodes(qualified_name);
CREATE INDEX IF NOT EXISTS idx_edges_src  ON edges(source_qn);
CREATE INDEX IF NOT EXISTS idx_edges_tgt  ON edges(target_qn);
CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
CREATE INDEX IF NOT EXISTS idx_edges_file ON edges(file_path);

CREATE TABLE IF NOT EXISTS symbol_docs (
    qualified_name TEXT PRIMARY KEY,
    file_path      TEXT NOT NULL,
    file_hash      TEXT,
    signature      TEXT NOT NULL DEFAULT '',
    bounds           TEXT NOT NULL DEFAULT '',
    doc              TEXT NOT NULL DEFAULT '',
    role_summary     TEXT NOT NULL DEFAULT '',
    embedding        BLOB,
    updated_at       REAL NOT NULL DEFAULT (julianday('now'))
);

CREATE INDEX IF NOT EXISTS idx_symbol_docs_file ON symbol_docs(file_path);

CREATE TABLE IF NOT EXISTS graph_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Names a file re-exports (Rust `pub use`, TS `export … from`, Python
-- from-imports), so references through them resolve to the defining file
-- even when the re-exporting file did not change in this build.
-- `target` is language-specific (see repo_map::resolve).
CREATE TABLE IF NOT EXISTS reexports (
    file_path TEXT NOT NULL,
    name      TEXT NOT NULL,
    target    TEXT NOT NULL,
    glob      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_reexports_file ON reexports(file_path);

-- Rust type facts for receiver typing (repo_map::rust_walk::RustFileFacts).
-- Type texts are repo_map::types::TypeRef display forms, written relative
-- to file_path (resolved with that file's rust_imports).
CREATE TABLE IF NOT EXISTS type_fields (
    file_path TEXT NOT NULL,
    type_name TEXT NOT NULL,
    field     TEXT NOT NULL,
    type_text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_type_fields_file ON type_fields(file_path);
CREATE TABLE IF NOT EXISTS fn_returns (
    file_path TEXT NOT NULL,
    owner     TEXT NOT NULL,
    name      TEXT NOT NULL,
    ret_text  TEXT NOT NULL,
    is_async  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_fn_returns_file ON fn_returns(file_path);
CREATE TABLE IF NOT EXISTS type_impls (
    file_path  TEXT NOT NULL,
    type_name  TEXT NOT NULL,
    trait_path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_type_impls_file ON type_impls(file_path);
CREATE TABLE IF NOT EXISTS type_aliases (
    file_path TEXT NOT NULL,
    name      TEXT NOT NULL,
    params    TEXT NOT NULL,
    type_text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_type_aliases_file ON type_aliases(file_path);
CREATE TABLE IF NOT EXISTS rust_imports (
    file_path TEXT NOT NULL,
    local     TEXT NOT NULL,
    path      TEXT NOT NULL,
    glob      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_rust_imports_file ON rust_imports(file_path);
CREATE TABLE IF NOT EXISTS type_methods (
    file_path TEXT NOT NULL,
    owner     TEXT NOT NULL,
    name      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_type_methods_file ON type_methods(file_path);

-- Why each indexed path has the symbols it has (repo_map::extraction).
-- Absent row = unknown (a database built before this table existed).
CREATE TABLE IF NOT EXISTS file_extraction (
    file_path      TEXT PRIMARY KEY,
    outcome        TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    content_hash   TEXT,
    content_bytes  INTEGER,
    symbol_count   INTEGER NOT NULL DEFAULT 0,
    updated_at     REAL NOT NULL DEFAULT (julianday('now'))
);
";

/// The file-level impact CTE over `_impact_seed_files`.
///
/// `CROSS JOIN` pins the join order (SQLite never reorders it): affected
/// file → its nodes (`idx_nodes_file`) → edges pointing at them
/// (`idx_edges_tgt`). With plain JOINs the planner drove the recursive step
/// from `edges` through `idx_edges_kind` — every edge, then a rescan of
/// `affected` per edge — and a depth-1 query on a 441k-edge graph took 84 s
/// instead of 17 ms.
fn impact_cte_sql(max_depth: usize, edge_kinds: &str) -> String {
    format!(
        "WITH RECURSIVE affected(file, depth) AS (
            SELECT fp, 0 FROM _impact_seed_files
            UNION
            SELECT DISTINCT e.file_path, a.depth + 1
            FROM affected a
            CROSS JOIN nodes n ON n.file_path = a.file
            CROSS JOIN edges e ON e.target_qn = n.qualified_name
            WHERE a.depth < {max_depth}
              AND e.kind IN ({edge_kinds})
              AND e.file_path != a.file
        )
        SELECT DISTINCT file FROM affected"
    )
}

/// Cancellation for one graph query running on a blocking thread.
///
/// A `tokio::time::timeout` around `spawn_blocking` only stops *waiting*:
/// the query kept running, holding a worker thread at full CPU long after
/// the caller gave up. [`Self::cancel`] sets a flag that the query checks
/// between steps and interrupts the SQLite statement in flight
/// (`sqlite3_interrupt`), so the work actually ends. Attach the store the
/// query runs on with [`Self::attach`] before starting.
#[derive(Default)]
pub struct GraphCancel {
    cancelled: std::sync::atomic::AtomicBool,
    /// Held only to store or read the handle; `interrupt` just sets a flag
    /// inside SQLite, so nothing slow runs under the lock.
    handle: std::sync::Mutex<Option<rusqlite::InterruptHandle>>,
}

impl GraphCancel {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    /// Bind the connection the query will run on. Fails if the query was
    /// already cancelled, so a late start never runs at all.
    pub fn attach(&self, store: &GraphStore) -> Result<()> {
        let mut slot = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(store.interrupt_handle());
        drop(slot);
        self.check()
    }

    /// Stop the query: later [`Self::check`]s fail and the statement in
    /// flight, if any, returns an interrupt error.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let slot = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = slot.as_ref() {
            h.interrupt();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// `Err` once cancelled.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            anyhow::bail!("graph query cancelled");
        }
        Ok(())
    }
}

/// One non-file node, as the resolver sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    /// [`NodeKind::as_str`].
    pub kind: String,
    /// `impl:Type`, `impl:Type:Trait`, `trait:Name`, or `None`.
    pub owner: Option<String>,
}

/// One `file_extraction` row: the persisted extraction outcome of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileExtraction {
    pub file_path: String,
    /// [`crate::repo_map::extraction::ExtractionOutcome::as_str`].
    pub outcome: String,
    pub policy_version: String,
    /// SHA-256 of the content, when it could be read.
    pub content_hash: Option<String>,
    pub content_bytes: Option<i64>,
    pub symbol_count: i64,
}

/// C4: one-shot migration applied at every `open*` call.
///
/// Pre-C4 databases stored edges with either NULL `kind` (very old
/// schema) or the legacy aliases `'Contains'`/`'TestedBy'`. The
/// in-memory `EdgeKind::parse` already handles those aliases
/// transparently, but the persisted rows are still untyped from a
/// query-planner perspective: `mode=callers` wouldn't match a
/// NULL-kind edge.
///
/// We default any NULL `kind` to `'Imports'` (the recommended neutral
/// fallback per the plan) so the row keeps showing up under
/// `mode=all`/`mode=impact`. We *don't* delete legacy rows — the
/// next incremental graph build replaces them by qualified-name
/// upsert with the correct typed kind. The migration is idempotent:
/// a fully-migrated database has no NULL `kind` rows so the UPDATE is
/// a no-op.
///
/// **Post-upgrade workflow.** Run `gaviero-cli --graph` once after
/// updating to a build that includes C4 to repopulate every edge
/// with its proper typed kind. Without that re-scan, legacy rows
/// remain `'Imports'` and `mode=callers` / `mode=tests` /
/// `mode=implementations` queries will under-return — they still
/// surface the file via `mode=all`, but per-intent precision degrades
/// until the rebuild lands.
fn migrate_typed_edges(conn: &Connection) -> Result<()> {
    // `OR REPLACE`: once edges are unique, a defaulted row may collide
    // with an identical `Imports` row; keep one.
    conn.execute(
        "UPDATE OR REPLACE edges SET kind = 'Imports' WHERE kind IS NULL OR kind = ''",
        [],
    )
    .context("typed-edge migration: defaulting null kinds")?;
    Ok(())
}

/// Adds `nodes.owner` (the enclosing `impl` / `trait` of a method, see
/// [`crate::repo_map::Symbol::owner`]) to databases created before it
/// existed. Old rows stay NULL until the next extraction rewrites them.
fn migrate_node_owner(conn: &Connection) -> Result<()> {
    let has_owner = {
        let mut stmt = conn.prepare("PRAGMA table_info(nodes)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut found = false;
        for name in names {
            if name? == "owner" {
                found = true;
            }
        }
        found
    };
    if !has_owner {
        conn.execute_batch("ALTER TABLE nodes ADD COLUMN owner TEXT")
            .context("node migration: adding owner column")?;
    }
    Ok(())
}

/// One-time migration that makes edges unique on
/// `(kind, source_qn, target_qn, file_path)`.
///
/// Reference resolution names an edge's source `<file>::<called name>`, so
/// every call site of the same name in a file produced an identical row, and
/// `insert_edge`'s `INSERT OR IGNORE` had no unique key to ignore against.
/// On this repository the table held 441,314 edges, 63,785 distinct, which
/// multiplied the work of every impact query and of PageRank's edge load.
/// Keeps the first row of each group (its `line` is the first call site).
/// Runs once: a database that already has the index skips the scan.
fn migrate_unique_edges(conn: &Connection) -> Result<()> {
    let has_index: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_edges_unique')",
        [],
        |row| row.get(0),
    )?;
    if has_index {
        return Ok(());
    }
    conn.execute_batch(
        "BEGIN;
         DELETE FROM edges WHERE rowid NOT IN (
             SELECT MIN(rowid) FROM edges GROUP BY kind, source_qn, target_qn, file_path
         );
         CREATE UNIQUE INDEX idx_edges_unique ON edges(kind, source_qn, target_qn, file_path);
         COMMIT;",
    )
    .context("edge migration: removing duplicate edges")?;
    Ok(())
}

// ── Types ────────────────────────────────────────────────────────

/// Node kinds in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Function,
    Struct,
    Trait,
    Enum,
    Impl,
    Const,
    Class,
    Interface,
    Test,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Function => "Function",
            Self::Struct => "Struct",
            Self::Trait => "Trait",
            Self::Enum => "Enum",
            Self::Impl => "Impl",
            Self::Const => "Const",
            Self::Class => "Class",
            Self::Interface => "Interface",
            Self::Test => "Test",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "File" => Some(Self::File),
            "Function" => Some(Self::Function),
            "Struct" => Some(Self::Struct),
            "Trait" => Some(Self::Trait),
            "Enum" => Some(Self::Enum),
            "Impl" => Some(Self::Impl),
            "Const" => Some(Self::Const),
            "Class" => Some(Self::Class),
            "Interface" => Some(Self::Interface),
            "Test" => Some(Self::Test),
            _ => None,
        }
    }
}

/// Edge kinds in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Function/method calls another function.
    Calls,
    /// File imports/uses another module.
    Imports,
    /// Type implements a trait/interface or extends a class.
    Implements,
    /// File/module defines a symbol.
    Defines,
    /// Test file/function tests source code.
    TestOf,
    /// Doc comment/string on source references a symbol.
    ReferencesDocstringOf,
    /// Tier D1 placeholder for promoted contracts from node docs.
    DeclaresContractWith,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "Calls",
            Self::Imports => "Imports",
            Self::Implements => "Implements",
            Self::Defines => "Defines",
            Self::TestOf => "TestOf",
            Self::ReferencesDocstringOf => "ReferencesDocstringOf",
            Self::DeclaresContractWith => "DeclaresContractWith",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Calls" => Some(Self::Calls),
            "Imports" => Some(Self::Imports),
            "Implements" => Some(Self::Implements),
            "Defines" | "Contains" => Some(Self::Defines),
            "TestOf" | "TestedBy" => Some(Self::TestOf),
            "ReferencesDocstringOf" => Some(Self::ReferencesDocstringOf),
            "DeclaresContractWith" => Some(Self::DeclaresContractWith),
            _ => None,
        }
    }
}

/// Query intent for graph blast-radius traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlastRadiusMode {
    Impact,
    Callers,
    Tests,
    Implementations,
    #[default]
    All,
}

impl BlastRadiusMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "impact" => Self::Impact,
            "callers" => Self::Callers,
            "tests" => Self::Tests,
            "implementations" => Self::Implementations,
            "all" | "" => Self::All,
            _ => Self::All,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Impact => "impact",
            Self::Callers => "callers",
            Self::Tests => "tests",
            Self::Implementations => "implementations",
            Self::All => "all",
        }
    }

    fn edge_kind_sql_list(&self) -> &'static str {
        match self {
            Self::Impact => {
                "'Calls','Imports','Implements','Defines','TestOf',\
                 'ReferencesDocstringOf','DeclaresContractWith','Contains','TestedBy'"
            }
            Self::Callers => "'Calls'",
            Self::Tests => "'TestOf','TestedBy'",
            Self::Implementations => "'Implements'",
            Self::All => {
                "'Calls','Imports','Implements','Defines','TestOf',\
                 'ReferencesDocstringOf','DeclaresContractWith','Contains','TestedBy'"
            }
        }
    }

    /// Per-intent edge weight preset. Returned weight is `0.0` when an edge
    /// kind should be excluded from PageRank propagation, `1.0` when it
    /// dominates, and an intermediate value when it contributes but is
    /// outranked by a more relevant kind for the intent.
    ///
    /// Equivalent to `EdgeWeights::default_for(self).weight(kind)` —
    /// provided as a thin shim so call sites that don't need to honor
    /// user overrides stay terse.
    pub fn edge_weight(&self, kind: EdgeKind) -> f64 {
        EdgeWeights::default_for(*self).weight(kind)
    }
}

/// Per-intent edge weight map for PageRank propagation.
///
/// `EdgeWeights::default_for(mode)` returns the plan's recommended
/// preset; an embedding application loading
/// `repoMap.edges.weights.<intent>` from settings can override
/// individual kinds via [`Self::set`]. Unknown kinds (or omitted
/// settings entries) keep the preset value.
///
/// Plan presets:
/// - `Impact`:           Calls=1.0, Implements=0.9, Defines=0.8,
///   Imports=0.5, TestOf=0.3, others=0.2
/// - `Callers`:          Calls=1.0, others=0.0
/// - `Tests`:            TestOf=1.0, others=0.0
/// - `Implementations`:  Implements=1.0, others=0.0
/// - `All`:              all=1.0
#[derive(Debug, Clone, Copy)]
pub struct EdgeWeights {
    pub calls: f64,
    pub imports: f64,
    pub implements: f64,
    pub defines: f64,
    pub test_of: f64,
    pub references_docstring_of: f64,
    pub declares_contract_with: f64,
}

impl EdgeWeights {
    pub fn default_for(mode: BlastRadiusMode) -> Self {
        match mode {
            BlastRadiusMode::Impact => Self {
                calls: 1.0,
                implements: 0.9,
                defines: 0.8,
                imports: 0.5,
                test_of: 0.3,
                references_docstring_of: 0.2,
                declares_contract_with: 0.2,
            },
            BlastRadiusMode::Callers => Self {
                calls: 1.0,
                ..Self::zero()
            },
            BlastRadiusMode::Tests => Self {
                test_of: 1.0,
                ..Self::zero()
            },
            BlastRadiusMode::Implementations => Self {
                implements: 1.0,
                ..Self::zero()
            },
            BlastRadiusMode::All => Self {
                calls: 1.0,
                imports: 1.0,
                implements: 1.0,
                defines: 1.0,
                test_of: 1.0,
                references_docstring_of: 1.0,
                declares_contract_with: 1.0,
            },
        }
    }

    fn zero() -> Self {
        Self {
            calls: 0.0,
            imports: 0.0,
            implements: 0.0,
            defines: 0.0,
            test_of: 0.0,
            references_docstring_of: 0.0,
            declares_contract_with: 0.0,
        }
    }

    pub fn weight(&self, kind: EdgeKind) -> f64 {
        match kind {
            EdgeKind::Calls => self.calls,
            EdgeKind::Imports => self.imports,
            EdgeKind::Implements => self.implements,
            EdgeKind::Defines => self.defines,
            EdgeKind::TestOf => self.test_of,
            EdgeKind::ReferencesDocstringOf => self.references_docstring_of,
            EdgeKind::DeclaresContractWith => self.declares_contract_with,
        }
    }

    /// Merge a settings-supplied JSON object into the preset. The
    /// object's keys must match the lowercase EdgeKind names
    /// (`"calls"`, `"imports"`, `"implements"`, `"defines"`,
    /// `"testOf"`, `"referencesDocstringOf"`,
    /// `"declaresContractWith"`); each value is an `f64` clamped to
    /// `[0.0, 1.0]`. Unknown keys are ignored — embedding apps should
    /// log a warning if they want to surface typos.
    pub fn apply_overrides(&mut self, overrides: &serde_json::Map<String, serde_json::Value>) {
        let pull = |key: &str| -> Option<f64> {
            overrides
                .get(key)
                .and_then(|v| v.as_f64())
                .map(|f| f.clamp(0.0, 1.0))
        };
        if let Some(v) = pull("calls") {
            self.calls = v;
        }
        if let Some(v) = pull("imports") {
            self.imports = v;
        }
        if let Some(v) = pull("implements") {
            self.implements = v;
        }
        if let Some(v) = pull("defines") {
            self.defines = v;
        }
        if let Some(v) = pull("testOf") {
            self.test_of = v;
        }
        if let Some(v) = pull("referencesDocstringOf") {
            self.references_docstring_of = v;
        }
        if let Some(v) = pull("declaresContractWith") {
            self.declares_contract_with = v;
        }
    }
}

/// A row in the `symbol_docs` enrichment sidecar (S2.1 / S2.2).
#[derive(Debug, Clone, Default)]
pub struct SymbolDoc {
    pub qualified_name: String,
    pub file_path: String,
    pub file_hash: Option<String>,
    pub signature: String,
    pub bounds: String,
    pub doc: String,
    pub role_summary: String,
    pub embedding: Option<Vec<f32>>,
}

/// A node stored in the graph.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub language: Option<String>,
}

/// Result of a blast-radius query (V9 §4 `ImpactSummary` — typed return for
/// graph impact queries; planner consumers receive structured data, not
/// pre-rendered prompt strings).
///
/// Renamed from `ImpactResult` in M3 to align with V9 §4 spec naming.
#[derive(Debug, Clone, Default)]
pub struct ImpactSummary {
    /// Files directly changed.
    pub changed_files: Vec<String>,
    /// All files transitively affected (including changed files).
    pub affected_files: Vec<String>,
    /// Test files in the affected set.
    pub affected_tests: Vec<String>,
    /// Files in scope that have no test coverage.
    pub test_gaps: Vec<String>,
    /// Whether the result was truncated due to max_nodes limit.
    pub truncated: bool,
}

fn embedding_to_blob(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Result<Vec<f32>> {
    if !blob.len().is_multiple_of(4) {
        anyhow::bail!("invalid embedding blob length {}", blob.len());
    }
    blob.chunks_exact(4)
        .map(|chunk| {
            let bytes: [u8; 4] = chunk.try_into().expect("chunk length checked");
            Ok(f32::from_le_bytes(bytes))
        })
        .collect()
}

// ── GraphStore ───────────────────────────────────────────────────

/// SQLite-backed code knowledge graph.
pub struct GraphStore {
    conn: Connection,
}

impl GraphStore {
    /// Open or create a graph database at `db_path`.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating graph db dir: {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening graph db: {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate_typed_edges(&conn)?;
        migrate_unique_edges(&conn)?;
        migrate_node_owner(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory graph database (for tests).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate_typed_edges(&conn)?;
        migrate_unique_edges(&conn)?;
        migrate_node_owner(&conn)?;
        Ok(Self { conn })
    }

    /// Record a node's owner (`impl:Type`, `impl:Type:Trait`, `trait:Name`).
    pub fn set_node_owner(&self, qualified_name: &str, owner: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE nodes SET owner = ?2 WHERE qualified_name = ?1",
            params![qualified_name, owner],
        )?;
        Ok(())
    }

    /// Every non-file definition, for building the resolver's indexes.
    pub fn all_definitions(&self) -> Result<Vec<Definition>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, qualified_name, file_path, kind, owner FROM nodes WHERE kind != 'File'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Definition {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                file_path: row.get(2)?,
                kind: row.get(3)?,
                owner: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete a file's reference edges (everything but `Defines`), so its
    /// references can be resolved again without re-extracting its symbols.
    pub fn delete_reference_edges(&self, file_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM edges WHERE file_path = ?1 AND kind != 'Defines'",
            params![file_path],
        )?;
        Ok(())
    }

    /// Handle that aborts whatever statement this connection is running,
    /// from another thread (`sqlite3_interrupt`). The interrupted call
    /// returns an error. See [`GraphCancel`].
    pub fn interrupt_handle(&self) -> rusqlite::InterruptHandle {
        self.conn.get_interrupt_handle()
    }

    // ── Node operations ──────────────────────────────────────────

    /// Insert or update a node. Returns the node id.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_node(
        &self,
        kind: NodeKind,
        name: &str,
        qualified_name: &str,
        file_path: &str,
        line_start: Option<i64>,
        line_end: Option<i64>,
        language: Option<&str>,
        file_hash: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO nodes (kind, name, qualified_name, file_path, line_start, line_end, language, file_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(qualified_name) DO UPDATE SET
                kind=excluded.kind, name=excluded.name, file_path=excluded.file_path,
                line_start=excluded.line_start, line_end=excluded.line_end,
                language=excluded.language, file_hash=excluded.file_hash,
                updated_at=julianday('now')",
            params![kind.as_str(), name, qualified_name, file_path, line_start, line_end, language, file_hash],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get all nodes for a given file path.
    pub fn nodes_for_file(&self, file_path: &str) -> Result<Vec<GraphNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end, language
             FROM nodes WHERE file_path = ?1",
        )?;
        let rows = stmt.query_map(params![file_path], |row| {
            Ok(GraphNode {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                language: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Whether a node with this `qualified_name` exists.
    pub fn has_node(&self, qualified_name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(1) FROM nodes WHERE qualified_name = ?1",
            params![qualified_name],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    // ── Symbol enrichment sidecar (S2.1 / S2.2) ─────────────────

    /// Insert or update one `symbol_docs` row.
    pub fn upsert_symbol_doc(&self, doc: &SymbolDoc) -> Result<()> {
        let embedding_blob = doc.embedding.as_ref().map(|v| embedding_to_blob(v));
        self.conn.execute(
            "INSERT INTO symbol_docs (
                qualified_name, file_path, file_hash, signature, bounds,
                doc, role_summary, embedding, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, julianday('now'))
             ON CONFLICT(qualified_name) DO UPDATE SET
                file_path = excluded.file_path,
                file_hash = excluded.file_hash,
                signature = excluded.signature,
                bounds = excluded.bounds,
                doc = excluded.doc,
                role_summary = excluded.role_summary,
                embedding = excluded.embedding,
                updated_at = julianday('now')",
            params![
                doc.qualified_name,
                doc.file_path,
                doc.file_hash,
                doc.signature,
                doc.bounds,
                doc.doc,
                doc.role_summary,
                embedding_blob,
            ],
        )?;
        Ok(())
    }

    /// Fetch enrichment for one symbol, if present.
    pub fn symbol_doc(&self, qualified_name: &str) -> Result<Option<SymbolDoc>> {
        let mut stmt = self.conn.prepare(
            "SELECT qualified_name, file_path, file_hash, signature, bounds,
                    doc, role_summary, embedding
             FROM symbol_docs WHERE qualified_name = ?1",
        )?;
        let mut rows = stmt.query(params![qualified_name])?;
        if let Some(row) = rows.next()? {
            let blob: Option<Vec<u8>> = row.get(7)?;
            Ok(Some(SymbolDoc {
                qualified_name: row.get(0)?,
                file_path: row.get(1)?,
                file_hash: row.get(2)?,
                signature: row.get(3)?,
                bounds: row.get(4)?,
                doc: row.get(5)?,
                role_summary: row.get(6)?,
                embedding: blob.map(|b| blob_to_embedding(&b)).transpose()?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Count rows in `symbol_docs`.
    pub fn symbol_doc_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM symbol_docs", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Load every enriched symbol row (for brute-force vector search).
    pub fn all_symbol_docs(&self) -> Result<Vec<SymbolDoc>> {
        let mut stmt = self.conn.prepare(
            "SELECT qualified_name, file_path, file_hash, signature, bounds,
                    doc, role_summary, embedding
             FROM symbol_docs",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let blob: Option<Vec<u8>> = row.get(7)?;
            out.push(SymbolDoc {
                qualified_name: row.get(0)?,
                file_path: row.get(1)?,
                file_hash: row.get(2)?,
                signature: row.get(3)?,
                bounds: row.get(4)?,
                doc: row.get(5)?,
                role_summary: row.get(6)?,
                embedding: blob.map(|b| blob_to_embedding(&b)).transpose()?,
            });
        }
        Ok(out)
    }

    /// Read one `graph_meta` value — sidecar-level metadata such as the
    /// `symbol_embedder` stamp written by enrichment (G2 / OD-2).
    pub fn graph_meta(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM graph_meta WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Upsert one `graph_meta` value.
    pub fn set_graph_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO graph_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// `Implements` edges whose target is `trait_qn` (impl sources).
    pub fn implementation_qns_for_trait(&self, trait_qn: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT source_qn FROM edges
             WHERE kind = 'Implements' AND target_qn = ?1",
        )?;
        let rows = stmt.query_map(params![trait_qn], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete everything stored for a file path: nodes, edges, hash,
    /// enrichment rows and extraction status.
    pub fn delete_file(&self, file_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM symbol_docs WHERE file_path = ?1",
            params![file_path],
        )?;
        self.conn.execute(
            "DELETE FROM file_extraction WHERE file_path = ?1",
            params![file_path],
        )?;
        self.delete_file_graph(file_path)
    }

    /// Replace a file's re-exports: `(name, target, glob)`.
    pub fn replace_reexports(&self, file_path: &str, rows: &[(String, String, bool)]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM reexports WHERE file_path = ?1",
            params![file_path],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO reexports (file_path, name, target, glob) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (name, target, glob) in rows {
            stmt.execute(params![file_path, name, target, *glob as i64])?;
        }
        Ok(())
    }

    /// What other files can resolve against in `file_path`: its
    /// definitions (name, kind, owner), re-exports and Rust type facts
    /// (field types, return types, impl headers, aliases, `use` bindings —
    /// a field's type is resolved with its file's imports), in a stable
    /// order. Two equal signatures mean no other file's resolution can
    /// change.
    pub fn definition_signature(&self, file_path: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT 'd:' || name || ':' || kind || ':' || COALESCE(owner, '')
               FROM nodes WHERE file_path = ?1 AND kind != 'File'
             UNION
             SELECT 'r:' || name || ':' || target || ':' || glob
               FROM reexports WHERE file_path = ?1
             UNION
             SELECT 'f:' || type_name || ':' || field || ':' || type_text
               FROM type_fields WHERE file_path = ?1
             UNION
             SELECT 'ret:' || owner || ':' || name || ':' || ret_text || ':' || is_async
               FROM fn_returns WHERE file_path = ?1
             UNION
             SELECT 'i:' || type_name || ':' || trait_path
               FROM type_impls WHERE file_path = ?1
             UNION
             SELECT 'a:' || name || ':' || params || ':' || type_text
               FROM type_aliases WHERE file_path = ?1
             UNION
             SELECT 'u:' || local || ':' || path || ':' || glob
               FROM rust_imports WHERE file_path = ?1
             UNION
             SELECT 'm:' || owner || ':' || name
               FROM type_methods WHERE file_path = ?1
             ORDER BY 1",
        )?;
        let rows = stmt.query_map(params![file_path], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every re-export: `(file, name, target, glob)`.
    pub fn all_reexports(&self) -> Result<Vec<(String, String, String, bool)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path, name, target, glob FROM reexports")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)? != 0))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete a path's nodes, edges and hash, keeping its enrichment rows
    /// and extraction status. For re-extracting *unchanged* content after an
    /// extraction-policy change: enrichment is keyed by content and stays
    /// valid; [`Self::prune_symbol_docs_for`] drops rows whose symbol the
    /// new extraction no longer emits.
    pub fn delete_file_graph(&self, file_path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM edges WHERE file_path = ?1", params![file_path])?;
        self.conn
            .execute("DELETE FROM nodes WHERE file_path = ?1", params![file_path])?;
        self.conn.execute(
            "DELETE FROM file_hashes WHERE file_path = ?1",
            params![file_path],
        )?;
        self.conn.execute(
            "DELETE FROM reexports WHERE file_path = ?1",
            params![file_path],
        )?;
        self.delete_type_facts(file_path)?;
        Ok(())
    }

    fn delete_type_facts(&self, file_path: &str) -> Result<()> {
        for table in ["type_fields", "fn_returns", "type_impls", "type_aliases", "rust_imports", "type_methods"] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE file_path = ?1"),
                params![file_path],
            )?;
        }
        Ok(())
    }

    /// Replace a Rust file's type facts.
    pub fn replace_type_facts(&self, file_path: &str, facts: &RustFileFacts) -> Result<()> {
        self.delete_type_facts(file_path)?;
        let mut s = self.conn.prepare_cached(
            "INSERT INTO type_fields (file_path, type_name, field, type_text) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (ty, field, text) in &facts.fields {
            s.execute(params![file_path, ty, field, text])?;
        }
        let mut s = self.conn.prepare_cached(
            "INSERT INTO fn_returns (file_path, owner, name, ret_text, is_async) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (owner, name, ret, is_async) in &facts.returns {
            s.execute(params![file_path, owner, name, ret, *is_async as i64])?;
        }
        let mut s = self
            .conn
            .prepare_cached("INSERT INTO type_impls (file_path, type_name, trait_path) VALUES (?1, ?2, ?3)")?;
        for (ty, tr) in &facts.impls {
            s.execute(params![file_path, ty, tr])?;
        }
        let mut s = self.conn.prepare_cached(
            "INSERT INTO type_aliases (file_path, name, params, type_text) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (name, ps, text) in &facts.aliases {
            s.execute(params![file_path, name, ps, text])?;
        }
        let mut s = self
            .conn
            .prepare_cached("INSERT INTO rust_imports (file_path, local, path, glob) VALUES (?1, ?2, ?3, ?4)")?;
        for (local, path, glob) in &facts.imports {
            s.execute(params![file_path, local, path, *glob as i64])?;
        }
        let mut s = self
            .conn
            .prepare_cached("INSERT INTO type_methods (file_path, owner, name) VALUES (?1, ?2, ?3)")?;
        for (owner, name) in &facts.methods {
            s.execute(params![file_path, owner, name])?;
        }
        Ok(())
    }

    /// Every file's type facts, keyed by file.
    pub fn all_type_facts(&self) -> Result<HashMap<String, RustFileFacts>> {
        let mut out: HashMap<String, RustFileFacts> = HashMap::new();
        let mut s = self.conn.prepare("SELECT file_path, type_name, field, type_text FROM type_fields")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))? {
            let (f, v) = row?;
            out.entry(f).or_default().fields.push(v);
        }
        let mut s = self.conn.prepare("SELECT file_path, owner, name, ret_text, is_async FROM fn_returns")?;
        for row in s.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0)))
        })? {
            let (f, v) = row?;
            out.entry(f).or_default().returns.push(v);
        }
        let mut s = self.conn.prepare("SELECT file_path, type_name, trait_path FROM type_impls")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))? {
            let (f, v) = row?;
            out.entry(f).or_default().impls.push(v);
        }
        let mut s = self.conn.prepare("SELECT file_path, name, params, type_text FROM type_aliases")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))? {
            let (f, v) = row?;
            out.entry(f).or_default().aliases.push(v);
        }
        let mut s = self.conn.prepare("SELECT file_path, local, path, glob FROM rust_imports")?;
        for row in s.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)))
        })? {
            let (f, v) = row?;
            out.entry(f).or_default().imports.push(v);
        }
        let mut s = self.conn.prepare("SELECT file_path, owner, name FROM type_methods")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))? {
            let (f, v) = row?;
            out.entry(f).or_default().methods.push(v);
        }
        Ok(out)
    }

    /// Drop `symbol_docs` rows of `file_path` whose qualified name is no
    /// longer a node of that file.
    pub fn prune_symbol_docs_for(&self, file_path: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM symbol_docs WHERE file_path = ?1
               AND qualified_name NOT IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)",
            params![file_path],
        )?;
        Ok(n)
    }

    /// Upsert a path's extraction status.
    pub fn set_file_extraction(&self, rec: &FileExtraction) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_extraction
                (file_path, outcome, policy_version, content_hash, content_bytes, symbol_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, julianday('now'))
             ON CONFLICT(file_path) DO UPDATE SET
                outcome = excluded.outcome,
                policy_version = excluded.policy_version,
                content_hash = excluded.content_hash,
                content_bytes = excluded.content_bytes,
                symbol_count = excluded.symbol_count,
                updated_at = excluded.updated_at",
            params![
                rec.file_path,
                rec.outcome,
                rec.policy_version,
                rec.content_hash,
                rec.content_bytes,
                rec.symbol_count
            ],
        )?;
        Ok(())
    }

    /// A path's extraction status; `None` when unknown.
    pub fn file_extraction(&self, file_path: &str) -> Result<Option<FileExtraction>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_path, outcome, policy_version, content_hash, content_bytes, symbol_count
             FROM file_extraction WHERE file_path = ?1",
        )?;
        let mut rows = stmt.query(params![file_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(FileExtraction {
                file_path: row.get(0)?,
                outcome: row.get(1)?,
                policy_version: row.get(2)?,
                content_hash: row.get(3)?,
                content_bytes: row.get(4)?,
                symbol_count: row.get(5)?,
            })),
            None => Ok(None),
        }
    }

    /// Every path with an extraction status.
    pub fn all_extraction_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT file_path FROM file_extraction")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ── Edge operations ──────────────────────────────────────────

    /// Insert an edge.
    pub fn insert_edge(
        &self,
        kind: EdgeKind,
        source_qn: &str,
        target_qn: &str,
        file_path: &str,
        line: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO edges (kind, source_qn, target_qn, file_path, line)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![kind.as_str(), source_qn, target_qn, file_path, line],
        )?;
        Ok(())
    }

    // ── Hash tracking ────────────────────────────────────────────

    /// Get the stored hash for a file, or None if not tracked.
    pub fn get_file_hash(&self, file_path: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash FROM file_hashes WHERE file_path = ?1")?;
        let result = stmt.query_row(params![file_path], |row| row.get::<_, String>(0));
        match result {
            Ok(h) => Ok(Some(h)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Store or update the hash for a file.
    pub fn set_file_hash(&self, file_path: &str, hash: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_hashes (file_path, hash)
             VALUES (?1, ?2)
             ON CONFLICT(file_path) DO UPDATE SET hash=excluded.hash, updated_at=julianday('now')",
            params![file_path, hash],
        )?;
        Ok(())
    }

    /// Return all tracked file paths and their hashes.
    pub fn all_file_hashes(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path, hash FROM file_hashes")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ── Stats ────────────────────────────────────────────────────

    /// Count total nodes and edges.
    pub fn stats(&self) -> Result<(usize, usize)> {
        let nodes: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let edges: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        Ok((nodes as usize, edges as usize))
    }

    // ── Blast-radius query ───────────────────────────────────────

    /// Compute the impact radius: given changed files, find all transitively
    /// affected files via reverse edge traversal (BFS at the file level).
    ///
    /// Uses a SQLite recursive CTE for performance on large graphs.
    /// The traversal works as follows:
    /// - Seed: all files in `changed_files`
    /// - Expand: for each file in the frontier, find other files that have
    ///   edges pointing INTO this file (i.e. files that depend on this file)
    /// - The edge table stores `source_qn` (the caller/importer) and
    ///   `target_qn` (the callee/imported). When a file changes, we want
    ///   files whose edges TARGET nodes in the changed file → those files
    ///   are the dependents.
    pub fn impact_radius(&self, changed_files: &[&str], max_depth: usize) -> Result<ImpactSummary> {
        self.impact_radius_with_mode(changed_files, max_depth, BlastRadiusMode::All)
    }

    /// Mode-aware blast-radius traversal over typed edges.
    pub fn impact_radius_with_mode(
        &self,
        changed_files: &[&str],
        max_depth: usize,
        mode: BlastRadiusMode,
    ) -> Result<ImpactSummary> {
        if changed_files.is_empty() {
            return Ok(ImpactSummary::default());
        }
        let edge_kinds = mode.edge_kind_sql_list();

        // 1. Create temp table for seed files
        self.conn.execute(
            "CREATE TEMP TABLE IF NOT EXISTS _impact_seed_files (fp TEXT PRIMARY KEY)",
            [],
        )?;
        self.conn.execute("DELETE FROM _impact_seed_files", [])?;
        for cf in changed_files {
            self.conn.execute(
                "INSERT OR IGNORE INTO _impact_seed_files (fp) VALUES (?1)",
                params![cf],
            )?;
        }

        // 2. Recursive CTE at the file level.
        //    Edge semantics: edge(source_qn, target_qn) means "source references target".
        //    When target's file changes, source's file is affected.
        //    So: find edges where target_qn belongs to a seed file → the edge's file_path
        //    (which is the source file) is affected.
        let cte_sql = impact_cte_sql(max_depth, &edge_kinds);
        let mut stmt = self.conn.prepare(&cte_sql)?;
        let all_files: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        // 3. Partition results
        let mut affected_files = Vec::new();
        let mut affected_tests = Vec::new();

        for file in &all_files {
            affected_files.push(file.clone());
            if file.contains("test") || file.starts_with("tests/") {
                affected_tests.push(file.clone());
            }
        }

        // 4. Find test gaps: changed files that have no edge from any test file
        let mut test_gaps = Vec::new();
        for cf in changed_files {
            let has_test: bool = self.conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM edges e
                    JOIN nodes n ON n.qualified_name = e.target_qn
                    WHERE n.file_path = ?1
                      AND (e.kind IN ('TestOf','TestedBy') OR e.file_path LIKE '%test%')
                )",
                params![cf],
                |row| row.get(0),
            )?;
            if !has_test {
                test_gaps.push(cf.to_string());
            }
        }

        Ok(ImpactSummary {
            changed_files: changed_files.iter().map(|s| s.to_string()).collect(),
            affected_files,
            affected_tests,
            test_gaps,
            truncated: false,
        })
    }

    /// C3: format impact result with per-file rank/specificity badges.
    ///
    /// `ranks` maps each file path to `(rank_score, specificity)` —
    /// produced by [`crate::repo_map::rank_files_with_mode`] or
    /// [`crate::repo_map::rank_files_with_weights`]. When `ranks`
    /// covers the affected set, each emitted line gets a `[sp 0.92]`
    /// suffix so the TUI panel and chat injection both surface the
    /// HippoRAG specificity score the agent is paying for.
    pub fn format_impact_for_prompt_ranked(
        result: &ImpactSummary,
        ranks: &std::collections::HashMap<String, (f64, f64)>,
    ) -> String {
        let badge = |path: &str| -> String {
            ranks
                .get(path)
                .map(|(_, sp)| format!("{path} (s{sp:.2})"))
                .unwrap_or_else(|| path.to_string())
        };

        let mut lines = Vec::new();
        lines.push("Imp:".to_string());

        if !result.changed_files.is_empty() {
            let labelled: Vec<String> = result.changed_files.iter().map(|p| badge(p)).collect();
            lines.push(format!("chg: {}", labelled.join(", ")));
        }

        let non_changed: Vec<String> = result
            .affected_files
            .iter()
            .filter(|f| !result.changed_files.contains(f))
            .map(|p| badge(p))
            .collect();
        if !non_changed.is_empty() {
            lines.push(format!("dep: {}", non_changed.join(", ")));
        }

        if !result.affected_tests.is_empty() {
            let labelled: Vec<String> = result.affected_tests.iter().map(|p| badge(p)).collect();
            lines.push(format!("tst: {}", labelled.join(", ")));
        }

        if !result.test_gaps.is_empty() {
            lines.push(format!("gaps: {}", result.test_gaps.join(", ")));
        }

        lines.join("\n")
    }

    /// Format impact result as a prompt-friendly string.
    pub fn format_impact_for_prompt(result: &ImpactSummary) -> String {
        let mut lines = Vec::new();
        lines.push("Imp:".to_string());

        if !result.changed_files.is_empty() {
            lines.push(format!("chg: {}", result.changed_files.join(", ")));
        }

        let non_changed: Vec<&str> = result
            .affected_files
            .iter()
            .filter(|f| !result.changed_files.contains(f))
            .map(|s| s.as_str())
            .collect();
        if !non_changed.is_empty() {
            lines.push(format!("dep: {}", non_changed.join(", ")));
        }

        if !result.affected_tests.is_empty() {
            lines.push(format!("tst: {}", result.affected_tests.join(", ")));
        }

        if !result.test_gaps.is_empty() {
            lines.push(format!("gaps: {}", result.test_gaps.join(", ")));
        }

        lines.join("\n")
    }

    /// PUSH→PULL Phase 2: a tiny (~150-token) count summary of an impact
    /// radius, for the strong-tier chat first turn. Replaces the full ranked
    /// render: it states the shape of the blast radius (changed / dependent /
    /// missing-tests counts), names `blast_radius(path)` so the model knows how
    /// to pull the ranked detail, and keeps the missing-tests filenames
    /// verbatim (the cheapest, highest-value signal) up to the char budget.
    ///
    /// Returns an empty string when there is nothing to report (no changed and
    /// no affected files), so the caller injects nothing for an empty buffer.
    pub fn format_impact_summary(result: &ImpactSummary) -> String {
        let changed = result.changed_files.len();
        let dependents = result
            .affected_files
            .iter()
            .filter(|f| !result.changed_files.contains(f))
            .count();
        if changed == 0 && dependents == 0 {
            return String::new();
        }
        let gaps = result.test_gaps.len();
        // ~150-token cap (project convention: ~4 chars/token).
        const MAX_CHARS: usize = 600;
        let mut s = format!(
            "Imp: {changed} changed, {dependents} dependent file(s) affected, \
             {gaps} missing tests — call blast_radius(path) for the ranked detail."
        );
        // Keep the missing-tests filenames verbatim; truncate the *list* (never
        // the count) to stay under budget.
        if !result.test_gaps.is_empty() {
            let mut listed: Vec<&str> = Vec::new();
            let mut used = s.len() + "\nno tests: ".len();
            let mut overflow = 0usize;
            for f in &result.test_gaps {
                // +2 for the ", " separator.
                if used + f.len() + 2 <= MAX_CHARS {
                    used += f.len() + 2;
                    listed.push(f.as_str());
                } else {
                    overflow += 1;
                }
            }
            if !listed.is_empty() {
                s.push_str("\nno tests: ");
                s.push_str(&listed.join(", "));
                if overflow > 0 {
                    s.push_str(&format!(" (+{overflow} more)"));
                }
            }
        }
        s
    }

    /// Begin a graph build that rolls back on error or unwinding.
    pub(crate) fn build_transaction(&self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.conn.unchecked_transaction()?)
    }

    /// Begin a transaction for batch operations.
    pub fn begin_transaction(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN TRANSACTION")?;
        Ok(())
    }

    /// Commit a transaction.
    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    /// C4: project the persisted graph into file-level (source, target,
    /// kind) triples for in-memory PageRank.
    ///
    /// `source` is the edge's `file_path` (where the reference lives);
    /// `target` is the file containing the symbol matched by
    /// `target_qn`. Self-edges are filtered. Used by the MCP
    /// `blast_radius` handler to apply per-intent edge weights via
    /// `BlastRadiusMode::edge_weight`.
    pub fn file_edges(&self) -> Result<Vec<(String, String, EdgeKind)>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.file_path, n.file_path, e.kind
             FROM edges e
             JOIN nodes n ON n.qualified_name = e.target_qn",
        )?;
        let rows = stmt.query_map([], |row| {
            let src: String = row.get(0)?;
            let tgt: String = row.get(1)?;
            let kind_s: String = row.get(2)?;
            Ok((src, tgt, kind_s))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (src, tgt, kind_s) = r?;
            if src == tgt {
                continue;
            }
            if let Some(kind) = EdgeKind::parse(&kind_s) {
                out.push((src, tgt, kind));
            }
        }
        Ok(out)
    }

    /// All distinct file paths known to the graph (from both `nodes`
    /// and `edges`). The MCP handler uses this as the node set when
    /// projecting into a `DiGraph` for PageRank.
    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT file_path FROM nodes
             UNION
             SELECT DISTINCT file_path FROM edges",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Per-symbol document frequency: how many distinct files reference
    /// a symbol with the same name. C3 callers use this to derive a
    /// file-level specificity map without rebuilding the in-memory
    /// `RepoMap` graph from disk.
    pub fn symbol_document_frequency(&self) -> Result<std::collections::HashMap<String, usize>> {
        let mut stmt = self
            .conn
            .prepare("SELECT n.name, COUNT(DISTINCT n.file_path) FROM nodes n GROUP BY n.name")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        let mut out = std::collections::HashMap::new();
        for r in rows {
            let (name, df) = r?;
            out.insert(name, df);
        }
        Ok(out)
    }

    /// Names of all symbols defined in `file_path`. Combined with
    /// [`Self::symbol_document_frequency`] this lets the MCP handler
    /// compute per-file specificity without loading the source files.
    pub fn symbols_in_file(&self, file_path: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM nodes WHERE file_path = ?1")?;
        let rows = stmt.query_map(params![file_path], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Resolve a symbol name to its qualified name(s) across the graph.
    /// Used for cross-file reference resolution.
    pub fn resolve_symbol(&self, name: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT qualified_name, file_path FROM nodes WHERE name = ?1")?;
        let rows = stmt.query_map(params![name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_docs_round_trip() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "src/lib.rs::foo",
                "src/lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        let doc = SymbolDoc {
            qualified_name: "src/lib.rs::foo".into(),
            file_path: "src/lib.rs".into(),
            file_hash: Some("abc".into()),
            signature: "fn foo()".into(),
            bounds: String::new(),
            doc: "Does foo things".into(),
            role_summary: "function".into(),
            embedding: Some(vec![1.0, 0.0, 0.5]),
        };
        store.upsert_symbol_doc(&doc).unwrap();
        let loaded = store.symbol_doc("src/lib.rs::foo").unwrap().unwrap();
        assert_eq!(loaded.signature, "fn foo()");
        assert_eq!(loaded.embedding, doc.embedding);
        store.delete_file("src/lib.rs").unwrap();
        assert!(store.symbol_doc("src/lib.rs::foo").unwrap().is_none());
    }

    #[test]
    fn create_and_query_nodes() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "src/lib.rs::foo",
                "src/lib.rs",
                Some(10),
                Some(20),
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "bar",
                "src/lib.rs::bar",
                "src/lib.rs",
                Some(25),
                Some(35),
                Some("rust"),
                None,
            )
            .unwrap();

        let nodes = store.nodes_for_file("src/lib.rs").unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "foo");
    }

    #[test]
    fn delete_file_removes_nodes_and_edges() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "a.rs::foo",
                "a.rs",
                Some(1),
                Some(5),
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 3)
            .unwrap();
        store.set_file_hash("a.rs", "abc123").unwrap();

        store.delete_file("a.rs").unwrap();

        assert!(store.nodes_for_file("a.rs").unwrap().is_empty());
        assert!(store.get_file_hash("a.rs").unwrap().is_none());
    }

    #[test]
    fn file_hash_tracking() {
        let store = GraphStore::open_memory().unwrap();
        assert!(store.get_file_hash("foo.rs").unwrap().is_none());

        store.set_file_hash("foo.rs", "hash1").unwrap();
        assert_eq!(
            store.get_file_hash("foo.rs").unwrap().as_deref(),
            Some("hash1")
        );

        store.set_file_hash("foo.rs", "hash2").unwrap();
        assert_eq!(
            store.get_file_hash("foo.rs").unwrap().as_deref(),
            Some("hash2")
        );
    }

    #[test]
    fn impact_radius_linear_chain() {
        // a.rs::foo → b.rs::bar → c.rs::baz (Calls edges)
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "a.rs::foo",
                "a.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "bar",
                "b.rs::bar",
                "b.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "baz",
                "c.rs::baz",
                "c.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();

        // Edge direction: source calls target → source depends on target
        // If c.rs changes, who is affected? b.rs (calls c.rs), then a.rs (calls b.rs)
        // Edges: a calls b, b calls c → a.rs::foo -> b.rs::bar, b.rs::bar -> c.rs::baz
        store
            .insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 2)
            .unwrap();
        store
            .insert_edge(EdgeKind::Calls, "b.rs::bar", "c.rs::baz", "b.rs", 3)
            .unwrap();

        // Change c.rs → should affect b.rs (direct caller) and a.rs (transitive)
        let result = store.impact_radius(&["c.rs"], 5).unwrap();
        assert!(result.affected_files.contains(&"c.rs".to_string()));
        assert!(result.affected_files.contains(&"b.rs".to_string()));
        assert!(result.affected_files.contains(&"a.rs".to_string()));
    }

    #[test]
    fn impact_radius_respects_depth() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "a.rs::foo",
                "a.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "bar",
                "b.rs::bar",
                "b.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "baz",
                "c.rs::baz",
                "c.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();

        store
            .insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 2)
            .unwrap();
        store
            .insert_edge(EdgeKind::Calls, "b.rs::bar", "c.rs::baz", "b.rs", 3)
            .unwrap();

        // Depth 1: c.rs changes → only b.rs affected (direct), not a.rs
        let result = store.impact_radius(&["c.rs"], 1).unwrap();
        assert!(result.affected_files.contains(&"b.rs".to_string()));
        assert!(!result.affected_files.contains(&"a.rs".to_string()));
    }

    #[test]
    fn impact_radius_finds_tests() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "src/lib.rs::foo",
                "src/lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Test,
                "test_foo",
                "tests/test_lib.rs::test_foo",
                "tests/test_lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();

        // Test imports/calls source
        store
            .insert_edge(
                EdgeKind::Calls,
                "tests/test_lib.rs::test_foo",
                "src/lib.rs::foo",
                "tests/test_lib.rs",
                3,
            )
            .unwrap();

        let result = store.impact_radius(&["src/lib.rs"], 3).unwrap();
        assert!(
            result
                .affected_tests
                .contains(&"tests/test_lib.rs".to_string())
        );
    }

    #[test]
    fn impact_radius_mode_filters_edge_kinds() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "src/lib.rs::foo",
                "src/lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "run",
                "src/app.rs::run",
                "src/app.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Test,
                "test_foo",
                "tests/test_lib.rs::test_foo",
                "tests/test_lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .insert_edge(
                EdgeKind::Calls,
                "src/app.rs::run",
                "src/lib.rs::foo",
                "src/app.rs",
                2,
            )
            .unwrap();
        store
            .insert_edge(
                EdgeKind::TestOf,
                "tests/test_lib.rs::test_foo",
                "src/lib.rs::foo",
                "tests/test_lib.rs",
                3,
            )
            .unwrap();

        let callers = store
            .impact_radius_with_mode(&["src/lib.rs"], 2, BlastRadiusMode::Callers)
            .unwrap();
        assert!(callers.affected_files.contains(&"src/app.rs".to_string()));
        assert!(
            !callers
                .affected_files
                .contains(&"tests/test_lib.rs".to_string())
        );

        let tests = store
            .impact_radius_with_mode(&["src/lib.rs"], 2, BlastRadiusMode::Tests)
            .unwrap();
        assert!(
            tests
                .affected_files
                .contains(&"tests/test_lib.rs".to_string())
        );
        assert!(!tests.affected_files.contains(&"src/app.rs".to_string()));
    }

    #[test]
    fn edge_weights_overrides_clamp_and_fallback() {
        let mut weights = EdgeWeights::default_for(BlastRadiusMode::Impact);
        // Plan defaults for Impact mode.
        assert_eq!(weights.weight(EdgeKind::Calls), 1.0);
        assert_eq!(weights.weight(EdgeKind::TestOf), 0.3);

        // User overrides via the JSON object shape from
        // `repoMap.edges.weights.impact`. Out-of-range values clamp;
        // unknown keys are ignored; un-listed kinds keep their preset.
        let json = serde_json::json!({
            "calls": 0.4,
            "testOf": 1.5,
            "imports": -0.1,
            "unknownKind": 0.99,
        });
        weights.apply_overrides(json.as_object().unwrap());
        assert_eq!(weights.weight(EdgeKind::Calls), 0.4);
        assert_eq!(
            weights.weight(EdgeKind::TestOf),
            1.0,
            "1.5 must clamp to 1.0"
        );
        assert_eq!(
            weights.weight(EdgeKind::Imports),
            0.0,
            "-0.1 must clamp to 0.0"
        );
        assert_eq!(
            weights.weight(EdgeKind::Implements),
            0.9,
            "Implements untouched: must keep preset"
        );
    }

    #[test]
    fn typed_edge_migration_defaults_null_kind_to_imports() {
        // Simulate a pre-C4 database: insert a row with NULL kind and
        // re-run the migration. After migration the kind is 'Imports'
        // and the row participates in mode=all blast-radius queries.
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "process",
                "src/lib.rs::process",
                "src/lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "caller",
                "src/app.rs::caller",
                "src/app.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        // The current schema has `kind TEXT NOT NULL`, but a pre-C4
        // database may have either NULL kinds (older schema) or empty
        // strings (very early B-tier rows). The migration treats both
        // the same, so we exercise the empty-string path which the
        // current schema accepts.
        store
            .conn
            .execute(
                "INSERT INTO edges (kind, source_qn, target_qn, file_path, line)
                 VALUES ('', 'src/app.rs::caller', 'src/lib.rs::process', 'src/app.rs', 3)",
                [],
            )
            .unwrap();

        super::migrate_typed_edges(&store.conn).unwrap();

        let kind: String = store
            .conn
            .query_row(
                "SELECT kind FROM edges WHERE source_qn = 'src/app.rs::caller'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "Imports", "NULL kind must default to 'Imports'");

        // The migrated edge participates in mode=all traversal.
        let result = store.impact_radius(&["src/lib.rs"], 2).unwrap();
        assert!(
            result.affected_files.contains(&"src/app.rs".to_string()),
            "migrated edge must be reachable: {:?}",
            result.affected_files
        );
    }

    #[test]
    fn impact_radius_empty_input() {
        let store = GraphStore::open_memory().unwrap();
        let result = store.impact_radius(&[], 5).unwrap();
        assert!(result.affected_files.is_empty());
    }

    #[test]
    fn test_gaps_detected() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "src/lib.rs::foo",
                "src/lib.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        // No TestOf edge for src/lib.rs
        let result = store.impact_radius(&["src/lib.rs"], 3).unwrap();
        assert!(result.test_gaps.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn stats_counts() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "foo",
                "a.rs::foo",
                "a.rs",
                None,
                None,
                None,
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "bar",
                "b.rs::bar",
                "b.rs",
                None,
                None,
                None,
                None,
            )
            .unwrap();
        store
            .insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 1)
            .unwrap();

        let (nodes, edges) = store.stats().unwrap();
        assert_eq!(nodes, 2);
        assert_eq!(edges, 1);
    }

    #[test]
    fn resolve_symbol_finds_matches() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "process",
                "a.rs::process",
                "a.rs",
                None,
                None,
                None,
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "process",
                "b.rs::process",
                "b.rs",
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let matches = store.resolve_symbol("process").unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn format_impact_prompt_ranked_renders_specificity_badge() {
        let result = ImpactSummary {
            changed_files: vec!["src/auth.rs".into()],
            affected_files: vec!["src/auth.rs".into(), "src/api.rs".into()],
            affected_tests: vec!["tests/auth_test.rs".into()],
            test_gaps: vec![],
            truncated: false,
        };
        let mut ranks = std::collections::HashMap::new();
        ranks.insert("src/auth.rs".into(), (0.9, 0.92));
        ranks.insert("src/api.rs".into(), (0.4, 0.71));
        ranks.insert("tests/auth_test.rs".into(), (0.2, 0.05));

        let text = GraphStore::format_impact_for_prompt_ranked(&result, &ranks);
        assert!(text.contains("src/auth.rs (s0.92)"), "{text}");
        assert!(text.contains("src/api.rs (s0.71)"), "{text}");
        assert!(text.contains("tests/auth_test.rs (s0.05)"), "{text}");
    }

    #[test]
    fn format_impact_prompt() {
        let result = ImpactSummary {
            changed_files: vec!["src/auth.rs".into()],
            affected_files: vec![
                "src/auth.rs".into(),
                "src/api.rs".into(),
                "tests/auth_test.rs".into(),
            ],
            affected_tests: vec!["tests/auth_test.rs".into()],
            test_gaps: vec![],
            truncated: false,
        };
        let text = GraphStore::format_impact_for_prompt(&result);
        assert!(text.contains("Imp:"));
        assert!(text.contains("chg: src/auth.rs"));
        assert!(text.contains("dep: src/api.rs"));
        assert!(text.contains("tst: tests/auth_test.rs"));
    }

    // PUSH→PULL Phase 2: the thin impact summary names blast_radius, keeps the
    // missing-tests slice verbatim, and stays inside the ~150-token cap.
    #[test]
    fn format_impact_summary_is_thin_and_names_blast_radius() {
        let result = ImpactSummary {
            changed_files: vec!["src/auth.rs".into(), "src/api.rs".into()],
            affected_files: vec![
                "src/auth.rs".into(),
                "src/api.rs".into(),
                "src/router.rs".into(),
                "src/server.rs".into(),
            ],
            affected_tests: vec![],
            test_gaps: vec!["src/auth.rs".into()],
            truncated: false,
        };
        let text = GraphStore::format_impact_summary(&result);
        // Counts: 2 changed, 2 dependents (router/server, not the changed set).
        assert!(text.contains("2 changed"), "{text}");
        assert!(text.contains("2 dependent"), "{text}");
        assert!(text.contains("1 missing tests"), "{text}");
        // Names the pull tool and keeps the missing-tests filename verbatim.
        assert!(text.contains("blast_radius(path)"), "{text}");
        assert!(text.contains("no tests: src/auth.rs"), "{text}");
        // ≤150 tokens (project convention ~4 chars/token → ≤600 chars).
        assert!(
            text.len() / 4 <= 150,
            "summary too large: {} chars",
            text.len()
        );
    }

    #[test]
    fn format_impact_summary_empty_when_nothing_to_report() {
        // Empty buffer / no impact → empty string → caller injects nothing.
        let empty = ImpactSummary::default();
        assert!(GraphStore::format_impact_summary(&empty).is_empty());
    }

    #[test]
    fn format_impact_summary_caps_long_test_gap_list() {
        // A large missing-tests list is truncated (never the count) to stay
        // under the char budget, with a "+N more" marker.
        let gaps: Vec<String> = (0..200)
            .map(|i| format!("crates/some/long/path/to/module_number_{i:04}.rs"))
            .collect();
        let result = ImpactSummary {
            changed_files: gaps.clone(),
            affected_files: gaps.clone(),
            affected_tests: vec![],
            test_gaps: gaps,
            truncated: false,
        };
        let text = GraphStore::format_impact_summary(&result);
        assert!(
            text.len() / 4 <= 150,
            "summary too large: {} chars",
            text.len()
        );
        assert!(text.contains("more)"), "{text}");
        assert!(text.contains("200 missing tests"), "{text}");
    }

    // ── query plan, unique edges, cancellation ───────────────────────

    fn edge_count(store: &GraphStore) -> i64 {
        store
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap()
    }

    /// The recursive step must walk affected file → nodes → edges through
    /// the file and target indexes, never start from a scan of `edges`.
    #[test]
    fn impact_cte_walks_from_files_through_the_indexes() {
        let store = GraphStore::open_memory().unwrap();
        store
            .conn
            .execute_batch("CREATE TEMP TABLE _impact_seed_files (fp TEXT PRIMARY KEY)")
            .unwrap();
        let sql = format!(
            "EXPLAIN QUERY PLAN {}",
            impact_cte_sql(2, BlastRadiusMode::All.edge_kind_sql_list())
        );
        let mut stmt = store.conn.prepare(&sql).unwrap();
        let plan: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let text = plan.join("\n");
        assert!(text.contains("idx_nodes_file"), "{text}");
        assert!(text.contains("idx_edges_tgt"), "{text}");
        assert!(!text.contains("idx_edges_kind"), "edges must not drive the step:\n{text}");
    }

    #[test]
    fn duplicate_edges_are_ignored() {
        let store = GraphStore::open_memory().unwrap();
        for _ in 0..3 {
            store
                .insert_edge(EdgeKind::Calls, "a.rs::new", "b.rs::new", "a.rs", 7)
                .unwrap();
        }
        store
            .insert_edge(EdgeKind::Calls, "a.rs::new", "c.rs::new", "a.rs", 9)
            .unwrap();
        assert_eq!(edge_count(&store), 2);
    }

    /// An existing database full of duplicates is de-duplicated once on
    /// open, keeping the first row of each group.
    #[test]
    fn opening_a_duplicated_database_removes_the_duplicates_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("g.db");
        {
            let store = GraphStore::open(&db).unwrap();
            store
                .conn
                .execute_batch("DROP INDEX idx_edges_unique")
                .unwrap();
            for line in [3, 5, 8] {
                store
                    .conn
                    .execute(
                        "INSERT INTO edges (kind, source_qn, target_qn, file_path, line)
                         VALUES ('Calls', 'a.rs::new', 'b.rs::new', 'a.rs', ?1)",
                        params![line],
                    )
                    .unwrap();
            }
            store
                .conn
                .execute(
                    "INSERT INTO edges (kind, source_qn, target_qn, file_path, line)
                     VALUES ('Imports', 'a.rs::new', 'b.rs::new', 'a.rs', 1)",
                    [],
                )
                .unwrap();
            assert_eq!(edge_count(&store), 4);
        }
        let store = GraphStore::open(&db).unwrap();
        assert_eq!(edge_count(&store), 2, "one Calls + one Imports edge remain");
        let line: i64 = store
            .conn
            .query_row(
                "SELECT line FROM edges WHERE kind = 'Calls'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(line, 3, "first row kept");
        store
            .insert_edge(EdgeKind::Calls, "a.rs::new", "b.rs::new", "a.rs", 99)
            .unwrap();
        assert_eq!(edge_count(&store), 2, "the unique index now holds");
    }

    #[test]
    fn node_owners_round_trip_and_old_databases_gain_the_column() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("old.db");
        {
            // A nodes table from before the `owner` column existed.
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE nodes (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
                   name TEXT NOT NULL, qualified_name TEXT NOT NULL UNIQUE, file_path TEXT NOT NULL,
                   line_start INTEGER, line_end INTEGER, language TEXT, file_hash TEXT,
                   updated_at REAL NOT NULL DEFAULT (julianday('now')));
                 INSERT INTO nodes (kind, name, qualified_name, file_path)
                   VALUES ('Function', 'new', 'w.rs::new', 'w.rs');",
            )
            .unwrap();
        }
        let store = GraphStore::open(&db).unwrap();
        let defs = store.all_definitions().unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].owner, None, "legacy rows stay NULL");
        store.set_node_owner("w.rs::new", Some("impl:Widget")).unwrap();
        drop(store);
        let store = GraphStore::open(&db).unwrap();
        let defs = store.all_definitions().unwrap();
        assert_eq!(defs[0].owner.as_deref(), Some("impl:Widget"));
        assert_eq!(defs[0].kind, "Function");
    }

    #[test]
    fn type_facts_round_trip_replace_delete_and_old_databases_gain_the_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("old.db");
        {
            // A database from before the type-fact tables existed.
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch("CREATE TABLE graph_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
                .unwrap();
        }
        let store = GraphStore::open(&db).unwrap();
        let facts = RustFileFacts {
            fields: vec![("App".into(), "git_panel".into(), "GitPanel".into())],
            returns: vec![("impl:App".into(), "panel".into(), "&GitPanel".into(), false)],
            impls: vec![("App".into(), String::new()), ("App".into(), "Observer".into())],
            aliases: vec![("Res".into(), "T".into(), "Result<T,Error>".into())],
            imports: vec![("GitPanel".into(), "crate::git::GitPanel".into(), false)],
            methods: vec![("impl:App".into(), "panel".into())],
        };
        store.replace_type_facts("app.rs", &facts).unwrap();
        store.replace_type_facts("app.rs", &facts).unwrap();
        assert_eq!(store.all_type_facts().unwrap()["app.rs"], facts, "replace, not append");
        store.delete_file_graph("app.rs").unwrap();
        assert!(store.all_type_facts().unwrap().is_empty());
    }

    #[test]
    fn a_cancelled_query_never_starts() {
        let store = GraphStore::open_memory().unwrap();
        let cancel = GraphCancel::new();
        cancel.cancel();
        assert!(cancel.attach(&store).is_err());
        assert!(cancel.check().is_err());
    }

    /// `cancel` interrupts a statement that is already running on another
    /// thread (here one that would never end on its own).
    #[test]
    fn cancel_interrupts_a_running_statement() {
        let store = GraphStore::open_memory().unwrap();
        let cancel = GraphCancel::new();
        cancel.attach(&store).unwrap();
        let worker = std::thread::spawn(move || {
            store.conn.query_row(
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
                 SELECT COUNT(*) FROM c",
                [],
                |r| r.get::<_, i64>(0),
            )
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        let started = std::time::Instant::now();
        cancel.cancel();
        let result = worker.join().unwrap();
        assert!(result.is_err(), "the endless statement must be interrupted");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "interrupt took {:?}",
            started.elapsed()
        );
    }
}
