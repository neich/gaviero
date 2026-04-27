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

use super::scope::{WriteMeta, WriteScope};
use super::store::MemoryStore;
use super::stores::MemoryStores;
use super::writer::{WriteResult, WriterConfig, WriterHandle, spawn_writer_task};

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
///
/// Step 7: holds the multi-DB registry. Reads against run rows go to
/// `stores.workspace()`; maintenance passes (decay/prune, promotion
/// analysis) fan out across every opened store via
/// [`MemoryStores::opened_stores`] / [`MemoryStores::opened_folder_stores`].
pub struct Consolidator {
    stores: Arc<MemoryStores>,
    writer: WriterHandle,
}

impl Consolidator {
    /// Legacy constructor: wraps a single [`MemoryStore`] in a
    /// single-store-fallback [`MemoryStores`]. Kept for backward
    /// compatibility with call sites that still hold an `Arc<MemoryStore>`
    /// directly. New call sites should use [`Self::with_stores`].
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self::with_stores(MemoryStores::from_single_store(memory))
    }

    /// Step 7: registry-aware constructor. The consolidator's writer
    /// dispatches per-scope and the maintenance passes fan out across
    /// every opened DB.
    pub fn with_stores(stores: Arc<MemoryStores>) -> Self {
        let writer = spawn_writer_task(WriterConfig {
            stores: stores.clone(),
            llm: None,
            observer: None,
            manifest_observer: None,
        });
        Self { stores, writer }
    }

    pub fn new_with_writer(memory: Arc<MemoryStore>, writer: WriterHandle) -> Self {
        Self {
            stores: MemoryStores::from_single_store(memory),
            writer,
        }
    }

    /// Registry + custom writer. Used by call sites (TUI) that already
    /// own a writer handle wired to the same registry.
    pub fn with_stores_and_writer(stores: Arc<MemoryStores>, writer: WriterHandle) -> Self {
        Self { stores, writer }
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

        // Run rows live in the workspace store (StoreKind::Workspace
        // routing for WriteScope::Run). Promotions land in the folder
        // DB via the writer, which dispatches by target_store().
        let run_memories = self.stores.workspace().query_by_run(run_id).await?;
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

            let meta = WriteMeta::for_source(super::trust_defaults::MemorySource::LlmConsolidated)
                .with_importance(mem.importance)
                .with_type(mem.memory_type)
                .with_tag(
                    mem.tag
                        .clone()
                        .unwrap_or_else(|| format!("consolidation:run:{run_id}")),
                );

            match self
                .writer
                .swarm_consolidate_wait(target, mem.content.clone(), meta)
                .await
            {
                Ok(WriteResult::Inserted(_)) => {
                    report.promoted += 1;
                }
                Ok(WriteResult::Deduplicated(_)) => {
                    report.reinforced += 1;
                }
                Ok(WriteResult::AlreadyCovered) => {
                    // Already exists at broader scope — skip
                }
                Ok(WriteResult::Skipped) => {}
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
        match self.writer.delete_run(run_id).await? {
            WriteResult::Inserted(deleted) => {
                report.pruned = deleted as usize;
            }
            WriteResult::Skipped => {}
            _ => {}
        }

        tracing::info!(
            run_id,
            promoted = report.promoted,
            reinforced = report.reinforced,
            pruned = report.pruned,
            "consolidation: run triage complete"
        );

        Ok(report)
    }

    /// Phase 2: Decay importance and prune stale entries. Step 7:
    /// fans out across every opened physical store so per-folder DBs
    /// also age out their own rows.
    pub async fn decay_and_prune(&self) -> Result<(usize, usize)> {
        // Materialise every registered folder store first so the
        // decay pass touches all of them, not just whatever was
        // lazy-opened by retrieval traffic.
        let _ = self.stores.open_all_folders().await;
        let mut decayed_total = 0usize;
        let mut pruned_total = 0usize;
        for store in self.stores.opened_stores().await {
            let (decayed, pruned) = store.decay_and_prune().await?;
            decayed_total += decayed;
            pruned_total += pruned;
        }
        Ok((decayed_total, pruned_total))
    }

    /// Phase 3: Promote module memories accessed across multiple modules.
    ///
    /// If a module-level memory is accessed by agents in 3+ different modules,
    /// promote a copy to repo scope with boosted importance.
    ///
    /// Step 7: scans every opened folder store independently. Module →
    /// repo promotion is intra-folder (both scopes live in the same
    /// folder DB), so per-store iteration is the natural shape.
    pub async fn promote_frequent_cross_scope(&self, min_cross_hits: i64) -> Result<usize> {
        let _ = self.stores.open_all_folders().await;
        let mut candidates = Vec::new();
        for store in self.stores.opened_folder_stores().await {
            candidates.extend(store.find_promotion_candidates(min_cross_hits).await?);
        }
        let mut promoted = 0;

        for mem in &candidates {
            let Some(repo_id) = &mem.repo_id else {
                continue;
            };

            // Preserve the source promoted memories were authored under;
            // promotion only boosts importance, not trust or origin.
            let meta = WriteMeta::for_source(mem.source)
                .with_trust_score(mem.trust_score)
                .with_importance((mem.importance * 1.2).min(1.0))
                .with_type(mem.memory_type)
                .with_tag(format!("promotion:module_to_repo:{}", mem.id));

            match self
                .writer
                .swarm_consolidate_wait(
                    WriteScope::Repo {
                        repo_id: repo_id.clone(),
                    },
                    mem.content.clone(),
                    meta,
                )
                .await
            {
                Ok(WriteResult::Inserted(_)) => {
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
        // Legacy namespace-keyed dedup runs against the workspace store
        // (the only store that holds legacy namespace rows today).
        let results = self
            .stores
            .workspace()
            .search(namespace, content, 1)
            .await?;
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
    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        fn name(&self) -> &str {
            "mock"
        }

        fn dimension(&self) -> usize {
            8
        }

        async fn embed(
            &self,
            text: &str,
            _purpose: crate::memory::embedder::EmbeddingPurpose,
        ) -> Result<Vec<f32>> {
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
