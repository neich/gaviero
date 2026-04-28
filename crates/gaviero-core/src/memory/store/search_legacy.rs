//! Legacy namespace-based search surface on `MemoryStore`.
//!
//! Carved out of `store/mod.rs` (Phase 4.7a). Houses the pre-scoped-
//! retrieval API: namespace-keyed `search`, `search_multi`,
//! `search_candidates`, `search_context`, `search_context_filtered`,
//! plus the private `search_multi_filtered` they all delegate to.
//!
//! The scoped retrieval pipeline (`search_scoped`,
//! `multi_scope_retrieve`, etc.) lives in `store/search.rs`. Most
//! production callers should prefer the scoped path; this module
//! stays around because the swarm context bundle and a handful of
//! pre-A2 paths still call `search_context` / `search_multi`.

use anyhow::{Context, Result};

use super::{
    MemoryCandidate, MemoryEntry, MemoryStore, PrivacyFilter, SearchResult, chrono_now_utc,
    embedding_to_blob, hours_since, retrieval_score,
};

impl MemoryStore {
    /// Search for memories similar to the query text within a single namespace.
    ///
    /// Uses sqlite-vec KNN search with composite scoring (recency + importance + relevance).
    pub async fn search(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_multi_filtered(
            &[namespace.to_string()],
            query,
            limit,
            PrivacyFilter::IncludeAll,
        )
        .await
    }

    /// Search across multiple namespaces.
    ///
    /// Results from all namespaces are merged and sorted by composite score.
    pub async fn search_multi(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_multi_filtered(namespaces, query, limit, PrivacyFilter::IncludeAll)
            .await
    }

    /// V9 §11 M3: structured memory candidates for the planner.
    ///
    /// Returns an empty `Vec` on error or no hits. Unlike [`Self::search_context`]
    /// (which formats results as a prompt-ready string), this returns the
    /// underlying records so the planner can record memory IDs in the ledger
    /// and emit per-selection tracing. M10 deletes `search_context`.
    pub async fn search_candidates(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> Vec<MemoryCandidate> {
        match self.search_multi(namespaces, query, limit).await {
            Ok(results) => {
                // M0 legacy aggregate event — kept for baseline tooling
                // continuity. M3's per-candidate `memory_candidate` events
                // are emitted by the planner (`ContextPlanner::collect_memory`).
                let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
                let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
                tracing::info!(
                    target: "turn_metrics",
                    memory_ids = ?ids,
                    memory_scores = ?scores,
                    memory_count = results.len(),
                    "memory_selection"
                );
                results.iter().map(MemoryCandidate::from).collect()
            }
            Err(e) => {
                tracing::warn!("memory search failed: {}", e);
                tracing::info!(
                    target: "turn_metrics",
                    memory_count = 0usize,
                    "memory_selection"
                );
                Vec::new()
            }
        }
    }

    /// Search across namespaces and format results as a prompt-ready string.
    ///
    /// Returns an empty string on error or if no results are found.
    pub async fn search_context(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> String {
        match self.search_multi(namespaces, query, limit).await {
            Ok(results) if !results.is_empty() => {
                // M0 instrumentation: expose selected memory IDs + scores so
                // baselines can measure repeated-context waste across turns.
                // Safe under V9 §2 "structured APIs land in M3" — no API change.
                let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
                let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
                tracing::info!(
                    target: "turn_metrics",
                    memory_ids = ?ids,
                    memory_scores = ?scores,
                    memory_count = results.len(),
                    "memory_selection"
                );
                let mut ctx = String::from("Mem:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "{}|{}|s{:.2}\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => {
                tracing::info!(
                    target: "turn_metrics",
                    memory_count = 0usize,
                    "memory_selection"
                );
                String::new()
            }
        }
    }

    /// Search across namespaces with privacy filtering, returning a prompt-ready string.
    pub async fn search_context_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> String {
        match self
            .search_multi_filtered(namespaces, query, limit, filter)
            .await
        {
            Ok(results) if !results.is_empty() => {
                let mut ctx = String::from("Mem:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "{}|{}|s{:.2}\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => String::new(),
        }
    }

    /// Core search implementation using sqlite-vec KNN with post-filtering and composite scoring.
    ///
    /// sqlite-vec doesn't support WHERE clauses in KNN queries, so we over-fetch
    /// and filter by namespace/privacy in Rust.
    pub(crate) async fn search_multi_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> Result<Vec<SearchResult>> {
        if namespaces.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        // Compute query embedding outside the lock (CPU-heavy)
        let query_embedding = self
            .embedder
            .embed_query(query)
            .await
            .context("computing query embedding")?;
        let query_blob = embedding_to_blob(&query_embedding);
        let active_model_id = self.embedder.name().to_string();

        let conn = self.conn.lock().await;

        // Over-fetch from vec_memories (5x limit to allow for post-filtering).
        // C5/B4: filter by `m.model_id` so dimension/model coexistence is
        // safe. Without this filter, a row embedded by an older model
        // (potentially with a different dimensionality) joins against the
        // current query vector and returns garbage similarities — or, if
        // the dim differs, sqlite-vec rejects the JOIN. Rows from other
        // models stay in the table for `reembed_migration` to rewrite.
        let fetch_k = limit * 5;
        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance,
                    m.id, m.namespace, m.key, m.content, m.metadata,
                    m.created_at, m.updated_at, m.importance, m.access_count,
                    m.last_accessed_at, m.privacy
             FROM vec_memories v
             JOIN memories m ON m.id = v.memory_id
             WHERE v.embedding MATCH ?1 AND k = ?2 AND m.model_id = ?3",
            )
            .context("preparing KNN search")?;

        let now = chrono_now_utc();
        let ns_set: std::collections::HashSet<&str> =
            namespaces.iter().map(|s| s.as_str()).collect();

        let mut results: Vec<SearchResult> = stmt
            .query_map(
                rusqlite::params![query_blob, fetch_k as i64, active_model_id],
                |row| {
                    let distance: f32 = row.get(1)?;
                    let namespace: String = row.get(3)?;
                    let privacy: String = row.get(12)?;
                    let entry = MemoryEntry {
                        id: row.get(2)?,
                        namespace,
                        key: row.get(4)?,
                        content: row.get(5)?,
                        metadata: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                        importance: row.get(9)?,
                        access_count: row.get(10)?,
                        last_accessed_at: row.get(11)?,
                    };
                    Ok((entry, distance, privacy))
                },
            )
            .context("executing KNN search")?
            .filter_map(|r| r.ok())
            .filter(|(entry, _, privacy)| {
                // Post-filter: namespace membership
                if !ns_set.contains(entry.namespace.as_str()) {
                    return false;
                }
                // Post-filter: privacy
                if filter == PrivacyFilter::ExcludeLocalOnly && privacy == "local_only" {
                    return false;
                }
                true
            })
            .map(|(entry, distance, _)| {
                // Convert L2 distance to cosine similarity for L2-normalized vectors:
                // L2_dist² = 2 - 2·cos_sim  →  cos_sim = 1 - L2_dist²/2
                let relevance = (1.0 - distance * distance / 2.0).max(0.0);
                let hours = hours_since(&entry.last_accessed_at, &entry.updated_at, &now);
                let score = retrieval_score(hours, entry.importance, relevance, entry.access_count);
                SearchResult { entry, score }
            })
            .collect();

        // Sort by composite score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        // Update access tracking for returned results
        if !results.is_empty() {
            let ids: Vec<i64> = results.iter().map(|r| r.entry.id).collect();
            let placeholders: String = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "UPDATE memories SET access_count = access_count + 1, last_accessed_at = datetime('now')
                 WHERE id IN ({})",
                placeholders
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = ids
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            let _ = conn.execute(&sql, params.as_slice()); // best-effort
        }

        Ok(results)
    }
}
