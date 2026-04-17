//! Personalized PageRank via the power method.
//!
//! Used to rank files in the repo graph by relevance to a given agent's
//! owned paths. Files that are closer (in the reference graph) to owned
//! files receive higher scores and are included first in the context plan.
//!
//! ## Algorithm
//! Standard damped PageRank with a personalization vector:
//! - Owned file nodes get a 50× initial weight in the personalization vector
//! - Damping factor d = 0.85 (standard)
//! - Power iterations until convergence or max_iterations reached

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};

/// Run personalized PageRank on `graph`.
///
/// Returns a map from `NodeIndex` to rank score (not normalized).
///
/// - `personalized_nodes`: nodes that get extra weight (50× in start vector)
/// - `damping`: typically 0.85
/// - `max_iterations`: typically 10–20 (sufficient for repo-map approximation)
pub fn rank_nodes<N, E>(
    graph: &DiGraph<N, E>,
    personalized_nodes: &[NodeIndex],
    damping: f64,
    max_iterations: usize,
) -> HashMap<NodeIndex, f64> {
    let n = graph.node_count();
    if n == 0 {
        return HashMap::new();
    }

    // Build personalization vector
    let base_weight = 1.0 / n as f64;
    let owned_boost = 50.0 * base_weight;
    let owned_set: std::collections::HashSet<NodeIndex> =
        personalized_nodes.iter().copied().collect();

    let mut personalization: HashMap<NodeIndex, f64> = graph
        .node_indices()
        .map(|idx| {
            let w = if owned_set.contains(&idx) {
                owned_boost
            } else {
                base_weight
            };
            (idx, w)
        })
        .collect();

    // Normalize personalization so it sums to 1
    let p_sum: f64 = personalization.values().sum();
    if p_sum > 0.0 {
        for v in personalization.values_mut() {
            *v /= p_sum;
        }
    }

    // Initial rank: uniform
    let mut rank: HashMap<NodeIndex, f64> = graph
        .node_indices()
        .map(|idx| (idx, 1.0 / n as f64))
        .collect();

    // Pre-compute out-degrees
    let out_degree: HashMap<NodeIndex, usize> = graph
        .node_indices()
        .map(|idx| {
            (
                idx,
                graph.neighbors_directed(idx, Direction::Outgoing).count(),
            )
        })
        .collect();

    // Power iterations
    for _ in 0..max_iterations {
        let mut new_rank: HashMap<NodeIndex, f64> =
            graph.node_indices().map(|idx| (idx, 0.0)).collect();

        // Dangling mass: nodes with no outgoing edges contribute to all nodes
        let dangling_mass: f64 = graph
            .node_indices()
            .filter(|idx| out_degree[idx] == 0)
            .map(|idx| rank[&idx])
            .sum();

        for idx in graph.node_indices() {
            // Collect incoming rank
            let incoming: f64 = graph
                .neighbors_directed(idx, Direction::Incoming)
                .map(|src| {
                    let deg = out_degree[&src].max(1) as f64;
                    rank[&src] / deg
                })
                .sum();

            let r = new_rank.get_mut(&idx).unwrap();
            *r = (1.0 - damping) * personalization[&idx]
                + damping * (incoming + dangling_mass * personalization[&idx]);
        }

        rank = new_rank;
    }

    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_returns_entry_for_each_node() {
        let mut g: DiGraph<(), ()> = DiGraph::new();
        let a = g.add_node(());
        let b = g.add_node(());
        let c = g.add_node(());
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());

        let ranks = rank_nodes(&g, &[a], 0.85, 10);
        assert_eq!(ranks.len(), 3);
    }

    #[test]
    fn personalized_node_ranks_highest() {
        let mut g: DiGraph<(), ()> = DiGraph::new();
        let a = g.add_node(());
        let b = g.add_node(());
        let c = g.add_node(());
        g.add_edge(a, b, ());
        g.add_edge(a, c, ());

        let ranks = rank_nodes(&g, &[a], 0.85, 20);
        assert!(ranks[&a] >= ranks[&b]);
        assert!(ranks[&a] >= ranks[&c]);
    }

    #[test]
    fn empty_graph_returns_empty() {
        let g: DiGraph<(), ()> = DiGraph::new();
        let ranks = rank_nodes(&g, &[], 0.85, 10);
        assert!(ranks.is_empty());
    }
}
