//! Repo-map: context budget planning for multi-agent swarm runs.
//!
//! Builds a reference graph of source files using tree-sitter symbol
//! extraction, then uses personalized PageRank to rank files by relevance to
//! an agent's owned paths. The resulting `ContextPlan` tells the runner which
//! files to include as full content vs. signatures-only vs. just a path list.
//!
//! ## Usage
//! ```ignore
//! let repo_map = RepoMap::build(&workspace_root, &[])?;
//! let plan = repo_map.rank_for_agent(&["src/auth/"], 32_000);
//! // Inject plan.repo_outline into the agent's system prompt
//! ```
//!
//! See Phase 5 of the implementation plan.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use petgraph::graph::{DiGraph, NodeIndex};

pub mod builder;
pub mod edges;
pub mod graph_builder;
pub mod page_rank;
pub mod store;

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
}

/// A typed edge between two file nodes.
#[derive(Debug, Clone)]
pub struct ReferenceEdge {
    pub kind: store::EdgeKind,
}

impl ReferenceEdge {
    pub fn new(kind: store::EdgeKind) -> Self {
        Self { kind }
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
    use petgraph::graph::DiGraph;

    // 1. Pull the projected file edges and the universe of files.
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
        let mut map = HashMap::new();
        for p in &paths {
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
            if let Some(&idx) = idx_for.get(p) {
                map.insert(idx, s);
            }
        }
        map
    } else {
        idx_for.values().map(|&i| (i, 1.0)).collect()
    };

    // 4. Mode-weighted personalized PageRank seeded by the changed files.
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
    pub fn build_with_config(
        workspace: &Path,
        excludes: &[String],
        config: SpecificityConfig,
    ) -> anyhow::Result<Self> {
        let graph = builder::build(workspace, excludes)?;
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
    /// uniform weighting.
    pub fn rank_for_agent_structured_with_mode(
        &self,
        owned: &[String],
        budget_tokens: usize,
        mode: store::BlastRadiusMode,
    ) -> Vec<GraphCandidate> {
        let owned_indices: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| {
                let p = self.graph[idx].path.to_string_lossy();
                owned
                    .iter()
                    .any(|o| o == "." || o == "./" || crate::path_pattern::matches(o, &p))
            })
            .collect();

        let ranks: HashMap<NodeIndex, f64> = page_rank::rank_nodes_weighted(
            &self.graph,
            &owned_indices,
            0.85,
            15,
            Some(&self.specificity_map),
            |edge: &ReferenceEdge| mode.edge_weight(edge.kind),
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
            if tokens_used >= budget_tokens {
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

            if owned_set.contains(idx) {
                if tokens_used + node.token_estimate <= budget_tokens {
                    let line = format!("  [owned] {} [sp {:.2}]", path_str, specificity);
                    out.push(GraphCandidate {
                        path: node.path.clone(),
                        rank_score,
                        specificity,
                        confidence,
                        decision: GraphDecision::FullAttach,
                        token_estimate: node.token_estimate,
                        symbols: node.symbols.clone(),
                        rendered_line: line,
                        content_digest: None,
                    });
                    tokens_used += node.token_estimate;
                }
            } else if !node.symbols.is_empty() {
                let sig_tokens = node.symbols.len() * 10;
                if tokens_used + sig_tokens <= budget_tokens {
                    let syms = node.symbols.clone();
                    let sym_names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
                    let line = format!(
                        "  {} [sp {:.2}] ({})",
                        path_str,
                        specificity,
                        sym_names.join(", ")
                    );
                    out.push(GraphCandidate {
                        path: node.path.clone(),
                        rank_score,
                        specificity,
                        confidence,
                        decision: GraphDecision::SignatureOnly,
                        token_estimate: sig_tokens,
                        symbols: syms,
                        rendered_line: line,
                        content_digest: None,
                    });
                    tokens_used += sig_tokens;
                }
            } else {
                let line = format!("  {} [sp {:.2}]", path_str, specificity);
                out.push(GraphCandidate {
                    path: node.path.clone(),
                    rank_score,
                    specificity,
                    confidence,
                    decision: GraphDecision::PathOnly,
                    token_estimate: 5,
                    symbols: Vec::new(),
                    rendered_line: line,
                    content_digest: None,
                });
                tokens_used += 5;
            }
        }

        // M3 per-selection tracing (V9 §11 M3 acceptance: "planner logs show
        // ... graph file-score/mode per selection").
        for c in &out {
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

        // M0 legacy aggregate event — emit even when consumers call the
        // structured API directly, so M0 baseline tooling (which greps for
        // `graph_selection`) keeps working unchanged.
        let full_paths: Vec<String> = out
            .iter()
            .filter(|c| c.decision == GraphDecision::FullAttach)
            .map(|c| c.path.to_string_lossy().to_string())
            .collect();
        let sig_paths: Vec<String> = out
            .iter()
            .filter(|c| c.decision == GraphDecision::SignatureOnly)
            .map(|c| c.path.to_string_lossy().to_string())
            .collect();
        tracing::info!(
            target: "turn_metrics",
            graph_full_content = ?full_paths,
            graph_signatures = ?sig_paths,
            graph_token_estimate = tokens_used,
            graph_budget = budget_tokens,
            "graph_selection"
        );

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
            format!("## Repository context:\n{}", outline_lines.join("\n"))
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
            assert!(owned.rendered_line.contains("[owned]"));
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
            }],
        });
        let by_call = g.add_node(FileNode {
            path: PathBuf::from("by_call.rs"),
            token_estimate: 10,
            symbols: vec![Symbol {
                name: "by_call".into(),
                kind: "function_item".into(),
                line: 0,
            }],
        });
        let by_test = g.add_node(FileNode {
            path: PathBuf::from("by_test.rs"),
            token_estimate: 10,
            symbols: vec![Symbol {
                name: "by_test".into(),
                kind: "function_item".into(),
                line: 0,
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
            format!("## Repository context:\n{}", lines.join("\n"))
        };
        let plan = map.rank_for_agent(&["main.rs".to_string()], 10_000);
        assert_eq!(plan.repo_outline, expected_outline);
    }
}
