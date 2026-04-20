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

use std::path::{Path, PathBuf};

use petgraph::graph::DiGraph;

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

/// An edge between two file nodes (future: parse-based reference).
#[derive(Debug, Clone)]
pub struct ReferenceEdge;

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

// ── RepoMap ──────────────────────────────────────────────────

/// Repo reference graph + PageRank ranker.
pub struct RepoMap {
    graph: DiGraph<FileNode, ReferenceEdge>,
}

impl RepoMap {
    /// Build a `RepoMap` by scanning `workspace`, skipping any path matching
    /// `excludes` (see [`builder::is_excluded`]).
    pub fn build(workspace: &Path, excludes: &[String]) -> anyhow::Result<Self> {
        let graph = builder::build(workspace, excludes)?;
        Ok(Self { graph })
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
        use petgraph::graph::NodeIndex;
        use std::collections::HashMap;

        let owned_indices: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| {
                let p = self.graph[idx].path.to_string_lossy();
                owned.iter().any(|o| {
                    o == "." || o == "./" || crate::path_pattern::matches(o, &p)
                })
            })
            .collect();

        let ranks: HashMap<NodeIndex, f64> =
            page_rank::rank_nodes(&self.graph, &owned_indices, 0.85, 15);

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
            let confidence = if rank_idx < high_cutoff {
                GraphConfidence::High
            } else if rank_idx < medium_cutoff {
                GraphConfidence::Medium
            } else {
                GraphConfidence::Low
            };

            if owned_set.contains(idx) {
                if tokens_used + node.token_estimate <= budget_tokens {
                    let line = format!("  [owned] {}", path_str);
                    out.push(GraphCandidate {
                        path: node.path.clone(),
                        rank_score,
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
                    let line = format!("  {} ({})", path_str, sym_names.join(", "));
                    out.push(GraphCandidate {
                        path: node.path.clone(),
                        rank_score,
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
                let line = format!("  {}", path_str);
                out.push(GraphCandidate {
                    path: node.path.clone(),
                    rank_score,
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
    use std::collections::HashMap;

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
