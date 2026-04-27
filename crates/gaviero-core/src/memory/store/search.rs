//! Tier B / B3 scoped retrieval surface on `MemoryStore`.
//!
//! Carved out of `store/mod.rs` (Phase 4.7b). Houses the merged
//! multi-scope retrieval pipeline:
//!
//! - the public entry points `search_scoped`, `multi_scope_retrieve`,
//!   `retrieve_at_level` (registry-driven per-store path),
//!   `record_access`, `search_scoped_cascade` (legacy kill-switch),
//!   `search_scoped_context`, and `search_at_level`,
//! - the per-scope hybrid plumbing: `vec_search_at_level`,
//!   `fts_search_at_level`, `filter_by_scope`,
//!   `matches_scope_filter`, `load_scoped_memory`,
//! - the access-log writers `touch_accessed`, `log_access`,
//! - the no-search level lister `list_at_level`.
//!
//! The legacy namespace search lives next door in
//! `store/search_legacy.rs`.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{
    MemoryStore, chrono_now_utc, embedding_to_blob, hours_since,
};
use crate::memory::scope::{MemoryScope, MemoryType, ScopeFilter, Trust};
use crate::memory::scoring::{self, ScoredMemory, SearchConfig};
use crate::memory::trust_defaults::MemorySource;

impl MemoryStore {
    /// B3-aware scope-aware search across all admissible scope levels.
    ///
    /// B3: this is the merged-multi-scope path. All scope levels
    /// participate in a single ranking pool — the cascade with the
    /// 0.70 early-exit is gone. Scope bias survives as the soft
    /// `scope_weight` multiplier already applied inside scoring.
    ///
    /// The legacy cascade is reachable via [`Self::search_scoped_cascade`]
    /// for one release as a kill switch (settings flag
    /// `memory.retrieval.mode = "cascade"`); it will be removed once
    /// the merged path stabilises in eval.
    pub async fn search_scoped(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        self.multi_scope_retrieve(config).await
    }

    /// B3: merged multi-scope retrieval.
    ///
    /// 1. Embed the query **once** (before the SQLite lock).
    /// 2. Acquire the SQLite mutex once and walk every admissible scope
    ///    level under that single lock. The per-scope hybrid (vec + FTS
    ///    + RRF) reads run sequentially — sqlite-vec ships only a
    ///    single connection in this store, so genuine I/O parallelism
    ///    would need a connection pool (a Tier C refactor). The win
    ///    over the pre-B3 cascade is the cascade's serial early-exit,
    ///    not extra cores.
    /// 3. Merge candidates from all scopes into one pool, dedupe by
    ///    content hash (narrower scope wins on ties so module-specific
    ///    memories don't get masked by a workspace duplicate).
    /// 4. Score each via the existing composite formula; the scope
    ///    multiplier applied inside the scorer carries the (now soft)
    ///    scope bias.
    /// 5. Sort by composite score, truncate to `config.max_results`
    ///    (this is the `memory.retrieval.maxMergedPool` cap when called
    ///    from the central `retrieve_ranked` engine). Reranker
    ///    integration happens above this layer in
    ///    `retrieve_for_chat_with_reranker`.
    pub async fn multi_scope_retrieve(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .await
            .context("computing query embedding for multi-scope retrieve")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;
        let now = chrono_now_utc();

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut by_hash: HashMap<String, usize> = HashMap::new();

        for level in config.scope.levels() {
            let scope_level_int = level.level_int();
            let vec_candidates = self.vec_search_at_level(
                &conn,
                &query_blob,
                scope_level_int,
                &level,
                config.per_level_limit,
            )?;

            let fts_candidates = if config.use_fts {
                self.fts_search_at_level(
                    &conn,
                    &config.query,
                    scope_level_int,
                    &level,
                    config.per_level_limit,
                )?
            } else {
                Vec::new()
            };

            // B2/B3 fix: honour FTS-only candidates. The pre-fix code
            // dropped FTS hits when vec returned nothing (fell through
            // to `vec_candidates`), and even in the both-non-empty case
            // FTS-only RRF entries arrived with raw_sim=0 and got
            // filtered by the similarity threshold below. Now:
            //   - vec ∪ fts both non-empty → RRF, then promote FTS-only
            //     (raw_sim==0) entries to the threshold so they survive
            //     the filter and contribute via composite scoring.
            //   - vec empty, fts non-empty → use FTS hits with the
            //     threshold as a synthetic similarity floor.
            //   - vec non-empty, fts empty → use vec as-is.
            //   - both empty → empty pool for this scope.
            let candidate_ids: Vec<(i64, f32)> =
                match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                    (false, false) => {
                        let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                        merged
                            .into_iter()
                            .map(|(id, _rrf, sim)| {
                                let sim = if sim == 0.0 {
                                    config.similarity_threshold
                                } else {
                                    sim
                                };
                                (id, sim)
                            })
                            .collect()
                    }
                    (true, false) => fts_candidates
                        .into_iter()
                        .map(|(id, _rank)| (id, config.similarity_threshold))
                        .collect(),
                    (false, true) => vec_candidates,
                    (true, true) => Vec::new(),
                };

            for (memory_id, raw_sim) in &candidate_ids {
                if *raw_sim < config.similarity_threshold {
                    continue;
                }
                let Some(mem) =
                    self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, &level)?
                else {
                    continue;
                };

                // Cross-scope dedup: when the same content_hash appears
                // at multiple scopes, keep the one with the higher
                // final_score (which, due to scope_multiplier, will
                // typically be the narrower scope at equal similarity).
                match by_hash.get(&mem.content_hash) {
                    Some(&idx) if accumulated[idx].final_score >= mem.final_score => {}
                    Some(&idx) => {
                        accumulated[idx] = mem;
                    }
                    None => {
                        by_hash.insert(mem.content_hash.clone(), accumulated.len());
                        accumulated.push(mem);
                    }
                }
            }
            // No early-exit (B3): every scope's candidates flow into
            // the merged pool unconditionally.
        }

        accumulated.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        accumulated.truncate(config.max_results);

        if !accumulated.is_empty() {
            let ids: Vec<i64> = accumulated.iter().map(|m| m.id).collect();
            self.touch_accessed(&conn, &ids);
            self.log_access(&conn, &ids, &config.scope);
        }

        Ok(accumulated)
    }

    /// Per-store, per-level retrieval. Returns scored memories for
    /// ONE scope level on this store. No cross-scope dedup, no
    /// truncation, no access logging — those happen at the merge
    /// layer in `super::stores::MemoryStores::multi_scope_retrieve`.
    ///
    /// Acquires this store's connection lock briefly. Callers from
    /// the registry pass the precomputed query embedding so it isn't
    /// recomputed per-store.
    pub async fn retrieve_at_level(
        &self,
        query: &str,
        query_blob: &[u8],
        level: &ScopeFilter,
        config: &SearchConfig,
    ) -> Result<Vec<ScoredMemory>> {
        let conn = self.conn.lock().await;
        let now = chrono_now_utc();
        let scope_level_int = level.level_int();
        let vec_candidates = self.vec_search_at_level(
            &conn,
            query_blob,
            scope_level_int,
            level,
            config.per_level_limit,
        )?;
        let fts_candidates = if config.use_fts {
            self.fts_search_at_level(&conn, query, scope_level_int, level, config.per_level_limit)?
        } else {
            Vec::new()
        };
        let candidate_ids: Vec<(i64, f32)> =
            match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                (false, false) => {
                    let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                    merged
                        .into_iter()
                        .map(|(id, _rrf, sim)| {
                            let sim = if sim == 0.0 {
                                config.similarity_threshold
                            } else {
                                sim
                            };
                            (id, sim)
                        })
                        .collect()
                }
                (true, false) => fts_candidates
                    .into_iter()
                    .map(|(id, _rank)| (id, config.similarity_threshold))
                    .collect(),
                (false, true) => vec_candidates,
                (true, true) => Vec::new(),
            };
        let mut out = Vec::with_capacity(candidate_ids.len());
        for (memory_id, raw_sim) in &candidate_ids {
            if *raw_sim < config.similarity_threshold {
                continue;
            }
            if let Some(mem) = self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, level)? {
                out.push(mem);
            }
        }
        Ok(out)
    }

    /// Bump `last_accessed_at` and append a `retrieval_use` entry for
    /// `ids` on this store. Called by the registry-level merge after
    /// a multi-DB retrieval, with `ids` filtered to those that came
    /// from THIS store.
    pub async fn record_access(&self, ids: &[i64], scope: &MemoryScope) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().await;
        self.touch_accessed(&conn, ids);
        self.log_access(&conn, ids, scope);
        Ok(())
    }

    /// B3 kill switch: legacy cascade-with-early-exit retrieval.
    ///
    /// Kept for one release behind `memory.retrieval.mode = "cascade"`
    /// in case the merged path regresses on a workload we haven't
    /// characterised. Will be removed in the next minor cycle.
    #[deprecated(note = "Use multi_scope_retrieve (B3); this is a kill switch.")]
    pub async fn search_scoped_cascade(&self, config: &SearchConfig) -> Result<Vec<ScoredMemory>> {
        let query_embedding = self
            .embedder
            .embed_query(&config.query)
            .await
            .context("computing query embedding for scoped search")?;
        let query_blob = embedding_to_blob(&query_embedding);

        let conn = self.conn.lock().await;
        let now = chrono_now_utc();

        let mut accumulated: Vec<ScoredMemory> = Vec::new();
        let mut seen_hashes: HashSet<String> = HashSet::new();

        for level in config.scope.levels() {
            let scope_level_int = level.level_int();
            let vec_candidates = self.vec_search_at_level(
                &conn,
                &query_blob,
                scope_level_int,
                &level,
                config.per_level_limit,
            )?;
            let fts_candidates = if config.use_fts {
                self.fts_search_at_level(
                    &conn,
                    &config.query,
                    scope_level_int,
                    &level,
                    config.per_level_limit,
                )?
            } else {
                Vec::new()
            };
            // Mirror the FTS-only fallback fix from `multi_scope_retrieve`
            // so the kill-switch path doesn't silently differ in
            // candidate-pool composition.
            let candidate_ids: Vec<(i64, f32)> =
                match (vec_candidates.is_empty(), fts_candidates.is_empty()) {
                    (false, false) => {
                        let merged = scoring::merge_rrf(&vec_candidates, &fts_candidates, 60);
                        merged
                            .into_iter()
                            .map(|(id, _rrf, sim)| {
                                let sim = if sim == 0.0 {
                                    config.similarity_threshold
                                } else {
                                    sim
                                };
                                (id, sim)
                            })
                            .collect()
                    }
                    (true, false) => fts_candidates
                        .into_iter()
                        .map(|(id, _rank)| (id, config.similarity_threshold))
                        .collect(),
                    (false, true) => vec_candidates,
                    (true, true) => Vec::new(),
                };

            for (memory_id, raw_sim) in &candidate_ids {
                if *raw_sim < config.similarity_threshold {
                    continue;
                }
                let Some(mem) =
                    self.load_scoped_memory(&conn, *memory_id, *raw_sim, &now, &level)?
                else {
                    continue;
                };
                if seen_hashes.contains(&mem.content_hash) {
                    continue;
                }
                seen_hashes.insert(mem.content_hash.clone());
                accumulated.push(mem);
            }
            if accumulated.len() >= config.max_results
                && let Some(best) = accumulated.iter().map(|m| m.final_score).reduce(f32::max)
                && best >= config.confidence_threshold
            {
                break;
            }
        }

        accumulated.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        accumulated.truncate(config.max_results);
        if !accumulated.is_empty() {
            let ids: Vec<i64> = accumulated.iter().map(|m| m.id).collect();
            self.touch_accessed(&conn, &ids);
            self.log_access(&conn, &ids, &config.scope);
        }
        Ok(accumulated)
    }

    /// Format scoped search results as a prompt-ready context string.
    pub async fn search_scoped_context(&self, config: &SearchConfig) -> String {
        match self.search_scoped(config).await {
            Ok(results) if !results.is_empty() => scoring::format_memories_for_prompt(&results),
            _ => String::new(),
        }
    }

    /// Query memories at a specific scope level (for consolidation diagnostics).
    pub async fn search_at_level(
        &self,
        level: &ScopeFilter,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        if query.is_empty() {
            // Return all memories at this level
            let conn = self.conn.lock().await;
            return self.list_at_level(&conn, level, limit);
        }

        // Use the cascading search with a single-level scope
        let scope = MemoryScope {
            global_db: std::path::PathBuf::new(),
            workspace_db: std::path::PathBuf::new(),
            repo_db: None,
            workspace_id: String::new(),
            repo_id: level.repo_id().map(String::from),
            module_path: level.module_path().map(String::from),
            run_id: level.run_id().map(String::from),
        };

        let config = SearchConfig {
            query: query.to_string(),
            max_results: limit,
            per_level_limit: limit,
            similarity_threshold: 0.0,
            confidence_threshold: 1.0, // don't terminate early
            use_fts: true,
            scope,
        };

        self.search_scoped(&config).await
    }

    // ── Internal helpers for scoped search ─────────────────────────

    /// Vector KNN search filtered to a scope level.
    ///
    /// C5/B4: also filters by `m.model_id = active_embedder.name()`
    /// so memories embedded by a previous model don't pollute results
    /// after an embedder swap. Without this, a 768-dim row joined to a
    /// 1024-dim query vector would either be rejected by sqlite-vec or
    /// return a meaningless distance.
    pub(super) fn vec_search_at_level(
        &self,
        conn: &Connection,
        query_blob: &[u8],
        scope_level: i32,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<(i64, f32)>> {
        // Over-fetch to allow post-filtering by repo_id/module_path
        let fetch_k = limit * 3;
        let active_model_id = self.embedder.name();

        let mut stmt = conn
            .prepare(
                "SELECT v.memory_id, v.distance
             FROM vec_memories_scoped v
             JOIN memories m ON m.id = v.memory_id
             WHERE v.embedding MATCH ?1 AND k = ?2 AND v.scope_level = ?3
               AND m.model_id = ?4",
            )
            .context("preparing scoped KNN")?;

        let results: Vec<(i64, f32)> = stmt
            .query_map(
                rusqlite::params![query_blob, fetch_k as i64, scope_level, active_model_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();

        // Post-filter by repo_id / module_path / run_id
        let filtered = self.filter_by_scope(conn, results, level)?;
        Ok(filtered.into_iter().take(limit).collect())
    }

    /// FTS search filtered to a scope level.
    pub(super) fn fts_search_at_level(
        &self,
        conn: &Connection,
        query: &str,
        scope_level: i32,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let fetch_k = limit * 3;

        let mut stmt = conn
            .prepare(
                "SELECT f.rowid, f.rank
             FROM memories_fts f
             JOIN memories m ON m.id = f.rowid
             WHERE memories_fts MATCH ?1
             AND m.scope_level = ?2
             AND m.superseded_by IS NULL
             ORDER BY f.rank
             LIMIT ?3",
            )
            .context("preparing FTS search")?;

        let results: Vec<(i64, f64)> = stmt
            .query_map(
                rusqlite::params![query, scope_level, fetch_k as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();

        // Post-filter
        let filtered: Vec<(i64, f64)> = results
            .into_iter()
            .filter(|(id, _)| self.matches_scope_filter(conn, *id, level))
            .take(limit)
            .collect();

        Ok(filtered)
    }

    /// Post-filter vector results by exact scope metadata.
    fn filter_by_scope(
        &self,
        conn: &Connection,
        results: Vec<(i64, f32)>,
        level: &ScopeFilter,
    ) -> Result<Vec<(i64, f32)>> {
        let mut filtered = Vec::new();
        for (id, distance) in results {
            if self.matches_scope_filter(conn, id, level) {
                // Convert L2 distance to cosine similarity
                let similarity = (1.0 - distance * distance / 2.0).max(0.0);
                filtered.push((id, similarity));
            }
        }
        Ok(filtered)
    }

    /// Check if a memory matches the given scope filter.
    fn matches_scope_filter(&self, conn: &Connection, memory_id: i64, level: &ScopeFilter) -> bool {
        match level {
            ScopeFilter::Global | ScopeFilter::Workspace => true,
            ScopeFilter::Repo { repo_id } => conn
                .prepare("SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2")
                .and_then(|mut s| s.query_row(rusqlite::params![memory_id, repo_id], |_| Ok(true)))
                .unwrap_or(false),
            ScopeFilter::Module {
                repo_id,
                module_path,
            } => conn
                .prepare(
                    "SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2 AND module_path = ?3",
                )
                .and_then(|mut s| {
                    s.query_row(rusqlite::params![memory_id, repo_id, module_path], |_| {
                        Ok(true)
                    })
                })
                .unwrap_or(false),
            ScopeFilter::Run { repo_id, run_id } => conn
                .prepare("SELECT 1 FROM memories WHERE id = ?1 AND repo_id = ?2 AND run_id = ?3")
                .and_then(|mut s| {
                    s.query_row(rusqlite::params![memory_id, repo_id, run_id], |_| Ok(true))
                })
                .unwrap_or(false),
        }
    }

    /// Load a full ScoredMemory from the database.
    fn load_scoped_memory(
        &self,
        conn: &Connection,
        memory_id: i64,
        raw_similarity: f32,
        now: &str,
        level: &ScopeFilter,
    ) -> Result<Option<ScoredMemory>> {
        // B4: snapshot the scoring config once per row (cheap RwLock read).
        let scoring_cfg = self.scoring_cfg();
        // B5: exclude superseded rows from retrieval. SUPERSEDE soft-
        // deletes by setting `superseded_by`; the row stays in storage
        // for audit (and for retrieval-by-id paths), but it must not
        // surface through scoped search. The vector and FTS searches
        // upstream may still match a superseded row (sqlite-vec carries
        // no `superseded_by` column); filtering here drops it before
        // scoring so the upstream cost is the only price paid.
        let result = conn
            .prepare(
                "SELECT id, content, content_hash, scope_level, scope_path,
                    repo_id, module_path, memory_type, trust, importance,
                    access_count, created_at, updated_at, last_accessed_at,
                    tag, namespace, key, source, trust_score
             FROM memories WHERE id = ?1 AND superseded_by IS NULL",
            )?
            .query_row(rusqlite::params![memory_id], |row| {
                let accessed_at: Option<String> = row.get(13)?;
                let updated_at: String = row.get(12)?;
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let importance: f32 = row.get(9)?;
                let access_count: i32 = row.get(10)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;

                let trust = Trust::parse_str(&trust_str);
                let source = MemorySource::parse_str(&source_str);
                let memory_type = MemoryType::parse_str(&type_str);
                let days = hours_since(&accessed_at, &updated_at, now) / 24.0;

                // A3: score from the fine-grained per-row trust_score, not
                // the legacy 3-level enum. Keeps the legacy column in the
                // SELECT for backward compat / audit but stops reading it
                // into the formula.
                //
                // B4: pass `memory_type` plus the configured floor +
                // exempt-type list so reference memories (decision /
                // convention / invariant / preference / gotcha) keep
                // recency = 1.0 and other types floor at the configured
                // value (default 0.35) instead of decaying to zero.
                let final_score = scoring::score_with_trust_score(
                    raw_similarity,
                    importance,
                    days,
                    access_count,
                    trust_score,
                    level,
                    memory_type,
                    scoring_cfg.recency_floor,
                    &scoring_cfg.decay_exempt_types,
                );

                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type,
                    trust,
                    importance,
                    access_count,
                    created_at: row.get(11)?,
                    updated_at,
                    accessed_at,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    source,
                    trust_score,
                    raw_similarity,
                    fts_rank: None,
                    final_score,
                })
            });

        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Bump access_count and last_accessed_at for returned results.
    fn touch_accessed(&self, conn: &Connection, ids: &[i64]) {
        for id in ids {
            let _ = conn.execute(
                "UPDATE memories SET access_count = access_count + 1,
                        last_accessed_at = datetime('now')
                 WHERE id = ?1",
                rusqlite::params![id],
            );
        }
    }

    /// Log access for cross-scope promotion heuristics.
    fn log_access(&self, conn: &Connection, ids: &[i64], scope: &MemoryScope) {
        let repo_id = scope.repo_id.as_deref();
        let module_path = scope.module_path.as_deref();

        for id in ids {
            let _ = conn.execute(
                "INSERT INTO memory_access_log (memory_id, repo_id, module_path)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, repo_id, module_path],
            );
        }
    }

    /// List all memories at a level without embedding search.
    pub(super) fn list_at_level(
        &self,
        conn: &Connection,
        level: &ScopeFilter,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>> {
        let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match level {
            ScopeFilter::Global => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 0 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Workspace => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 1 LIMIT ?1",
                vec![Box::new(limit as i64)],
            ),
            ScopeFilter::Repo { repo_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 2 AND repo_id = ?1 LIMIT ?2",
                vec![Box::new(repo_id.clone()), Box::new(limit as i64)],
            ),
            ScopeFilter::Module {
                repo_id,
                module_path,
            } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 3 AND repo_id = ?1 AND module_path = ?2 LIMIT ?3",
                vec![
                    Box::new(repo_id.clone()),
                    Box::new(module_path.clone()),
                    Box::new(limit as i64),
                ],
            ),
            ScopeFilter::Run { repo_id, run_id } => (
                "SELECT id, content, content_hash, scope_level, scope_path,
                        repo_id, module_path, memory_type, trust, importance,
                        access_count, created_at, updated_at, last_accessed_at,
                        tag, namespace, key, source, trust_score
                 FROM memories WHERE scope_level = 4 AND repo_id = ?1 AND run_id = ?2 LIMIT ?3",
                vec![
                    Box::new(repo_id.clone()),
                    Box::new(run_id.clone()),
                    Box::new(limit as i64),
                ],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(sql)?;
        let results: Vec<ScoredMemory> = stmt
            .query_map(params_refs.as_slice(), |row| {
                let trust_str: String = row.get(8)?;
                let type_str: String = row.get(7)?;
                let source_str: String = row.get(17)?;
                let trust_score: f32 = row.get(18)?;
                Ok(ScoredMemory {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    content_hash: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    scope_level: row.get(3)?,
                    scope_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    repo_id: row.get(5)?,
                    module_path: row.get(6)?,
                    memory_type: MemoryType::parse_str(&type_str),
                    trust: Trust::parse_str(&trust_str),
                    importance: row.get(9)?,
                    access_count: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    accessed_at: row.get(13)?,
                    tag: row.get(14)?,
                    namespace: row.get(15)?,
                    key: row.get(16)?,
                    source: MemorySource::parse_str(&source_str),
                    trust_score,
                    raw_similarity: 0.0,
                    fts_rank: None,
                    final_score: 0.0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }
}

