//! Repo-map: context budget planning for multi-agent swarm runs.
//!
//! Builds a reference graph of source files using tree-sitter symbol
//! extraction, then uses personalized PageRank to rank files by relevance to
//! an agent's owned paths. The resulting `ContextPlan` tells the runner which
//! files to include as full content vs. signatures-only vs. just a path list.
//!
//! ## Usage
//! ```ignore
//! let repo_map = RepoMap::build(&workspace_root)?;
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

// ── RepoMap ──────────────────────────────────────────────────

/// Repo reference graph + PageRank ranker.
pub struct RepoMap {
    graph: DiGraph<FileNode, ReferenceEdge>,
}

impl RepoMap {
    /// Build a `RepoMap` by scanning `workspace`.
    pub fn build(workspace: &Path) -> anyhow::Result<Self> {
        let graph = builder::build(workspace)?;
        Ok(Self { graph })
    }

    /// Rank files for an agent that owns `owned_paths`, within `budget_tokens`.
    ///
    /// Algorithm:
    /// 1. Find node indices for all nodes whose path starts with an owned prefix
    /// 2. Run personalized PageRank seeding those nodes
    /// 3. Sort all nodes by descending rank score
    /// 4. Fill the budget: owned files first as full content, then high-rank as signatures
    pub fn rank_for_agent(&self, owned: &[String], budget_tokens: usize) -> ContextPlan {
        use petgraph::graph::NodeIndex;
        use std::collections::HashMap;

        // Find owned node indices
        let owned_indices: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| {
                let p = self.graph[idx].path.to_string_lossy();
                owned.iter().any(|o| p.starts_with(o.as_str()) || *o == "." || *o == "./")
            })
            .collect();

        // Run PageRank
        let ranks: HashMap<NodeIndex, f64> =
            page_rank::rank_nodes(&self.graph, &owned_indices, 0.85, 15);

        // Sort nodes by descending rank
        let mut sorted: Vec<NodeIndex> = self.graph.node_indices().collect();
        sorted.sort_by(|a, b| {
            ranks.get(b).unwrap_or(&0.0)
                .partial_cmp(ranks.get(a).unwrap_or(&0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut full_content = Vec::new();
        let mut signatures = Vec::new();
        let mut tokens_used = 0usize;
        let mut outline_lines = Vec::new();

        let owned_set: std::collections::HashSet<NodeIndex> = owned_indices.into_iter().collect();

        for idx in sorted {
            if tokens_used >= budget_tokens {
                break;
            }
            let node = &self.graph[idx];
            let path_str = node.path.to_string_lossy().to_string();

            if owned_set.contains(&idx) {
                // Owned files: full content (within budget)
                if tokens_used + node.token_estimate <= budget_tokens {
                    full_content.push(node.path.clone());
                    tokens_used += node.token_estimate;
                    outline_lines.push(format!("  [owned] {}", path_str));
                }
            } else if !node.symbols.is_empty() {
                // High-rank files: signatures only (cheap)
                let sig_tokens = node.symbols.len() * 10; // ~10 tokens per symbol signature
                if tokens_used + sig_tokens <= budget_tokens {
                    let syms = node.symbols.clone();
                    let sym_names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
                    outline_lines.push(format!("  {} ({})", path_str, sym_names.join(", ")));
                    signatures.push((node.path.clone(), syms));
                    tokens_used += sig_tokens;
                }
            } else {
                // Files with no symbols: just list the path
                outline_lines.push(format!("  {}", path_str));
                tokens_used += 5;
            }
        }

        let repo_outline = if outline_lines.is_empty() {
            String::new()
        } else {
            format!("## Repository context:\n{}", outline_lines.join("\n"))
        };

        // M0 instrumentation: expose ranked selection so baselines can
        // measure graph-pruning effectiveness. Safe under V9 §2 — no API
        // change; structured graph returns land in M3.
        let full_paths: Vec<String> = full_content
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let sig_paths: Vec<String> = signatures
            .iter()
            .map(|(p, _)| p.to_string_lossy().to_string())
            .collect();
        tracing::info!(
            target: "turn_metrics",
            graph_full_content = ?full_paths,
            graph_signatures = ?sig_paths,
            graph_token_estimate = tokens_used,
            graph_budget = budget_tokens,
            "graph_selection"
        );

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
        let map = RepoMap::build(dir.path()).expect("build");
        assert_eq!(map.graph.node_count(), 0);
    }

    #[test]
    fn rank_for_agent_returns_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}").unwrap();

        let map = RepoMap::build(dir.path()).expect("build");
        let plan = map.rank_for_agent(&["main.rs".to_string()], 10_000);
        assert!(!plan.repo_outline.is_empty() || map.graph.node_count() == 0);
    }
}
