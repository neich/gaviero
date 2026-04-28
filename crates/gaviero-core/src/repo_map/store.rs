//! SQLite-backed code knowledge graph.
//!
//! Stores code structure as nodes (File, Function, Struct, Trait, Enum, Test)
//! and typed edges (Calls, Imports, Implements, Defines, TestOf, doc references,
//! and future contract declarations).
//!
//! Supports incremental updates via file-hash diffing and blast-radius queries
//! via recursive CTE.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

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
";

/// C4: one-shot migration applied at every `open*` call.
///
/// Pre-C4 databases stored edges with either NULL `kind` (very old
/// schema) or the legacy aliases `'Contains'`/`'TestedBy'`. The
/// in-memory `EdgeKind::from_str` already handles those aliases
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
    conn.execute(
        "UPDATE edges SET kind = 'Imports' WHERE kind IS NULL OR kind = ''",
        [],
    )
    .context("typed-edge migration: defaulting null kinds")?;
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

    pub fn from_str(s: &str) -> Option<Self> {
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

    pub fn from_str(s: &str) -> Option<Self> {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlastRadiusMode {
    Impact,
    Callers,
    Tests,
    Implementations,
    All,
}

impl Default for BlastRadiusMode {
    fn default() -> Self {
        Self::All
    }
}

impl BlastRadiusMode {
    pub fn from_str(s: &str) -> Self {
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
///                       Imports=0.5, TestOf=0.3, others=0.2
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
        Ok(Self { conn })
    }

    /// Open an in-memory graph database (for tests).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate_typed_edges(&conn)?;
        Ok(Self { conn })
    }

    // ── Node operations ──────────────────────────────────────────

    /// Insert or update a node. Returns the node id.
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

    /// Delete all nodes and edges for a given file path.
    pub fn delete_file(&self, file_path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM edges WHERE file_path = ?1", params![file_path])?;
        self.conn
            .execute("DELETE FROM nodes WHERE file_path = ?1", params![file_path])?;
        self.conn.execute(
            "DELETE FROM file_hashes WHERE file_path = ?1",
            params![file_path],
        )?;
        Ok(())
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
        let cte_sql = format!(
            "WITH RECURSIVE affected(file, depth) AS (
                SELECT fp, 0 FROM _impact_seed_files
                UNION
                SELECT DISTINCT e.file_path, a.depth + 1
                FROM affected a
                JOIN nodes n ON n.file_path = a.file
                JOIN edges e ON e.target_qn = n.qualified_name
                WHERE a.depth < {max_depth}
                  AND e.kind IN ({edge_kinds})
                  AND e.file_path != a.file
            )
            SELECT DISTINCT file FROM affected"
        );
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
            if let Some(kind) = EdgeKind::from_str(&kind_s) {
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
        let mut stmt = self.conn.prepare(
            "SELECT n.name, COUNT(DISTINCT n.file_path) FROM nodes n GROUP BY n.name",
        )?;
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
}
