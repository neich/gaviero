//! Compiled execution plan: a `petgraph` DAG of `WorkUnit`s.
//!
//! `CompiledPlan` is the output of `gaviero_dsl::compile()` and the input to
//! `swarm::pipeline::execute()`. It replaces the previous `Vec<WorkUnit>`
//! handoff, adding dependency edges, trigger rules, and checkpoint metadata.
//!
//! ## Type placement
//! Defined here (in `gaviero-core`) rather than in `gaviero-dsl` so that
//! `pipeline.rs` can reference it without introducing a circular dependency.
//! `gaviero-dsl` already depends on `gaviero-core` and returns this type.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use serde::{Deserialize, Serialize};

use super::models::WorkUnit;
use crate::iteration::IterationConfig;

// ── VerificationConfig ───────────────────────────────────────

/// Controls which post-edit verification checks are run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VerificationConfig {
    pub compile: bool,
    pub clippy: bool,
    pub test: bool,
}

// ── Graph types ──────────────────────────────────────────────

/// A single node in the execution plan DAG.
#[derive(Debug, Clone)]
pub struct PlanNode {
    pub work_unit: WorkUnit,
}

/// The kind of dependency between two `PlanNode`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DependencyEdge {
    /// Output of `from` feeds input of `to` (data dependency).
    Data,
    /// `from` must complete before `to` starts (ordering constraint only).
    Ordering,
}

/// When should a node be triggered relative to its dependencies?
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum TriggerRule {
    /// Start only when all dependencies succeeded. **Default.**
    #[default]
    AllSuccess,
    /// Start when all dependencies are terminal (including failures).
    AllDone,
    /// Start when at least one dependency failed (error handler pattern).
    OneFailed,
}

// ── CompiledPlan ─────────────────────────────────────────────

/// Immutable execution plan produced by the DSL compiler.
///
/// The `graph` field is a `DiGraph` where edges go from dependency to
/// dependent (i.e. if B depends on A, there is an edge A → B).
#[derive(Debug)]
pub struct CompiledPlan {
    /// DAG of work units and their dependencies.
    pub graph: DiGraph<PlanNode, DependencyEdge>,
    /// Optional `max_parallel` declared in the workflow block.
    pub max_parallel: Option<usize>,
    /// The source `.gaviero` file (for checkpoint naming and `--resume`).
    pub source_file: Option<PathBuf>,
    /// Iteration strategy configuration declared in the workflow block.
    pub iteration_config: IterationConfig,
    /// Post-edit verification checks declared in the workflow block.
    pub verification_config: VerificationConfig,
}

impl CompiledPlan {
    /// Extract all work units in topological order (dependencies first).
    ///
    /// Uses Kahn's algorithm with a `BTreeSet` ready-queue so that nodes at
    /// the same topological level are emitted in insertion (NodeIndex) order.
    /// This preserves declaration order for independent agents, which matters
    /// for test determinism and predictable CLI output.
    ///
    /// Returns `Err` if the graph contains a cycle (should have been caught
    /// by the DSL compiler, but checked here defensively).
    pub fn work_units_ordered(&self) -> anyhow::Result<Vec<WorkUnit>> {
        let mut in_degree: HashMap<NodeIndex, usize> = self
            .graph
            .node_indices()
            .map(|idx| {
                let deg = self
                    .graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .count();
                (idx, deg)
            })
            .collect();

        // BTreeSet gives us stable ordering by NodeIndex within each ready tier.
        let mut ready: BTreeSet<NodeIndex> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&idx, _)| idx)
            .collect();

        let mut result = Vec::with_capacity(self.graph.node_count());
        while let Some(idx) = ready.iter().next().copied() {
            ready.remove(&idx);
            result.push(self.graph[idx].work_unit.clone());
            for neighbor in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                let deg = in_degree.get_mut(&neighbor).expect("node in graph");
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(neighbor);
                }
            }
        }

        if result.len() == self.graph.node_count() {
            Ok(result)
        } else {
            Err(anyhow::anyhow!("dependency cycle in plan graph"))
        }
    }

    /// Return all work units in insertion order (no topological guarantee).
    pub fn work_units_unordered(&self) -> Vec<&WorkUnit> {
        self.graph.node_weights().map(|n| &n.work_unit).collect()
    }

    /// Find the `NodeIndex` for a work unit by its id.
    pub fn node_for_id(&self, id: &str) -> Option<NodeIndex> {
        self.graph
            .node_indices()
            .find(|&idx| self.graph[idx].work_unit.id == id)
    }

    /// Build a trivial `CompiledPlan` from a flat `Vec<WorkUnit>`.
    ///
    /// Dependencies declared in `work_unit.depends_on` become `Ordering`
    /// edges. Use this when constructing a plan programmatically (e.g. from
    /// `--task` or `--work-units` CLI flags).
    pub fn from_work_units(units: Vec<WorkUnit>, max_parallel: Option<usize>) -> Self {
        let mut graph: DiGraph<PlanNode, DependencyEdge> = DiGraph::new();
        let mut id_to_idx: std::collections::HashMap<String, NodeIndex> = Default::default();

        // First pass: add all nodes
        for unit in &units {
            let idx = graph.add_node(PlanNode { work_unit: unit.clone() });
            id_to_idx.insert(unit.id.clone(), idx);
        }

        // Second pass: add dependency edges
        for unit in &units {
            let to = id_to_idx[&unit.id];
            for dep_id in &unit.depends_on {
                if let Some(&from) = id_to_idx.get(dep_id) {
                    graph.add_edge(from, to, DependencyEdge::Ordering);
                }
            }
        }

        Self {
            graph,
            max_parallel,
            source_file: None,
            iteration_config: IterationConfig::default(),
            verification_config: VerificationConfig::default(),
        }
    }

    /// A stable hash over the plan's work unit IDs and descriptions.
    ///
    /// Used to name checkpoint files so that a resumed run uses the same
    /// checkpoint as the original.
    pub fn hash(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        // Sort by id for stability regardless of graph traversal order
        let mut ids: Vec<(&str, &str)> = self
            .graph
            .node_weights()
            .map(|n| (n.work_unit.id.as_str(), n.work_unit.description.as_str()))
            .collect();
        ids.sort_unstable();
        for (id, desc) in ids {
            id.hash(&mut hasher);
            desc.hash(&mut hasher);
        }
        format!("{:016x}", hasher.finish())
    }
}
