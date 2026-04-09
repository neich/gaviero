//! SQLite-backed code knowledge graph.
//!
//! Stores code structure as nodes (File, Function, Struct, Trait, Enum, Test)
//! and edges (Imports, Calls, Implements, TestedBy, Contains).
//!
//! Supports incremental updates via file-hash diffing and blast-radius queries
//! via recursive CTE.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// File imports/uses another module.
    Imports,
    /// Function/method calls another function.
    Calls,
    /// Type implements a trait/interface or extends a class.
    Implements,
    /// Test file/function tests source code.
    TestedBy,
    /// File contains a symbol (parent-child).
    Contains,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Imports => "Imports",
            Self::Calls => "Calls",
            Self::Implements => "Implements",
            Self::TestedBy => "TestedBy",
            Self::Contains => "Contains",
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

/// Result of a blast-radius query.
#[derive(Debug, Clone, Default)]
pub struct ImpactResult {
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
        Ok(Self { conn })
    }

    /// Open an in-memory graph database (for tests).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
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
             FROM nodes WHERE file_path = ?1"
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
        self.conn.execute("DELETE FROM edges WHERE file_path = ?1", params![file_path])?;
        self.conn.execute("DELETE FROM nodes WHERE file_path = ?1", params![file_path])?;
        self.conn.execute("DELETE FROM file_hashes WHERE file_path = ?1", params![file_path])?;
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
        let mut stmt = self.conn.prepare(
            "SELECT hash FROM file_hashes WHERE file_path = ?1"
        )?;
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
        let mut stmt = self.conn.prepare("SELECT file_path, hash FROM file_hashes")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ── Stats ────────────────────────────────────────────────────

    /// Count total nodes and edges.
    pub fn stats(&self) -> Result<(usize, usize)> {
        let nodes: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let edges: i64 = self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
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
    pub fn impact_radius(
        &self,
        changed_files: &[&str],
        max_depth: usize,
    ) -> Result<ImpactResult> {
        if changed_files.is_empty() {
            return Ok(ImpactResult::default());
        }

        // 1. Create temp table for seed files
        self.conn.execute(
            "CREATE TEMP TABLE IF NOT EXISTS _impact_seed_files (fp TEXT PRIMARY KEY)", [],
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
                      AND (e.file_path LIKE '%test%' OR e.kind = 'TestedBy')
                )",
                params![cf],
                |row| row.get(0),
            )?;
            if !has_test {
                test_gaps.push(cf.to_string());
            }
        }

        Ok(ImpactResult {
            changed_files: changed_files.iter().map(|s| s.to_string()).collect(),
            affected_files,
            affected_tests,
            test_gaps,
            truncated: false,
        })
    }

    /// Format impact result as a prompt-friendly string.
    pub fn format_impact_for_prompt(result: &ImpactResult) -> String {
        let mut lines = Vec::new();
        lines.push("[Impact analysis]:".to_string());

        if !result.changed_files.is_empty() {
            lines.push(format!("Changed: {}", result.changed_files.join(", ")));
        }

        let non_changed: Vec<&str> = result.affected_files.iter()
            .filter(|f| !result.changed_files.contains(f))
            .map(|s| s.as_str())
            .collect();
        if !non_changed.is_empty() {
            lines.push(format!("Affected dependents: {}", non_changed.join(", ")));
        }

        if !result.affected_tests.is_empty() {
            lines.push(format!("Affected tests: {}", result.affected_tests.join(", ")));
        }

        if !result.test_gaps.is_empty() {
            lines.push(format!("Test gaps (no coverage): {}", result.test_gaps.join(", ")));
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

    /// Resolve a symbol name to its qualified name(s) across the graph.
    /// Used for cross-file reference resolution.
    pub fn resolve_symbol(&self, name: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT qualified_name, file_path FROM nodes WHERE name = ?1"
        )?;
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
        store.upsert_node(
            NodeKind::Function, "foo", "src/lib.rs::foo", "src/lib.rs",
            Some(10), Some(20), Some("rust"), None,
        ).unwrap();
        store.upsert_node(
            NodeKind::Function, "bar", "src/lib.rs::bar", "src/lib.rs",
            Some(25), Some(35), Some("rust"), None,
        ).unwrap();

        let nodes = store.nodes_for_file("src/lib.rs").unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "foo");
    }

    #[test]
    fn delete_file_removes_nodes_and_edges() {
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(
            NodeKind::Function, "foo", "a.rs::foo", "a.rs",
            Some(1), Some(5), Some("rust"), None,
        ).unwrap();
        store.insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 3).unwrap();
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
        assert_eq!(store.get_file_hash("foo.rs").unwrap().as_deref(), Some("hash1"));

        store.set_file_hash("foo.rs", "hash2").unwrap();
        assert_eq!(store.get_file_hash("foo.rs").unwrap().as_deref(), Some("hash2"));
    }

    #[test]
    fn impact_radius_linear_chain() {
        // a.rs::foo → b.rs::bar → c.rs::baz (Calls edges)
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(NodeKind::Function, "foo", "a.rs::foo", "a.rs", Some(1), None, Some("rust"), None).unwrap();
        store.upsert_node(NodeKind::Function, "bar", "b.rs::bar", "b.rs", Some(1), None, Some("rust"), None).unwrap();
        store.upsert_node(NodeKind::Function, "baz", "c.rs::baz", "c.rs", Some(1), None, Some("rust"), None).unwrap();

        // Edge direction: source calls target → source depends on target
        // If c.rs changes, who is affected? b.rs (calls c.rs), then a.rs (calls b.rs)
        // Edges: a calls b, b calls c → a.rs::foo -> b.rs::bar, b.rs::bar -> c.rs::baz
        store.insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 2).unwrap();
        store.insert_edge(EdgeKind::Calls, "b.rs::bar", "c.rs::baz", "b.rs", 3).unwrap();

        // Change c.rs → should affect b.rs (direct caller) and a.rs (transitive)
        let result = store.impact_radius(&["c.rs"], 5).unwrap();
        assert!(result.affected_files.contains(&"c.rs".to_string()));
        assert!(result.affected_files.contains(&"b.rs".to_string()));
        assert!(result.affected_files.contains(&"a.rs".to_string()));
    }

    #[test]
    fn impact_radius_respects_depth() {
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(NodeKind::Function, "foo", "a.rs::foo", "a.rs", Some(1), None, Some("rust"), None).unwrap();
        store.upsert_node(NodeKind::Function, "bar", "b.rs::bar", "b.rs", Some(1), None, Some("rust"), None).unwrap();
        store.upsert_node(NodeKind::Function, "baz", "c.rs::baz", "c.rs", Some(1), None, Some("rust"), None).unwrap();

        store.insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 2).unwrap();
        store.insert_edge(EdgeKind::Calls, "b.rs::bar", "c.rs::baz", "b.rs", 3).unwrap();

        // Depth 1: c.rs changes → only b.rs affected (direct), not a.rs
        let result = store.impact_radius(&["c.rs"], 1).unwrap();
        assert!(result.affected_files.contains(&"b.rs".to_string()));
        assert!(!result.affected_files.contains(&"a.rs".to_string()));
    }

    #[test]
    fn impact_radius_finds_tests() {
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(NodeKind::Function, "foo", "src/lib.rs::foo", "src/lib.rs", Some(1), None, Some("rust"), None).unwrap();
        store.upsert_node(NodeKind::Test, "test_foo", "tests/test_lib.rs::test_foo", "tests/test_lib.rs", Some(1), None, Some("rust"), None).unwrap();

        // Test imports/calls source
        store.insert_edge(EdgeKind::Calls, "tests/test_lib.rs::test_foo", "src/lib.rs::foo", "tests/test_lib.rs", 3).unwrap();

        let result = store.impact_radius(&["src/lib.rs"], 3).unwrap();
        assert!(result.affected_tests.contains(&"tests/test_lib.rs".to_string()));
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
        store.upsert_node(NodeKind::Function, "foo", "src/lib.rs::foo", "src/lib.rs", Some(1), None, Some("rust"), None).unwrap();
        // No TestedBy edge for src/lib.rs
        let result = store.impact_radius(&["src/lib.rs"], 3).unwrap();
        assert!(result.test_gaps.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn stats_counts() {
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(NodeKind::Function, "foo", "a.rs::foo", "a.rs", None, None, None, None).unwrap();
        store.upsert_node(NodeKind::Function, "bar", "b.rs::bar", "b.rs", None, None, None, None).unwrap();
        store.insert_edge(EdgeKind::Calls, "a.rs::foo", "b.rs::bar", "a.rs", 1).unwrap();

        let (nodes, edges) = store.stats().unwrap();
        assert_eq!(nodes, 2);
        assert_eq!(edges, 1);
    }

    #[test]
    fn resolve_symbol_finds_matches() {
        let store = GraphStore::open_memory().unwrap();
        store.upsert_node(NodeKind::Function, "process", "a.rs::process", "a.rs", None, None, None, None).unwrap();
        store.upsert_node(NodeKind::Function, "process", "b.rs::process", "b.rs", None, None, None, None).unwrap();

        let matches = store.resolve_symbol("process").unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn format_impact_prompt() {
        let result = ImpactResult {
            changed_files: vec!["src/auth.rs".into()],
            affected_files: vec!["src/auth.rs".into(), "src/api.rs".into(), "tests/auth_test.rs".into()],
            affected_tests: vec!["tests/auth_test.rs".into()],
            test_gaps: vec![],
            truncated: false,
        };
        let text = GraphStore::format_impact_for_prompt(&result);
        assert!(text.contains("Changed: src/auth.rs"));
        assert!(text.contains("Affected dependents: src/api.rs"));
        assert!(text.contains("Affected tests: tests/auth_test.rs"));
    }
}
