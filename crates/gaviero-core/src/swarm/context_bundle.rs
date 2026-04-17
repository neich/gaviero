//! Swarm context bundle (M7 of PROVIDER_PLAN_V9).
//!
//! The pipeline builds a [`SwarmContextBundle`] once per swarm run.
//! It distributes shared memory and per-unit graph slices to each runner,
//! reducing memory-store queries from N+1 to ≤2 (coordinator + one shared
//! bundle query; runners consume pre-fetched data instead of querying).
//!
//! **Swarm isolation invariant:** `shared_memory` is read-only, derived from
//! a single query.  Each runner receives an immutable reference via
//! [`SwarmContextBundle::memory_text_for_prompt`] — no mutable shared state
//! is distributed across work units.

use std::collections::HashMap;

use crate::context_planner::types::MemoryCandidate;
use crate::memory::MemoryStore;
use crate::repo_map::store::ImpactSummary;

/// Swarm-wide context bundle built once by the pipeline before running any
/// work units (V9 §7 M7).
///
/// The coordinator already issues one memory query.  This bundle issues a
/// second query using the architectural intent (concatenated work-unit
/// descriptions) so every runner gets the same pre-fetched candidates and
/// issues zero additional DB operations.  Total DB round-trips for a 3-unit
/// swarm: coordinator(1) + bundle(1) = 2 ≤ M7 acceptance gate.
pub struct SwarmContextBundle {
    /// The query string used to fetch `shared_memory`.
    pub architectural_intent: String,
    /// Memory candidates shared across all work units (from one DB query).
    pub shared_memory: Vec<MemoryCandidate>,
    /// Per-unit graph impact summaries with typed `ImpactSummary`.
    ///
    /// Keyed by `work_unit_id`.  Built alongside `impact_texts` in the
    /// pipeline so the data is structured, not just a pre-rendered string.
    pub per_unit_graph: HashMap<String, GraphSlice>,
}

/// Per-unit graph context slice passed from the pipeline to each runner.
///
/// Carries the typed [`ImpactSummary`] so downstream consumers have
/// structured data rather than a pre-rendered prompt string.
pub struct GraphSlice {
    /// Typed blast-radius result from the code knowledge graph.
    pub impact: Option<ImpactSummary>,
}

impl SwarmContextBundle {
    /// Render `shared_memory` into the legacy `[Memory context]:` block
    /// format.  Returns `None` when there are no candidates.
    ///
    /// Called once per work unit to produce the `pre_fetched_memory_context`
    /// string for the planner.  Rendering happens here (pipeline side) so
    /// each runner's planner short-circuits its DB query.
    pub fn memory_text_for_prompt(&self) -> Option<String> {
        if self.shared_memory.is_empty() {
            return None;
        }
        let mut block = String::from("[Memory context]:\n");
        for m in &self.shared_memory {
            block.push_str(&format!(
                "- [{}] {} (score: {:.2})\n",
                m.namespace, m.content, m.score
            ));
        }
        Some(block)
    }
}

/// Build a [`SwarmContextBundle`] with a single shared memory query.
///
/// `architectural_intent` is used as the query string — typically the
/// concatenation of all work-unit descriptions for the current swarm run.
/// `read_namespaces` and `memory_limit` mirror the per-unit runner settings.
///
/// Returns an empty bundle immediately when `memory` is `None` or
/// `read_namespaces` is empty (no DB access occurs in those cases).
pub async fn build_bundle(
    architectural_intent: &str,
    memory: Option<&MemoryStore>,
    read_namespaces: &[String],
    memory_limit: usize,
) -> SwarmContextBundle {
    let shared_memory = if let (Some(mem), false) = (memory, read_namespaces.is_empty()) {
        tracing::info!(
            target: "turn_metrics",
            kind = "swarm_bundle",
            namespaces = ?read_namespaces,
            limit = memory_limit,
            "bundle_memory_query"
        );
        mem.search_candidates(read_namespaces, architectural_intent, memory_limit)
            .await
    } else {
        Vec::new()
    };

    SwarmContextBundle {
        architectural_intent: architectural_intent.to_string(),
        shared_memory,
        per_unit_graph: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(ns: &str, content: &str, score: f32) -> MemoryCandidate {
        MemoryCandidate {
            id: 1,
            namespace: ns.to_string(),
            scope_label: ns.to_string(),
            score,
            trust: None,
            content: content.to_string(),
            source_hash: None,
            updated_at: None,
        }
    }

    #[test]
    fn memory_text_empty_when_no_candidates() {
        let bundle = SwarmContextBundle {
            architectural_intent: "test".to_string(),
            shared_memory: Vec::new(),
            per_unit_graph: HashMap::new(),
        };
        assert!(bundle.memory_text_for_prompt().is_none());
    }

    #[test]
    fn memory_text_formats_candidates() {
        let bundle = SwarmContextBundle {
            architectural_intent: "test".to_string(),
            shared_memory: vec![make_candidate("ws", "use anyhow", 0.85)],
            per_unit_graph: HashMap::new(),
        };
        let text = bundle.memory_text_for_prompt().unwrap();
        assert!(text.starts_with("[Memory context]:\n"));
        assert!(text.contains("[ws]"));
        assert!(text.contains("use anyhow"));
        assert!(text.contains("0.85"));
    }

    #[test]
    fn graph_slice_holds_typed_impact() {
        let impact = ImpactSummary {
            changed_files: vec!["src/lib.rs".to_string()],
            affected_files: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            affected_tests: vec![],
            test_gaps: vec![],
            truncated: false,
        };
        let slice = GraphSlice {
            impact: Some(impact),
        };
        let imp = slice.impact.unwrap();
        assert_eq!(imp.affected_files.len(), 2);
        assert!(!imp.truncated);
    }

    #[tokio::test]
    async fn build_bundle_empty_when_no_memory() {
        let bundle = build_bundle("do something", None, &["ws".to_string()], 5).await;
        assert!(bundle.shared_memory.is_empty());
        assert_eq!(bundle.architectural_intent, "do something");
    }

    #[tokio::test]
    async fn build_bundle_empty_when_no_namespaces() {
        // memory = None is the only option in unit tests; verify the
        // namespace-empty guard also short-circuits (no panic).
        let bundle = build_bundle("do something", None, &[], 5).await;
        assert!(bundle.shared_memory.is_empty());
    }
}
