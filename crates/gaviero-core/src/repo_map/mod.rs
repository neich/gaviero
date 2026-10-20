//! Repo-map: context budget planning for multi-agent swarm runs.
//!
//! Builds a reference graph of source files: nodes from tree-sitter symbol
//! extraction, file references (edges) from the code graph
//! ([`store::GraphStore::file_edges`]). Personalized PageRank then ranks
//! files by relevance to an agent's owned paths. The resulting `ContextPlan`
//! tells the runner which files to include as full content vs.
//! signatures-only vs. just a path list.
//!
//! ## Usage
//! ```ignore
//! let repo_map = RepoMap::build(&workspace_root, &[])?;
//! let plan = repo_map.rank_for_agent(&["src/auth/"], 32_000);
//! // Inject plan.repo_outline into the agent's system prompt
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use petgraph::graph::{DiGraph, NodeIndex};

pub mod builder;
pub mod edges;
pub mod extraction;
pub mod graph_builder;
pub mod page_rank;
pub mod resolve;
pub mod rust_walk;
pub mod service;
pub mod store;
pub mod symbol_enrichment;
pub mod symbol_search;
pub mod topology;
pub mod types;

pub use service::{GraphFolder, GraphService, GraphServiceStats, GraphServices};
pub use topology::{TopologyConfig, build_folder_topology};

/// Normalize a workspace-relative path to `/` separators (Tier W1 /
/// PR-6). Rel paths are store keys and LLM-facing text — one spelling
/// on every host; absolute paths stay native for I/O.
pub(crate) fn normalize_rel_path(rel: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(rel.to_string_lossy().replace('\\', "/"))
    } else {
        rel.to_path_buf()
    }
}

// ── Graph types ──────────────────────────────────────────────

/// A source file node in the repo reference graph.
#[derive(Debug, Clone)]
pub struct FileNode {
    /// Path relative to workspace root.
    pub path: PathBuf,
    /// Rough token estimate (bytes / 4).
    pub token_estimate: usize,
    /// Top-level definitions extracted by tree-sitter.
    pub symbols: Vec<Symbol>,
}

/// A top-level definition (function, struct, trait, etc.) in a file.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    /// tree-sitter node kind (e.g. `"function_item"`).
    pub kind: String,
    /// 0-based line number.
    pub line: usize,
    /// Enclosing container, for methods: `impl:Type`, `impl:Type:Trait`
    /// (an `impl Trait for Type` block) or `trait:Name`. `None` at top
    /// level. Used by edge resolution to tell `Widget::new` from
    /// `Gadget::new`; not part of the symbol's `qualified_name`.
    pub owner: Option<String>,
}

/// Reverse-edge damping: a seed's rank flows to the files it references at
/// full edge weight and back to the files that reference it at this
/// fraction.
pub const REVERSE_EDGE_FACTOR: f64 = 0.5;

/// A typed edge between two file nodes.
#[derive(Debug, Clone)]
pub struct ReferenceEdge {
    pub kind: store::EdgeKind,
    /// True for the mirror of a code-graph edge (referenced file →
    /// referencing file), weighted by [`REVERSE_EDGE_FACTOR`].
    pub reverse: bool,
}

impl ReferenceEdge {
    pub fn new(kind: store::EdgeKind) -> Self {
        Self {
            kind,
            reverse: false,
        }
    }

    /// The mirror edge: from the referenced file back to the referencing one.
    pub fn reversed(kind: store::EdgeKind) -> Self {
        Self {
            kind,
            reverse: true,
        }
    }

    /// PageRank transition weight under `mode`: the mode's preset for this
    /// kind, times [`REVERSE_EDGE_FACTOR`] for a reverse edge.
    pub fn weight(&self, mode: store::BlastRadiusMode) -> f64 {
        let w = mode.edge_weight(self.kind);
        if self.reverse {
            w * REVERSE_EDGE_FACTOR
        } else {
            w
        }
    }
}

// ── Context plan ─────────────────────────────────────────────

/// The result of ranking for an agent: which files to include and how.
#[derive(Debug, Default)]
pub struct ContextPlan {
    /// Files whose full content should be injected.
    pub full_content: Vec<PathBuf>,
    /// Files where only signatures (symbol names) should be shown.
    pub signatures: Vec<(PathBuf, Vec<Symbol>)>,
    /// Brief text outline suitable for insertion into a prompt.
    pub repo_outline: String,
    /// Estimated total token cost of this plan.
    pub token_estimate: usize,
}

/// Compute IDF-like node specificity for the file-level repo map.
///
/// The persisted code graph stores symbol-level nodes, but this in-memory
/// `RepoMap` ranks files. We therefore collapse symbol specificity onto each
/// file by averaging the normalized specificity of its extracted symbols.
pub fn compute_specificity_map(
    graph: &DiGraph<FileNode, ReferenceEdge>,
    stop_symbol_threshold: f64,
) -> HashMap<NodeIndex, f64> {
    let n = graph.node_count();
    if n == 0 {
        return HashMap::new();
    }

    let mut df: HashMap<String, usize> = HashMap::new();
    for idx in graph.node_indices() {
        let mut seen: HashSet<&str> = HashSet::new();
        for sym in &graph[idx].symbols {
            if seen.insert(sym.name.as_str()) {
                *df.entry(sym.name.clone()).or_default() += 1;
            }
        }
    }

    let mut raw_by_symbol: HashMap<&str, f64> = HashMap::new();
    let mut max_raw = 0.0f64;
    for (symbol, count) in &df {
        let ratio = *count as f64 / n as f64;
        let raw = if ratio > stop_symbol_threshold {
            0.0
        } else {
            (n as f64 / *count as f64).ln().max(0.0)
        };
        max_raw = max_raw.max(raw);
        raw_by_symbol.insert(symbol.as_str(), raw);
    }

    graph
        .node_indices()
        .map(|idx| {
            let symbols = &graph[idx].symbols;
            let specificity = if symbols.is_empty() {
                1.0
            } else if max_raw <= f64::EPSILON {
                0.0
            } else {
                let sum: f64 = symbols
                    .iter()
                    .map(|sym| raw_by_symbol.get(sym.name.as_str()).copied().unwrap_or(0.0))
                    .sum();
                (sum / symbols.len() as f64 / max_raw).clamp(0.0, 1.0)
            };
            (idx, specificity)
        })
        .collect()
}

// ── M3 structured candidates (V9 §4) ─────────────────────────

/// Confidence band for a graph selection.
///
/// V9 §4 placeholder enum. M3 computes it via percentile bucketing on
/// PageRank score across the ranked set: top 10 % → `High`, next 30 % →
/// `Medium`, rest → `Low`. M7 (selective pre-attach) refines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphConfidence {
    Low,
    Medium,
    High,
}

/// Per-file decision the planner records for a [`GraphCandidate`].
///
/// V9 §4 `GraphDecision`. Lives in `repo_map` (not `context_planner/ledger`)
/// because the ranker emits this and the ledger consumes it — defining it
/// here breaks the would-be cycle. The ledger re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphDecision {
    PathOnly,
    SignatureOnly,
    OutlineOnly,
    FullAttach,
}

/// V9 §4 `GraphCandidate`: structured per-file ranking record.
///
/// Returned by [`RepoMap::rank_for_agent_structured`] for the planner to
/// consume and record in the ledger. Lives next to `RepoMap` (not under
/// `context_planner/`) for the same module-cycle reason as
/// `MemoryCandidate`. Re-exported from `crate::context_planner::types`.
#[derive(Debug, Clone)]
pub struct GraphCandidate {
    pub path: PathBuf,
    pub rank_score: f64,
    pub specificity: f64,
    pub confidence: GraphConfidence,
    pub decision: GraphDecision,
    pub token_estimate: usize,
    pub symbols: Vec<Symbol>,
    /// Pre-rendered outline line (matches today's `repo_outline` format
    /// per file). M3 keeps this so the adapter can stay byte-identical;
    /// later milestones may render entirely from structured fields.
    pub rendered_line: String,
    /// File content digest. M3 placeholder (`None`); M4 fills with a real
    /// hash so the ledger can detect modifications.
    pub content_digest: Option<String>,
}

/// Hard cap on `<repo_outline>` entries, independent of the token budget.
/// An anti-explosion guard: with the line-cost budgeting below, outline
/// entries are cheap, so a very large graph could otherwise dump thousands
/// of path-only lines. The token budget is the primary bound; this only
/// binds on pathologically large repos.
const OUTLINE_MAX_FILES: usize = 300;

/// Token cost of a single `<repo_outline>` line — what the outline actually
/// injects for a file. The outline budget is charged against *this*, never
/// the file's full content: a `FullAttach` file only emits its `OWN <path>`
/// line on the first turn (the full content is attached later by the M7
/// outline→full upgrade, keyed off the content digest). Charging
/// full-content tokens up front over-reserves the budget and starves
/// signature breadth, and inflates `graph_token_estimate` (the bottom-bar
/// `ctx:` readout) above what is actually sent.
fn outline_line_cost(line: &str) -> usize {
    (line.len() / 4).max(1)
}

/// A whole-workspace seed (`.`, `./`, empty) carries no personalization, so
/// the ranking is global and no file is `OWN`.
fn is_match_all_seed(seed: &str) -> bool {
    matches!(seed.trim(), "" | "." | "./")
}

/// M3 per-selection tracing (V9 §11 M3 acceptance: "planner logs show ...
/// graph file-score/mode per selection") plus the M0 aggregate
/// `graph_selection` event, which baseline tooling greps for. Token-unit
/// selections only.
fn trace_graph_selection(out: &[GraphCandidate], budget_tokens: usize) {
    for c in out {
        tracing::info!(
            target: "turn_metrics",
            path = %c.path.display(),
            rank_score = c.rank_score,
            specificity = c.specificity,
            decision = ?c.decision,
            confidence = ?c.confidence,
            token_estimate = c.token_estimate,
            "graph_candidate"
        );
    }
    let paths = |d: GraphDecision| -> Vec<String> {
        out.iter()
            .filter(|c| c.decision == d)
            .map(|c| c.path.to_string_lossy().to_string())
            .collect()
    };
    let tokens_used: usize = out.iter().map(|c| c.token_estimate).sum();
    tracing::info!(
        target: "turn_metrics",
        graph_full_content = ?paths(GraphDecision::FullAttach),
        graph_signatures = ?paths(GraphDecision::SignatureOnly),
        graph_token_estimate = tokens_used,
        graph_budget = budget_tokens,
        "graph_selection"
    );
}

/// C3 + C4: rank a set of files using mode-weighted PageRank +
/// specificity, sourced from the persisted [`store::GraphStore`].
///
/// Used by the MCP `blast_radius` handler: the SQL CTE picks which
/// files are reachable in the chosen mode, this function then orders
/// them by per-intent edge weights with HippoRAG-style stop-symbol
/// damping. Returns one `(score, specificity)` pair per file in
/// `affected`. Files absent from the graph (e.g. seed paths that don't
/// appear as edge endpoints) get `(0.0, 1.0)` so they don't get
/// silently dropped from the output.
pub fn rank_files_with_mode(
    store: &store::GraphStore,
    seeds: &[&str],
    affected: &[String],
    mode: store::BlastRadiusMode,
    config: SpecificityConfig,
) -> anyhow::Result<HashMap<String, (f64, f64)>> {
    rank_files_with_weights(
        store,
        seeds,
        affected,
        store::EdgeWeights::default_for(mode),
        config,
    )
}

/// Variant of [`rank_files_with_mode`] that takes an explicit
/// [`store::EdgeWeights`] map. Used by the MCP `blast_radius` handler
/// after merging `repoMap.edges.weights.<intent>` settings overrides
/// onto the mode preset.
pub fn rank_files_with_weights(
    store: &store::GraphStore,
    seeds: &[&str],
    affected: &[String],
    weights: store::EdgeWeights,
    config: SpecificityConfig,
) -> anyhow::Result<HashMap<String, (f64, f64)>> {
    rank_files_with_weights_cancellable(store, seeds, affected, weights, config, &|| false)
}

/// [`rank_files_with_weights`] that gives up with an error as soon as
/// `cancelled()` returns true, checked before each SQL step and inside the
/// per-file specificity loop (a statement that is already running is
/// stopped separately, by [`store::GraphCancel`]'s interrupt).
pub fn rank_files_with_weights_cancellable(
    store: &store::GraphStore,
    seeds: &[&str],
    affected: &[String],
    weights: store::EdgeWeights,
    config: SpecificityConfig,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<HashMap<String, (f64, f64)>> {
    use petgraph::graph::DiGraph;

    let check = || -> anyhow::Result<()> {
        if cancelled() {
            anyhow::bail!("graph query cancelled");
        }
        Ok(())
    };

    // 1. Pull the projected file edges and the universe of files.
    check()?;
    let edges = store.file_edges()?;
    let mut paths = store.all_file_paths()?;
    for s in seeds {
        if !paths.contains(&s.to_string()) {
            paths.push(s.to_string());
        }
    }
    for p in affected {
        if !paths.contains(p) {
            paths.push(p.clone());
        }
    }

    // 2. Build the DiGraph keyed by file path.
    let mut g: DiGraph<String, store::EdgeKind> = DiGraph::new();
    let mut idx_for: HashMap<String, NodeIndex> = HashMap::new();
    for p in &paths {
        let idx = g.add_node(p.clone());
        idx_for.insert(p.clone(), idx);
    }
    for (src, tgt, kind) in edges {
        if let (Some(&s_i), Some(&t_i)) = (idx_for.get(&src), idx_for.get(&tgt)) {
            g.add_edge(s_i, t_i, kind);
        }
    }

    // 3. C3 specificity at the file level: derive per-symbol IDF from
    //    the persisted nodes table, then average over each file's
    //    symbols. Mirrors `compute_specificity_map` but operates on
    //    SQLite-backed metadata so MCP doesn't have to rebuild the
    //    in-memory `RepoMap` graph.
    //
    //    Only query `symbols_in_file` for files we will actually emit
    //    (seeds + affected). Walking every path in the DB was an N+1
    //    that turned a 24k-file scratch tree into a multi-minute
    //    `blast_radius` while the MCP graph mutex blocked `node_doc`.
    check()?;
    let specificity_map: HashMap<NodeIndex, f64> = if config.enabled {
        let df = store.symbol_document_frequency()?;
        let n = paths.len().max(1) as f64;
        let mut max_raw = 0.0f64;
        let mut raw_by_symbol: HashMap<String, f64> = HashMap::new();
        for (sym, count) in &df {
            let ratio = *count as f64 / n;
            let raw = if ratio > config.stop_symbol_threshold {
                0.0
            } else {
                (n / *count as f64).ln().max(0.0)
            };
            max_raw = max_raw.max(raw);
            raw_by_symbol.insert(sym.clone(), raw);
        }
        let mut detail: HashSet<&str> = seeds.iter().copied().collect();
        for p in affected {
            detail.insert(p.as_str());
        }
        let mut map = HashMap::new();
        for p in &paths {
            let Some(&idx) = idx_for.get(p) else {
                continue;
            };
            if !detail.contains(p.as_str()) {
                map.insert(idx, 1.0);
                continue;
            }
            check()?;
            let symbols = store.symbols_in_file(p).unwrap_or_default();
            let s = if symbols.is_empty() {
                1.0
            } else if max_raw <= f64::EPSILON {
                0.0
            } else {
                let sum: f64 = symbols
                    .iter()
                    .map(|name| raw_by_symbol.get(name).copied().unwrap_or(0.0))
                    .sum();
                (sum / symbols.len() as f64 / max_raw).clamp(0.0, 1.0)
            };
            map.insert(idx, s);
        }
        map
    } else {
        idx_for.values().map(|&i| (i, 1.0)).collect()
    };

    // 4. Mode-weighted personalized PageRank seeded by the changed files.
    check()?;
    let seed_indices: Vec<NodeIndex> = seeds
        .iter()
        .filter_map(|s| idx_for.get(*s).copied())
        .collect();
    let ranks = page_rank::rank_nodes_weighted(
        &g,
        &seed_indices,
        0.85,
        15,
        Some(&specificity_map),
        |kind| weights.weight(*kind),
    );

    // 5. Project back to per-affected-file scores.
    let mut out = HashMap::new();
    for path in affected {
        let idx = idx_for.get(path).copied();
        let score = idx.and_then(|i| ranks.get(&i).copied()).unwrap_or(0.0);
        let specificity = idx
            .and_then(|i| specificity_map.get(&i).copied())
            .unwrap_or(1.0);
        out.insert(path.clone(), (score, specificity));
    }
    Ok(out)
}

// ── RepoMap ──────────────────────────────────────────────────

/// C3: per-build specificity configuration.
///
/// `enabled = false` skips the IDF computation entirely and emits a map
/// of `1.0` for every node — equivalent to disabling C3 weighting
/// without a code change. `stop_symbol_threshold` sets the file-frequency
/// ratio above which a symbol is treated as a stop symbol (specificity
/// 0.0). Defaults match the plan's recommendation.
#[derive(Debug, Clone, Copy)]
pub struct SpecificityConfig {
    pub enabled: bool,
    pub stop_symbol_threshold: f64,
}

impl Default for SpecificityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stop_symbol_threshold: 0.5,
        }
    }
}

/// Repo reference graph + PageRank ranker.
pub struct RepoMap {
    graph: DiGraph<FileNode, ReferenceEdge>,
    specificity_map: HashMap<NodeIndex, f64>,
}

impl RepoMap {
    /// Build a `RepoMap` by scanning `workspace`, skipping any path matching
    /// `excludes` (see [`builder::is_excluded`]).
    pub fn build(workspace: &Path, excludes: &[String]) -> anyhow::Result<Self> {
        Self::build_with_config(workspace, excludes, SpecificityConfig::default())
    }

    /// Variant of [`Self::build`] that takes an explicit
    /// [`SpecificityConfig`]. Used by the workspace settings layer to
    /// honor `repoMap.specificity.enabled` and
    /// `repoMap.specificity.stopSymbolThreshold` without rebuilding.
    ///
    /// Builds a throwaway in-memory code graph for `workspace`, projects it
    /// with [`Self::from_store`], and writes nothing to disk. Callers that
    /// already maintain a persisted graph should project it directly.
    pub fn build_with_config(
        workspace: &Path,
        excludes: &[String],
        config: SpecificityConfig,
    ) -> anyhow::Result<Self> {
        let store = store::GraphStore::open_memory()?;
        graph_builder::build_graph_into(&store, workspace, excludes)?;
        Self::from_store(&store, config)
    }

    /// Project a code graph into a `RepoMap`. RepoMap is a pure projection
    /// of the store: one file node per indexed file
    /// ([`store::GraphStore::outline_files`]), its symbols from the graph's
    /// nodes ([`store::GraphStore::outline_symbols`]) — two same-named
    /// symbols in one file appear once, as the graph keys nodes by
    /// `file::name` — and each [`store::GraphStore::file_edges`] row as a
    /// forward edge (referencing → referenced file) plus a reverse edge
    /// damped by [`REVERSE_EDGE_FACTOR`].
    pub fn from_store(store: &store::GraphStore, config: SpecificityConfig) -> anyhow::Result<Self> {
        let mut symbols_by_file: HashMap<String, Vec<Symbol>> = HashMap::new();
        for s in store.outline_symbols()? {
            symbols_by_file.entry(s.file_path).or_default().push(Symbol {
                name: s.name,
                kind: s.kind,
                line: s.line.unwrap_or(0).max(0) as usize,
                owner: s.owner,
            });
        }
        let mut graph: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
        let mut idx_for: HashMap<String, NodeIndex> = HashMap::new();
        for (path, bytes) in store.outline_files()? {
            let symbols = symbols_by_file.remove(&path).unwrap_or_default();
            let idx = graph.add_node(FileNode {
                path: PathBuf::from(&path),
                token_estimate: (bytes.max(0) as usize) / 4,
                symbols,
            });
            idx_for.insert(path, idx);
        }
        // Parallel edges are kept: multiplicity is weight, as in
        // `rank_files_with_weights_cancellable`.
        for (src, tgt, kind) in store.file_edges()? {
            if let (Some(&s), Some(&t)) = (idx_for.get(&src), idx_for.get(&tgt)) {
                graph.add_edge(s, t, ReferenceEdge::new(kind));
                graph.add_edge(t, s, ReferenceEdge::reversed(kind));
            }
        }
        let specificity_map = if config.enabled {
            compute_specificity_map(&graph, config.stop_symbol_threshold)
        } else {
            graph.node_indices().map(|idx| (idx, 1.0)).collect()
        };
        Ok(Self {
            graph,
            specificity_map,
        })
    }

    /// Number of edges in the reference graph, both directions counted
    /// (each code-graph edge contributes a forward and a reverse edge).
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// V9 §11 M3: structured per-file ranking for the planner.
    ///
    /// Same algorithm as [`Self::rank_for_agent`] but returns structured
    /// `Vec<GraphCandidate>` (V9 §4) instead of a pre-rendered prompt
    /// string. The planner consumes this directly and records each file's
    /// `GraphCandidateDecision` in the ledger so M4 can detect modified
    /// attached files and M7 can reason about which paths to upgrade
    /// from outline to full content without re-injecting.
    ///
    /// The legacy [`Self::rank_for_agent`] remains as a thin wrapper for
    /// the M0 baseline trace and any non-planner consumers; M10 deletes it.
    ///
    /// A match-all seed (`.`, `./`, empty) personalizes nothing: the ranking
    /// is global and no file is `OWN`.
    pub fn rank_for_agent_structured(
        &self,
        owned: &[String],
        budget_tokens: usize,
    ) -> Vec<GraphCandidate> {
        self.rank_for_agent_structured_with_mode(owned, budget_tokens, store::BlastRadiusMode::All)
    }

    /// C4: mode-aware structured ranking.
    ///
    /// Uses the per-intent edge weight preset from `BlastRadiusMode` so
    /// `mode=callers` ranks files by inbound `Calls` edges only,
    /// `mode=tests` by `TestOf` only, `mode=impact` by the recommended
    /// blend (Calls=1.0, Implements=0.9, Defines=0.8, Imports=0.5,
    /// TestOf=0.3, others=0.2), and `mode=all` reproduces the legacy
    /// uniform weighting. Match-all seeds (`.`, `./`, empty) are ignored.
    pub fn rank_for_agent_structured_with_mode(
        &self,
        owned: &[String],
        budget_tokens: usize,
        mode: store::BlastRadiusMode,
    ) -> Vec<GraphCandidate> {
        let out = self.rank_candidates(owned, budget_tokens, mode, &outline_line_cost);
        trace_graph_selection(&out, budget_tokens);
        out
    }

    /// Same ranking and FullAttach → SignatureOnly → PathOnly downgrade as
    /// [`Self::rank_for_agent_structured_with_mode`], but every candidate
    /// line is charged `cost(line)` against `budget`, both in the caller's
    /// unit. The MCP `repo_outline` tool uses it to budget the serialized
    /// JSON entry (bytes) instead of the injected line (tokens).
    ///
    /// Each candidate's `token_estimate` is its `cost` in that unit. When
    /// even the path-only line of the next-ranked file no longer fits, the
    /// selection stops there (the same rule as the injector), so the result
    /// is always a rank-ordered prefix. No tracing: the units are not tokens.
    /// With only match-all seeds (`.`, `./`, empty) the ranking is global.
    pub fn rank_with_line_cost(
        &self,
        owned: &[String],
        budget: usize,
        mode: store::BlastRadiusMode,
        cost: &dyn Fn(&str) -> usize,
    ) -> Vec<GraphCandidate> {
        self.rank_candidates(owned, budget, mode, cost)
    }

    fn rank_candidates(
        &self,
        owned: &[String],
        budget_tokens: usize,
        mode: store::BlastRadiusMode,
        line_cost: &dyn Fn(&str) -> usize,
    ) -> Vec<GraphCandidate> {
        // Filter match-all seeds first: `path_pattern::matches(".", _)` is
        // always true.
        let seeds: Vec<&String> = owned.iter().filter(|o| !is_match_all_seed(o)).collect();
        let owned_indices: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| {
                let p = self.graph[idx].path.to_string_lossy();
                seeds.iter().any(|o| crate::path_pattern::matches(o, &p))
            })
            .collect();

        let ranks: HashMap<NodeIndex, f64> = page_rank::rank_nodes_weighted(
            &self.graph,
            &owned_indices,
            0.85,
            15,
            Some(&self.specificity_map),
            |edge: &ReferenceEdge| edge.weight(mode),
        );

        let mut sorted: Vec<NodeIndex> = self.graph.node_indices().collect();
        sorted.sort_by(|a, b| {
            ranks
                .get(b)
                .unwrap_or(&0.0)
                .partial_cmp(ranks.get(a).unwrap_or(&0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Confidence buckets by rank percentile. Computed up-front so each
        // candidate carries its band; M7 uses confidence to decide whether
        // to upgrade outline → full attach.
        let total = sorted.len();
        let high_cutoff = (total as f64 * 0.10).ceil() as usize;
        let medium_cutoff = high_cutoff + (total as f64 * 0.30).ceil() as usize;

        let owned_set: std::collections::HashSet<NodeIndex> = owned_indices.into_iter().collect();
        let mut tokens_used: usize = 0;
        let mut out: Vec<GraphCandidate> = Vec::new();

        for (rank_idx, idx) in sorted.iter().enumerate() {
            // Anti-explosion guard; the token budget below is the primary bound.
            if out.len() >= OUTLINE_MAX_FILES {
                break;
            }
            let node = &self.graph[*idx];
            let path_str = node.path.to_string_lossy().to_string();
            let rank_score = *ranks.get(idx).unwrap_or(&0.0);
            let specificity = *self.specificity_map.get(idx).unwrap_or(&1.0);
            let confidence = if rank_idx < high_cutoff {
                GraphConfidence::High
            } else if rank_idx < medium_cutoff {
                GraphConfidence::Medium
            } else {
                GraphConfidence::Low
            };

            let remaining = budget_tokens.saturating_sub(tokens_used);

            // PathOnly is the cheapest tier and the budget floor: when even a
            // path line won't fit, the outline budget is spent — stop.
            let path_line = format!("  {} (s{:.2})", path_str, specificity);
            let path_cost = line_cost(&path_line);
            if path_cost > remaining {
                break;
            }

            // Pick the richest tier whose *outline-line* cost fits the
            // remaining budget, downgrading FullAttach → SignatureOnly →
            // PathOnly. Trading per-file depth keeps a file in the map rather
            // than dropping it once the budget tightens (breadth-preserving).
            let (decision, line, cost) = if owned_set.contains(idx) {
                let own_line = format!("  OWN {} (s{:.2})", path_str, specificity);
                let own_cost = line_cost(&own_line);
                if own_cost <= remaining {
                    (GraphDecision::FullAttach, own_line, own_cost)
                } else {
                    (GraphDecision::PathOnly, path_line, path_cost)
                }
            } else if !node.symbols.is_empty() {
                let sym_names: Vec<&str> = node.symbols.iter().map(|s| s.name.as_str()).collect();
                let sig_line = format!(
                    "  {} (s{:.2}) ({})",
                    path_str,
                    specificity,
                    sym_names.join(", ")
                );
                let sig_cost = line_cost(&sig_line);
                if sig_cost <= remaining {
                    (GraphDecision::SignatureOnly, sig_line, sig_cost)
                } else {
                    (GraphDecision::PathOnly, path_line, path_cost)
                }
            } else {
                (GraphDecision::PathOnly, path_line, path_cost)
            };

            // Symbols ride along on Full/Signature candidates (ledger + M7
            // upgrade); a PathOnly downgrade drops them from the rendered line.
            let symbols = match decision {
                GraphDecision::FullAttach | GraphDecision::SignatureOnly => node.symbols.clone(),
                GraphDecision::OutlineOnly | GraphDecision::PathOnly => Vec::new(),
            };

            tokens_used += cost;
            out.push(GraphCandidate {
                path: node.path.clone(),
                rank_score,
                specificity,
                confidence,
                decision,
                token_estimate: cost,
                symbols,
                rendered_line: line,
                content_digest: None,
            });
        }
        out
    }

    /// Rank files for an agent that owns `owned_paths`, within `budget_tokens`.
    ///
    /// **Legacy parity wrapper around [`Self::rank_for_agent_structured`].**
    /// Kept for M0 baseline tracing (`graph_selection` event below) and any
    /// non-planner consumer; M10 deletes it. New planner code calls
    /// `rank_for_agent_structured` directly.
    pub fn rank_for_agent(&self, owned: &[String], budget_tokens: usize) -> ContextPlan {
        let candidates = self.rank_for_agent_structured(owned, budget_tokens);

        let mut full_content = Vec::new();
        let mut signatures = Vec::new();
        let mut outline_lines = Vec::new();
        let mut tokens_used = 0usize;

        for c in &candidates {
            tokens_used += c.token_estimate;
            outline_lines.push(c.rendered_line.clone());
            match c.decision {
                GraphDecision::FullAttach => {
                    full_content.push(c.path.clone());
                }
                GraphDecision::SignatureOnly => {
                    signatures.push((c.path.clone(), c.symbols.clone()));
                }
                _ => {}
            }
        }

        let repo_outline = if outline_lines.is_empty() {
            String::new()
        } else {
            format!("Repo:\n{}", outline_lines.join("\n"))
        };

        // `graph_selection` aggregate event is emitted by
        // `rank_for_agent_structured` above — do not double-fire here.

        ContextPlan {
            full_content,
            signatures,
            repo_outline,
            token_estimate: tokens_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_workspace_builds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        assert_eq!(map.graph.node_count(), 0);
    }

    #[test]
    fn rank_for_agent_returns_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}").unwrap();

        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let plan = map.rank_for_agent(&["main.rs".to_string()], 10_000);
        assert!(!plan.repo_outline.is_empty() || map.graph.node_count() == 0);
    }

    #[test]
    fn m3_rank_for_agent_structured_emits_per_file_candidates() {
        // V9 §11 M3 acceptance: per-file (path, score, confidence,
        // decision, token_estimate, symbols) returned to the planner.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}").unwrap();
        std::fs::write(dir.path().join("util.rs"), "pub fn other() {}").unwrap();

        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let candidates = map.rank_for_agent_structured(&["main.rs".to_string()], 10_000);
        if map.graph.node_count() == 0 {
            // Graph build is heuristic — skip if empty workspace detected.
            return;
        }
        assert!(!candidates.is_empty(), "expected per-file candidates");
        for c in &candidates {
            // Every candidate carries a path and a decision.
            assert!(c.path.components().next().is_some());
            assert!(c.token_estimate > 0);
            assert!(matches!(
                c.decision,
                GraphDecision::FullAttach
                    | GraphDecision::SignatureOnly
                    | GraphDecision::OutlineOnly
                    | GraphDecision::PathOnly
            ));
            // rendered_line is non-empty so the renderer can use it
            // verbatim for byte-identity with `rank_for_agent`.
            assert!(!c.rendered_line.is_empty());
        }
        // The owned file should be tagged FullAttach.
        let owned = candidates
            .iter()
            .find(|c| c.path.to_string_lossy().contains("main.rs"));
        if let Some(owned) = owned {
            assert_eq!(owned.decision, GraphDecision::FullAttach);
            assert!(owned.rendered_line.contains("OWN "));
        }
    }

    #[test]
    fn rank_files_with_mode_orders_by_intent() {
        // Build a persisted graph where seed.rs is referenced by two
        // dependents: caller.rs via `Calls` and tester.rs via `TestOf`.
        // mode=Callers should rank caller.rs above tester.rs;
        // mode=Tests should invert.
        use crate::repo_map::store::{BlastRadiusMode, EdgeKind, GraphStore, NodeKind};
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "process",
                "seed.rs::process",
                "seed.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "caller_fn",
                "caller.rs::caller_fn",
                "caller.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .upsert_node(
                NodeKind::Function,
                "test_process",
                "tester.rs::test_process",
                "tester.rs",
                Some(1),
                None,
                Some("rust"),
                None,
            )
            .unwrap();
        store
            .insert_edge(
                EdgeKind::Calls,
                "caller.rs::caller_fn",
                "seed.rs::process",
                "caller.rs",
                3,
            )
            .unwrap();
        store
            .insert_edge(
                EdgeKind::TestOf,
                "tester.rs::test_process",
                "seed.rs::process",
                "tester.rs",
                3,
            )
            .unwrap();

        let affected = vec![
            "caller.rs".to_string(),
            "tester.rs".to_string(),
            "seed.rs".to_string(),
        ];
        let callers = rank_files_with_mode(
            &store,
            &["seed.rs"],
            &affected,
            BlastRadiusMode::Callers,
            SpecificityConfig::default(),
        )
        .unwrap();
        let tests = rank_files_with_mode(
            &store,
            &["seed.rs"],
            &affected,
            BlastRadiusMode::Tests,
            SpecificityConfig::default(),
        )
        .unwrap();
        assert!(
            callers["caller.rs"].0 >= callers["tester.rs"].0,
            "Callers mode must rank Calls-reached at least as high: {callers:?}"
        );
        assert!(
            tests["tester.rs"].0 >= tests["caller.rs"].0,
            "Tests mode must rank TestOf-reached at least as high: {tests:?}"
        );
        // specificity in [0, 1] for every affected file.
        for (_, sp) in callers.values() {
            assert!((0.0..=1.0).contains(sp));
        }
    }

    #[test]
    fn rank_with_mode_callers_zeros_non_call_edges() {
        // Build a tiny in-memory graph where the seed has two outgoing
        // neighbours, one reached only by a `Calls` edge and one only by
        // a `TestOf` edge. Under `mode=Callers`, the `TestOf`-reached
        // node must lose all PageRank propagation (weight 0.0) and
        // therefore rank below the `Calls`-reached node. Under
        // `mode=Tests` the relationship inverts.
        use crate::repo_map::store::{BlastRadiusMode, EdgeKind};
        let mut g: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
        let seed = g.add_node(FileNode {
            path: PathBuf::from("seed.rs"),
            token_estimate: 10,
            symbols: vec![Symbol {
                name: "seed".into(),
                kind: "function_item".into(),
                line: 0,
                owner: None,
            }],
        });
        let by_call = g.add_node(FileNode {
            path: PathBuf::from("by_call.rs"),
            token_estimate: 10,
            symbols: vec![Symbol {
                name: "by_call".into(),
                kind: "function_item".into(),
                line: 0,
                owner: None,
            }],
        });
        let by_test = g.add_node(FileNode {
            path: PathBuf::from("by_test.rs"),
            token_estimate: 10,
            symbols: vec![Symbol {
                name: "by_test".into(),
                kind: "function_item".into(),
                line: 0,
                owner: None,
            }],
        });
        g.add_edge(seed, by_call, ReferenceEdge::new(EdgeKind::Calls));
        g.add_edge(seed, by_test, ReferenceEdge::new(EdgeKind::TestOf));

        let specificity_map = compute_specificity_map(&g, 0.5);
        let map = RepoMap {
            graph: g,
            specificity_map,
        };

        let callers = map.rank_for_agent_structured_with_mode(
            &["seed.rs".to_string()],
            10_000,
            BlastRadiusMode::Callers,
        );
        let tests = map.rank_for_agent_structured_with_mode(
            &["seed.rs".to_string()],
            10_000,
            BlastRadiusMode::Tests,
        );

        let score = |cands: &[GraphCandidate], path: &str| -> f64 {
            cands
                .iter()
                .find(|c| c.path.to_string_lossy() == path)
                .map(|c| c.rank_score)
                .unwrap_or(0.0)
        };
        assert!(
            score(&callers, "by_call.rs") > score(&callers, "by_test.rs"),
            "mode=Callers must rank Calls-reached higher: callers={callers:?}"
        );
        assert!(
            score(&tests, "by_test.rs") > score(&tests, "by_call.rs"),
            "mode=Tests must rank TestOf-reached higher: tests={tests:?}"
        );
    }

    #[test]
    fn fullattach_charges_outline_line_cost_not_full_content() {
        // D1: an owned file with a huge full-content estimate must still fit
        // a tiny outline budget — the outline injects only its `OWN <path>`
        // line, so it is charged the line cost, not `node.token_estimate`.
        // (Old behaviour reserved 10_000 > budget and dropped the file.)
        let mut g: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
        g.add_node(FileNode {
            path: PathBuf::from("a.rs"),
            token_estimate: 10_000,
            symbols: Vec::new(),
        });
        let specificity_map = compute_specificity_map(&g, 0.5);
        let map = RepoMap {
            graph: g,
            specificity_map,
        };

        let cands = map.rank_for_agent_structured(&["a.rs".to_string()], 50);
        let a = cands
            .iter()
            .find(|c| c.path.to_string_lossy() == "a.rs")
            .expect("owned file must appear, not be dropped for over-budget full content");
        assert_eq!(a.decision, GraphDecision::FullAttach);
        assert!(
            a.token_estimate < 50,
            "FullAttach must be charged the outline line cost, got {}",
            a.token_estimate
        );
    }

    /// The serialization-aware cost hook keeps the downgrade: a file whose
    /// signature line does not fit the byte budget still appears as a
    /// path-only line, and the selection is a rank-ordered prefix whose
    /// summed cost never exceeds the budget.
    #[test]
    fn line_cost_hook_downgrades_to_pathonly_within_a_byte_budget() {
        let mut g: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
        let a = g.add_node(FileNode {
            path: PathBuf::from("owned.rs"),
            token_estimate: 9_000,
            symbols: Vec::new(),
        });
        let symbols: Vec<Symbol> = (0..40)
            .map(|i| Symbol {
                name: format!("a_long_symbol_name_number_{i}"),
                kind: "function_item".into(),
                line: 0,
                owner: None,
            })
            .collect();
        let b = g.add_node(FileNode {
            path: PathBuf::from("heavy.rs"),
            token_estimate: 10,
            symbols,
        });
        g.add_edge(
            a,
            b,
            ReferenceEdge::new(crate::repo_map::store::EdgeKind::Calls),
        );
        let specificity_map = compute_specificity_map(&g, 0.5);
        let map = RepoMap {
            graph: g,
            specificity_map,
        };
        // Bytes plus a JSON-ish per-entry overhead.
        let cost = |line: &str| line.len() + 16;
        let budget = 160;
        let cands = map.rank_with_line_cost(
            &["owned.rs".to_string()],
            budget,
            store::BlastRadiusMode::All,
            &cost,
        );
        let heavy = cands
            .iter()
            .find(|c| c.path.to_string_lossy() == "heavy.rs")
            .expect("heavy.rs must be downgraded, not dropped");
        assert_eq!(heavy.decision, GraphDecision::PathOnly);
        assert!(
            !heavy.rendered_line.contains("a_long_symbol"),
            "{}",
            heavy.rendered_line.trim()
        );
        let spent: usize = cands.iter().map(|c| c.token_estimate).sum();
        assert!(spent <= budget, "{spent} > {budget}");
        for c in &cands {
            assert_eq!(c.token_estimate, cost(&c.rendered_line));
        }
        // The token-unit injector path is unchanged by the hook.
        let injected = map.rank_for_agent_structured(&["owned.rs".to_string()], 40);
        assert!(injected.iter().all(|c| c.token_estimate == outline_line_cost(&c.rendered_line)));
    }

    #[test]
    fn ranking_stops_when_cancelled() {
        let store = store::GraphStore::open_memory().unwrap();
        let affected = vec!["a.rs".to_string()];
        let weights = store::EdgeWeights::default_for(store::BlastRadiusMode::All);
        let config = SpecificityConfig::default();
        let err = rank_files_with_weights_cancellable(
            &store,
            &["a.rs"],
            &affected,
            weights,
            config,
            &|| true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        let ok = rank_files_with_weights(&store, &["a.rs"], &affected, weights, config).unwrap();
        assert!(ok.contains_key("a.rs"));
    }

    #[test]
    fn downgrades_to_pathonly_instead_of_dropping() {
        // D2: a non-owned, symbol-heavy file whose full signature line won't
        // fit the budget must still appear — downgraded to PathOnly — rather
        // than vanishing from the map.
        let mut g: DiGraph<FileNode, ReferenceEdge> = DiGraph::new();
        let a = g.add_node(FileNode {
            path: PathBuf::from("owned.rs"),
            token_estimate: 9_000,
            symbols: Vec::new(),
        });
        let symbols: Vec<Symbol> = (0..40)
            .map(|i| Symbol {
                name: format!("a_long_symbol_name_number_{i}"),
                kind: "function_item".into(),
                line: 0,
                owner: None,
            })
            .collect();
        let b = g.add_node(FileNode {
            path: PathBuf::from("heavy.rs"),
            token_estimate: 10,
            symbols,
        });
        g.add_edge(
            a,
            b,
            ReferenceEdge::new(crate::repo_map::store::EdgeKind::Calls),
        );
        let specificity_map = compute_specificity_map(&g, 0.5);
        let map = RepoMap {
            graph: g,
            specificity_map,
        };

        // Budget fits short lines but never heavy.rs's 40-symbol signature.
        let cands = map.rank_for_agent_structured(&["owned.rs".to_string()], 40);
        let heavy = cands
            .iter()
            .find(|c| c.path.to_string_lossy() == "heavy.rs")
            .expect("symbol-heavy file must be downgraded, not dropped");
        assert_eq!(
            heavy.decision,
            GraphDecision::PathOnly,
            "heavy.rs should downgrade to PathOnly under budget pressure"
        );
        assert!(
            heavy.symbols.is_empty(),
            "a PathOnly downgrade drops symbols from the rendered line"
        );
        assert!(
            cands.iter().any(|c| c.path.to_string_lossy() == "owned.rs"),
            "the owned file is still present alongside the downgraded one"
        );
    }

    #[test]
    fn m3_rank_for_agent_structured_and_legacy_agree_on_outline() {
        // Pins byte-identity: building the legacy `repo_outline` from
        // structured candidates' rendered_line entries must equal the
        // outline `rank_for_agent` returns directly.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}").unwrap();

        let map = RepoMap::build(dir.path(), &[]).expect("build");
        if map.graph.node_count() == 0 {
            return;
        }
        let candidates = map.rank_for_agent_structured(&["main.rs".to_string()], 10_000);
        let lines: Vec<String> = candidates.iter().map(|c| c.rendered_line.clone()).collect();
        let expected_outline = if lines.is_empty() {
            String::new()
        } else {
            format!("Repo:\n{}", lines.join("\n"))
        };
        let plan = map.rank_for_agent(&["main.rs".to_string()], 10_000);
        assert_eq!(plan.repo_outline, expected_outline);
    }

    // ── code-graph edges ─────────────────────────────────────────

    /// `src/seed.rs` calls `src/zz_callee.rs` and is called by
    /// `src/yy_caller.rs`; `src/aa_unrelated.rs` links to nothing. The
    /// unique bare-call names resolve across files. Plus three files with
    /// no tree-sitter grammar.
    fn linked_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("seed.rs"), "pub fn gv_seed_fn() { gv_callee_fn(); }\n").unwrap();
        std::fs::write(src.join("zz_callee.rs"), "pub fn gv_callee_fn() {}\n").unwrap();
        std::fs::write(src.join("yy_caller.rs"), "pub fn gv_caller_fn() { gv_seed_fn(); }\n")
            .unwrap();
        std::fs::write(src.join("aa_unrelated.rs"), "pub fn gv_unrelated_fn() {}\n").unwrap();
        std::fs::write(src.join("notes.md"), "# notes\n").unwrap();
        std::fs::write(src.join("build.log"), "log line\n").unwrap();
        std::fs::write(src.join("Cargo.lock"), "# lock\n").unwrap();
        dir
    }

    fn node_index(map: &RepoMap, path: &str) -> NodeIndex {
        map.graph
            .node_indices()
            .find(|&i| map.graph[i].path.to_string_lossy() == path)
            .unwrap_or_else(|| panic!("no node for {path}"))
    }

    fn position(cands: &[GraphCandidate], path: &str) -> usize {
        cands
            .iter()
            .position(|c| c.path.to_string_lossy() == path)
            .unwrap_or_else(|| panic!("{path} not in candidates"))
    }

    fn score(cands: &[GraphCandidate], path: &str) -> f64 {
        cands[position(cands, path)].rank_score
    }

    #[test]
    fn build_lists_only_code_graph_files() {
        let dir = linked_workspace();
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let mut paths: Vec<String> = map
            .graph
            .node_indices()
            .map(|i| map.graph[i].path.to_string_lossy().into_owned())
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "src/aa_unrelated.rs",
                "src/seed.rs",
                "src/yy_caller.rs",
                "src/zz_callee.rs"
            ]
        );
    }

    #[test]
    fn build_adds_forward_and_reverse_edges() {
        let dir = linked_workspace();
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        assert!(map.edge_count() >= 4, "edges: {}", map.edge_count());
        let seed = node_index(&map, "src/seed.rs");
        let callee = node_index(&map, "src/zz_callee.rs");
        let forward = map.graph.edges_connecting(seed, callee).collect::<Vec<_>>();
        assert!(
            forward.iter().any(|e| !e.weight().reverse),
            "expected a forward seed → callee edge"
        );
        let back = map.graph.edges_connecting(callee, seed).collect::<Vec<_>>();
        assert!(
            back.iter().any(|e| e.weight().reverse),
            "expected a reverse callee → seed edge"
        );
    }

    #[test]
    fn from_store_matches_in_memory_build() {
        let dir = linked_workspace();
        let store = store::GraphStore::open_memory().unwrap();
        graph_builder::build_graph_into(&store, dir.path(), &[]).unwrap();
        let from_store =
            RepoMap::from_store(&store, SpecificityConfig::default()).expect("from_store");
        let in_memory = RepoMap::build(dir.path(), &[]).expect("build");
        assert!(from_store.edge_count() > 0);
        assert_eq!(from_store.edge_count(), in_memory.edge_count());
    }

    #[test]
    fn seeded_rank_prefers_callee_then_caller_then_unrelated() {
        let dir = linked_workspace();
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let cands = map.rank_for_agent_structured(&["src/seed.rs".into()], 100_000);
        let callee = score(&cands, "src/zz_callee.rs");
        let caller = score(&cands, "src/yy_caller.rs");
        let unrelated = score(&cands, "src/aa_unrelated.rs");
        assert!(callee > caller, "callee {callee} vs caller {caller}");
        assert!(caller > unrelated, "caller {caller} vs unrelated {unrelated}");
    }

    #[test]
    fn linked_file_listed_before_unlinked() {
        let dir = linked_workspace();
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let cands = map.rank_for_agent_structured(&["src/seed.rs".into()], 100_000);
        assert!(
            position(&cands, "src/zz_callee.rs") < position(&cands, "src/aa_unrelated.rs"),
            "walk order would list aa_ first"
        );
    }

    #[test]
    fn match_all_seed_is_unseeded() {
        let dir = linked_workspace();
        let map = RepoMap::build(dir.path(), &[]).expect("build");
        let dot = map.rank_for_agent_structured(&[".".into()], 100_000);
        assert!(!dot.is_empty());
        assert!(dot.iter().all(|c| c.decision != GraphDecision::FullAttach));
        assert!(dot.iter().all(|c| !c.rendered_line.contains("OWN ")));
        assert!(dot.iter().any(|c| c.rendered_line.contains("(gv_")));
        let none = map.rank_for_agent_structured(&[], 100_000);
        let paths = |cs: &[GraphCandidate]| cs.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&dot), paths(&none));
    }
}
