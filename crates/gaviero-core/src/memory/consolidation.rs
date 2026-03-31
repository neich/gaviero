//! Memory consolidation: dedup, merge, and episodic summarization.
//!
//! On each new memory write, embedding similarity is checked against existing entries:
//! - Similarity > 0.85: reinforce existing entry (bump access_count + timestamp)
//! - Similarity 0.7-0.85: flag for LLM merge (background)
//! - Similarity < 0.7: normal insert
//!
//! Periodic sweep merges near-duplicates and prunes low-strength entries.
//! LLM-based merge uses Claude subprocess for content fusion.

use std::sync::Arc;

use anyhow::Result;

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

/// Result of a consolidation sweep.
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub reinforced: usize,
    pub merged: usize,
    pub pruned: usize,
}

/// Handles memory deduplication, merging, and episodic summarization.
pub struct Consolidator {
    memory: Arc<MemoryStore>,
}

impl Consolidator {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
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
        // Compute embedding for the new content
        let results = self.memory.search(namespace, content, 1).await?;

        if let Some(top) = results.first() {
            // Cosine similarity from composite score isn't directly available,
            // but we can use the raw search for dedup checking.
            // For now, use a simplified approach: if the top result is very similar,
            // reinforce it instead of inserting.
            if top.score > 2.5 {
                // High composite score suggests high relevance — reinforce
                // (The actual similarity threshold logic would need raw distance,
                // which requires a separate query. This is a simplified version.)
                return Ok(Some(top.entry.id));
            }
        }

        Ok(None)
    }

    /// Run a consolidation sweep over a namespace.
    ///
    /// - Finds near-duplicate entries (similarity 0.7-0.85)
    /// - Merges them using LLM if available, otherwise keeps both
    /// - Prunes entries with retrieval score below threshold
    pub async fn sweep(&self, _namespace: &str) -> Result<ConsolidationReport> {
        let mut report = ConsolidationReport::default();

        // TODO: Implement full sweep with pairwise similarity comparison
        // For now, this is a no-op placeholder that will be filled in when
        // LLM integration is added.

        // The sweep algorithm:
        // 1. Load all entries in namespace
        // 2. For each pair with similarity 0.7-0.85, merge via LLM
        // 3. Delete originals, insert merged
        // 4. Prune entries with composite score < 0.1

        let _ = &mut report;
        Ok(report)
    }

    /// Summarize old episodic sequences via LLM.
    ///
    /// Finds sequences of episodic entries (same task_id) older than max_age_days,
    /// summarizes each into a single entry, and deletes the originals.
    pub async fn summarize_episodes(
        &self,
        _namespace: &str,
        _max_age_days: u32,
    ) -> Result<usize> {
        // TODO: Implement when episode storage is wired into the swarm pipeline.
        Ok(0)
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
    // For now, concatenate with separator
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
            if norm > 0.0 { for v in &mut vec { *v /= norm; } }
            Ok(vec)
        }
        fn dimensions(&self) -> usize { 8 }
        fn model_id(&self) -> &str { "mock" }
    }

    #[tokio::test]
    async fn test_consolidator_sweep_no_crash() {
        let store = Arc::new(
            MemoryStore::in_memory(Arc::new(MockEmbedder)).unwrap()
        );
        let consolidator = Consolidator::new(store);
        let report = consolidator.sweep("ns").await.unwrap();
        assert_eq!(report.merged, 0);
    }
}
