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
use std::path::Path;

use crate::context_planner::types::MemoryCandidate;
use crate::memory::{
    CONSTITUTION_ARCHIVE_HINT, ChatInjectionConfig, MemoryScope, MemoryStores, RetrievalConfig,
    admit_constitution, retrieve_ranked,
};
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
    /// When true, [`Self::memory_text_for_prompt`] appends the constitution
    /// archive hint so swarm agents know to pull lessons via MCP.
    pub constitution_only: bool,
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
    /// Render `shared_memory` into the caveman `Mem:` block. Returns `None`
    /// when there are no candidates. Called once per work unit to produce
    /// `pre_fetched_memory_context` for the planner; rendering happens here
    /// so each runner's planner short-circuits its DB query.
    pub fn memory_text_for_prompt(&self) -> Option<String> {
        if self.shared_memory.is_empty() {
            return None;
        }
        let mut block = String::from("Mem:\n");
        if self.constitution_only {
            block.push_str(CONSTITUTION_ARCHIVE_HINT);
        }
        for m in &self.shared_memory {
            block.push_str(&format!("{}|{}|s{:.2}\n", m.namespace, m.content, m.score));
        }
        Some(block)
    }
}

/// Build a [`SwarmContextBundle`] with a single shared memory query.
///
/// Goes through the central [`retrieve_ranked`] entry point (B3) so the
/// swarm's shared-memory query benefits from the same merged-multi-scope
/// retrieval, scope/trust scoring, and B2 rerank as chat injection /
/// MCP / panel search. `read_namespaces` is preserved as a noop hint
/// for tracing — Tier B retrieval is scope-based, not namespace-based.
///
/// Returns an empty bundle immediately when `memory` is `None` or the
/// architectural intent is empty (no DB access occurs in those cases).
pub async fn build_bundle(
    architectural_intent: &str,
    memory: Option<&std::sync::Arc<MemoryStores>>,
    workspace_root: &Path,
    read_namespaces: &[String],
    memory_limit: usize,
    injection: &ChatInjectionConfig,
) -> SwarmContextBundle {
    let shared_memory: Vec<MemoryCandidate> =
        if let Some(mem) = memory.filter(|_| !architectural_intent.trim().is_empty()) {
            tracing::info!(
                target: "turn_metrics",
                kind = "swarm_bundle",
                namespaces = ?read_namespaces,
                limit = memory_limit,
                "bundle_memory_query"
            );
            // Bundle is shared across all units in the swarm — no
            // single owning folder. Pass folder = None so the registry
            // walks workspace + global only (matches the bundle's
            // intent: cross-cutting workspace knowledge, not
            // folder-specific).
            let scope = MemoryScope::from_context(workspace_root, None, None, None);
            let engine_limit = (memory_limit * 4).max(40);
            match retrieve_ranked(
                mem,
                &scope,
                architectural_intent,
                engine_limit,
                &RetrievalConfig::default(),
                None,
                None,
            )
            .await
            {
                Ok(out) => out
                    .items
                    .iter()
                    .filter(|m| admit_constitution(m, injection))
                    .take(memory_limit)
                    .map(MemoryCandidate::from_scored)
                    .collect(),
                Err(e) => {
                    tracing::warn!("swarm bundle retrieval failed: {e}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

    SwarmContextBundle {
        architectural_intent: architectural_intent.to_string(),
        shared_memory,
        per_unit_graph: HashMap::new(),
        constitution_only: injection.constitution_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{
        MemoryServices, MemoryType, WriteMeta, WriteScope, trust_defaults::MemorySource,
    };

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
            constitution_only: true,
        };
        assert!(bundle.memory_text_for_prompt().is_none());
    }

    #[test]
    fn memory_text_formats_candidates() {
        let bundle = SwarmContextBundle {
            architectural_intent: "test".to_string(),
            shared_memory: vec![make_candidate("ws", "use anyhow", 0.85)],
            per_unit_graph: HashMap::new(),
            constitution_only: true,
        };
        let text = bundle.memory_text_for_prompt().unwrap();
        assert!(text.starts_with("Mem:\n"));
        assert!(text.contains(CONSTITUTION_ARCHIVE_HINT.trim_end()));
        assert!(text.contains("ws|"));
        assert!(text.contains("use anyhow"));
        assert!(text.contains("s0.85"));
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
        let root = std::path::PathBuf::from("/tmp");
        let bundle = build_bundle(
            "do something",
            None,
            &root,
            &["ws".to_string()],
            5,
            &ChatInjectionConfig::default(),
        )
        .await;
        assert!(bundle.shared_memory.is_empty());
        assert_eq!(bundle.architectural_intent, "do something");
    }

    #[tokio::test]
    async fn build_bundle_empty_when_no_namespaces() {
        // memory = None is the only option in unit tests; verify the
        // namespace-empty guard also short-circuits (no panic).
        let root = std::path::PathBuf::from("/tmp");
        let bundle = build_bundle(
            "do something",
            None,
            &root,
            &[],
            5,
            &ChatInjectionConfig::default(),
        )
        .await;
        assert!(bundle.shared_memory.is_empty());
    }

    #[tokio::test]
    async fn build_bundle_serializes_only_constitution_rows() {
        let services = MemoryServices::for_tests_in_memory().unwrap();
        let root = std::path::PathBuf::from("/tmp/ws-bundle-constitution");
        let scope = WriteScope::Workspace;
        services
            .stores
            .store_scoped(
                &scope,
                "extracted hashing factual for archive only",
                &WriteMeta::for_source(MemorySource::LlmExtracted).with_type(MemoryType::Factual),
            )
            .await
            .unwrap();
        services
            .stores
            .store_scoped(
                &scope,
                "decision: hashing uses sha256 in this repo",
                &WriteMeta::for_source(MemorySource::LlmAnnotated).with_type(MemoryType::Decision),
            )
            .await
            .unwrap();

        let bundle = build_bundle(
            "hashing sha256",
            Some(&services.stores),
            &root,
            &["ws".to_string()],
            5,
            &ChatInjectionConfig::default(),
        )
        .await;
        assert!(
            bundle
                .shared_memory
                .iter()
                .all(|m| !m.content.contains("archive only")),
            "extracted factuals must not enter the swarm bundle"
        );
        assert!(
            bundle
                .shared_memory
                .iter()
                .any(|m| m.content.contains("sha256")),
            "Decision rows must serialize"
        );
        let text = bundle.memory_text_for_prompt().unwrap();
        assert!(text.contains(CONSTITUTION_ARCHIVE_HINT.trim_end()));
    }
}
