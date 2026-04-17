//! Memory consolidation: run triage, importance decay, and upward promotion.
//!
//! Three phases run after each swarm execution:
//!
//! 1. **Run memory triage**: Scan run-level memories, promote durable facts
//!    to module or repo scope, then delete the ephemeral run memories.
//!
//! 2. **Importance decay and pruning**: Exponentially decay importance
//!    based on time since last access. Prune entries below threshold.
//!
//! 3. **Upward promotion**: Module memories accessed across multiple modules
//!    get promoted to repo scope.

use std::sync::Arc;

use anyhow::Result;
use tracing;

use super::scope::{Trust, WriteMeta, WriteScope};
use super::store::MemoryStore;

/// Policy for consolidation during store operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationPolicy {
    /// Automatic dedup and merge flagging on store.
    Auto,
    /// No consolidation — always insert.
    None,
}

impl Default for ConsolidationPolicy {
    fn default() -> Self {
        Self::Auto
    }
}

/// Result of a consolidation run.
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub reinforced: usize,
    pub promoted: usize,
    pub pruned: usize,
    pub decayed: usize,
}

/// Full maintenance report combining all phases.
#[derive(Debug, Default)]
pub struct MaintenanceReport {
    pub consolidation: Option<ConsolidationReport>,
    pub decay_count: usize,
    pub prune_count: usize,
    pub promotion_count: usize,
}

/// Handles memory consolidation, decay, and promotion.
pub struct Consolidator {
    memory: Arc<MemoryStore>,
}

impl Consolidator {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }

    /// Phase 1: Triage run memories after a swarm execution.
    ///
    /// Scans all run-level memories for the completed run. For each memory,
    /// decides whether to promote to module/repo scope or discard.
    ///
    /// Without LLM: promotes all run memories with importance >= 0.4 to
    /// module scope (if module_path is set) or repo scope.
    ///
    /// After promotion, deletes all run-level memories.
    pub async fn consolidate_run(
        &self,
        run_id: &str,
        repo_id: &str,
    ) -> Result<ConsolidationReport> {
        let mut report = ConsolidationReport::default();

        let run_memories = self.memory.query_by_run(run_id).await?;
        if run_memories.is_empty() {
            tracing::debug!(run_id, "consolidation: no run memories to triage");
            return Ok(report);
        }

        tracing::info!(
            run_id,
            count = run_memories.len(),
            "consolidation: triaging run memories"
        );

        for mem in &run_memories {
            // Only promote memories with meaningful importance
            if mem.importance < 0.4 {
                continue;
            }

            // Determine target scope: module if we have one, otherwise repo
            let target = if let Some(module_path) = &mem.module_path {
                WriteScope::Module {
                    repo_id: repo_id.to_string(),
                    module_path: module_path.clone(),
                }
            } else {
                WriteScope::Repo {
                    repo_id: repo_id.to_string(),
                }
            };

            let meta = WriteMeta {
                memory_type: mem.memory_type,
                importance: mem.importance,
                trust: Trust::Low, // consolidated from run = inferred
                source: format!("consolidation:run:{run_id}"),
                tag: mem.tag.clone(),
            };

            match self.memory.store_scoped(&target, &mem.content, &meta).await {
                Ok(super::scope::StoreResult::Inserted(_)) => {
                    report.promoted += 1;
                }
                Ok(super::scope::StoreResult::Deduplicated(_)) => {
                    report.reinforced += 1;
                }
                Ok(super::scope::StoreResult::AlreadyCovered) => {
                    // Already exists at broader scope — skip
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        memory_id = mem.id,
                        "consolidation: failed to promote run memory"
                    );
                }
            }
        }

        // Delete all run-level memories
        let deleted = self.memory.delete_by_run(run_id).await?;
        report.pruned = deleted as usize;

        tracing::info!(
            run_id,
            promoted = report.promoted,
            reinforced = report.reinforced,
            pruned = report.pruned,
            "consolidation: run triage complete"
        );

        Ok(report)
    }

    /// Phase 2: Decay importance and prune stale entries.
    pub async fn decay_and_prune(&self) -> Result<(usize, usize)> {
        self.memory.decay_and_prune().await
    }

    /// Phase 3: Promote module memories accessed across multiple modules.
    ///
    /// If a module-level memory is accessed by agents in 3+ different modules,
    /// promote a copy to repo scope with boosted importance.
    pub async fn promote_frequent_cross_scope(&self, min_cross_hits: i64) -> Result<usize> {
        let candidates = self
            .memory
            .find_promotion_candidates(min_cross_hits)
            .await?;
        let mut promoted = 0;

        for mem in &candidates {
            let Some(repo_id) = &mem.repo_id else {
                continue;
            };

            let meta = WriteMeta {
                memory_type: mem.memory_type,
                importance: (mem.importance * 1.2).min(1.0), // boost
                trust: mem.trust,
                source: format!("promotion:module_to_repo:{}", mem.id),
                tag: mem.tag.clone(),
            };

            match self
                .memory
                .store_scoped(
                    &WriteScope::Repo {
                        repo_id: repo_id.clone(),
                    },
                    &mem.content,
                    &meta,
                )
                .await
            {
                Ok(super::scope::StoreResult::Inserted(_)) => {
                    promoted += 1;
                    tracing::debug!(memory_id = mem.id, "promotion: module → repo");
                }
                Ok(_) => {} // deduplicated or already covered
                Err(e) => {
                    tracing::warn!(error = %e, "promotion: failed");
                }
            }
        }

        Ok(promoted)
    }

    /// Run full maintenance: consolidation + decay + promotion.
    pub async fn maintain(&self) -> Result<MaintenanceReport> {
        let mut report = MaintenanceReport::default();

        // Phase 2: decay and prune
        let (decayed, pruned) = self.decay_and_prune().await?;
        report.decay_count = decayed;
        report.prune_count = pruned;

        // Phase 3: promote cross-module memories
        let promoted = self.promote_frequent_cross_scope(3).await?;
        report.promotion_count = promoted;

        tracing::info!(
            decayed = report.decay_count,
            pruned = report.prune_count,
            promoted = report.promotion_count,
            "maintenance complete"
        );

        Ok(report)
    }

    /// Check if a new entry should be deduplicated against existing entries.
    ///
    /// Returns `Some(existing_id)` if the entry was reinforced (similarity > 0.85),
    /// or `None` if the entry should be inserted normally.
    pub async fn check_dedup(
        &self,
        namespace: &str,
        content: &str,
        _limit: usize,
    ) -> Result<Option<i64>> {
        let results = self.memory.search(namespace, content, 1).await?;
        if let Some(top) = results.first() {
            if top.score > 2.5 {
                return Ok(Some(top.entry.id));
            }
        }
        Ok(None)
    }
}

/// Helper to merge two memory entries' content using LLM.
///
/// Falls back to concatenation if LLM is unavailable.
pub async fn merge_content(
    content_a: &str,
    content_b: &str,
    _llm_available: bool,
) -> Result<String> {
    // TODO: Implement LLM-based merge via AcpSession
    Ok(format!("{content_a}\n---\n{content_b}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::embedder::Embedder;

    struct MockEmbedder;
    impl Embedder for MockEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let mut vec = vec![0.0f32; 8];
            for (i, byte) in text.bytes().enumerate() {
                vec[i % 8] += byte as f32;
            }
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }
        fn dimensions(&self) -> usize {
            8
        }
        fn model_id(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_consolidate_run_empty() {
        let store = Arc::new(MemoryStore::in_memory(Arc::new(MockEmbedder)).unwrap());
        let consolidator = Consolidator::new(store);
        let report = consolidator
            .consolidate_run("nonexistent", "repo1")
            .await
            .unwrap();
        assert_eq!(report.promoted, 0);
        assert_eq!(report.pruned, 0);
    }

    #[tokio::test]
    async fn test_maintain_no_crash() {
        let store = Arc::new(MemoryStore::in_memory(Arc::new(MockEmbedder)).unwrap());
        let consolidator = Consolidator::new(store);
        let report = consolidator.maintain().await.unwrap();
        assert_eq!(report.prune_count, 0);
    }
}
