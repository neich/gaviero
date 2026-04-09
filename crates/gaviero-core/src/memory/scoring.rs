//! Retrieval scoring for hierarchical memory.
//!
//! Combines vector similarity, FTS keyword rank, scope weight, trust,
//! recency, and importance into a single composite score.
//! Uses Reciprocal Rank Fusion (RRF) to merge vector and FTS results.

use std::collections::HashMap;

use super::scope::{MemoryScope, ScopeFilter, Trust, MemoryType};

// ── SearchConfig ──────────────────────────────────────────────

/// Configuration for a cascading memory search.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// The search query text.
    pub query: String,
    /// Maximum number of results to return (default: 10).
    pub max_results: usize,
    /// Maximum candidates per scope level (default: 20).
    pub per_level_limit: usize,
    /// Minimum cosine similarity to include a result (default: 0.35).
    pub similarity_threshold: f32,
    /// Stop widening scope when best score exceeds this (default: 0.70).
    pub confidence_threshold: f32,
    /// Enable hybrid vector + keyword search (default: true).
    pub use_fts: bool,
    /// The scope chain to cascade through.
    pub scope: MemoryScope,
}

impl SearchConfig {
    /// Create a search config with the given query and scope.
    pub fn new(query: impl Into<String>, scope: MemoryScope) -> Self {
        Self {
            query: query.into(),
            max_results: 10,
            per_level_limit: 20,
            similarity_threshold: 0.35,
            confidence_threshold: 0.70,
            use_fts: true,
            scope,
        }
    }

    pub fn with_max_results(mut self, n: usize) -> Self {
        self.max_results = n;
        self
    }

    pub fn with_fts(mut self, enabled: bool) -> Self {
        self.use_fts = enabled;
        self
    }
}

// ── ScoredMemory ──────────────────────────────────────────────

/// A memory entry with its computed retrieval score.
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub id: i64,
    pub content: String,
    pub content_hash: String,
    pub scope_level: i32,
    pub scope_path: String,
    pub repo_id: Option<String>,
    pub module_path: Option<String>,
    pub memory_type: MemoryType,
    pub trust: Trust,
    pub importance: f32,
    pub access_count: i32,
    pub created_at: String,
    pub updated_at: String,
    pub accessed_at: Option<String>,
    pub tag: Option<String>,
    pub namespace: String,
    pub key: String,

    // Scoring components
    pub raw_similarity: f32,
    pub fts_rank: Option<f32>,
    pub final_score: f32,
}

// ── Scope weights ─────────────────────────────────────────────

/// Default scope weights — narrower scopes get higher weight.
pub fn scope_weight(level: &ScopeFilter) -> f32 {
    match level {
        ScopeFilter::Run { .. } => 1.8,
        ScopeFilter::Module { .. } => 1.5,
        ScopeFilter::Repo { .. } => 1.2,
        ScopeFilter::Workspace => 1.0,
        ScopeFilter::Global => 0.8,
    }
}

// ── Scoring function ──────────────────────────────────────────

/// Compute the final retrieval score for a candidate memory.
///
/// Combines:
/// - `similarity` (0.50 weight): cosine similarity from vector search
/// - `importance` (0.20 weight): stored importance value
/// - `recency` (0.15 weight): exponential decay with 30-day half-life
/// - base existence score (0.15)
///
/// Multiplied by scope weight and trust weight.
pub fn score(
    similarity: f32,
    importance: f32,
    days_since_access: f64,
    access_count: i32,
    trust: Trust,
    level: &ScopeFilter,
) -> f32 {
    let sw = scope_weight(level);
    let tw = trust.weight();

    // Recency: half-life of 30 days → ln(2)/30 ≈ 0.023
    let recency = (-0.023 * days_since_access).exp() as f32;

    // Access reinforcement: mild boost, capped
    let reinforcement = (1.0 + access_count as f32 * 0.05).min(2.0);

    let raw = similarity * 0.50 + importance * 0.20 + recency * 0.15 + 0.15;

    raw * sw * tw * reinforcement
}

// ── Reciprocal Rank Fusion ────────────────────────────────────

/// Merge vector and FTS results via Reciprocal Rank Fusion.
///
/// `vec_results`: (memory_id, cosine_similarity) sorted by similarity desc.
/// `fts_results`: (memory_id, bm25_rank) sorted by rank.
/// `k`: smoothing constant (default 60).
///
/// Returns merged (memory_id, rrf_score, best_similarity) sorted by RRF score desc.
pub fn merge_rrf(
    vec_results: &[(i64, f32)],
    fts_results: &[(i64, f64)],
    k: u32,
) -> Vec<(i64, f32, f32)> {
    let mut scores: HashMap<i64, (f32, f32)> = HashMap::new(); // id → (rrf_score, best_sim)
    let k = k as f32;

    // Vector results get 0.7 weight
    for (rank, &(id, sim)) in vec_results.iter().enumerate() {
        let entry = scores.entry(id).or_insert((0.0, 0.0));
        entry.0 += 0.7 / (k + rank as f32 + 1.0);
        entry.1 = entry.1.max(sim);
    }

    // FTS results get 0.3 weight
    for (rank, &(id, _)) in fts_results.iter().enumerate() {
        let entry = scores.entry(id).or_insert((0.0, 0.0));
        entry.0 += 0.3 / (k + rank as f32 + 1.0);
    }

    let mut merged: Vec<(i64, f32, f32)> = scores
        .into_iter()
        .map(|(id, (rrf, sim))| (id, rrf, sim))
        .collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged
}

// ── Format for prompt injection ───────────────────────────────

/// Format scored memories into a prompt-ready context block.
pub fn format_memories_for_prompt(memories: &[ScoredMemory]) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut ctx = String::from("[Memory context]:\n");
    for m in memories {
        let scope_label = match m.scope_level {
            0 => "global",
            1 => "workspace",
            2 => "repo",
            3 => "module",
            4 => "run",
            _ => "unknown",
        };
        ctx.push_str(&format!(
            "- [{}:{}] {} (score: {:.2})\n",
            scope_label, m.memory_type.as_str(), m.content, m.final_score
        ));
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_narrower_scope_wins() {
        let module_score = score(
            0.8, 0.5, 1.0, 0, Trust::Medium,
            &ScopeFilter::Module { repo_id: "r".into(), module_path: "m".into() },
        );
        let global_score = score(
            0.8, 0.5, 1.0, 0, Trust::Medium,
            &ScopeFilter::Global,
        );
        assert!(module_score > global_score);
    }

    #[test]
    fn test_score_high_trust_wins() {
        let high = score(
            0.7, 0.5, 1.0, 0, Trust::High,
            &ScopeFilter::Repo { repo_id: "r".into() },
        );
        let low = score(
            0.7, 0.5, 1.0, 0, Trust::Low,
            &ScopeFilter::Repo { repo_id: "r".into() },
        );
        assert!(high > low);
    }

    #[test]
    fn test_score_recency_matters() {
        let recent = score(
            0.7, 0.5, 0.0, 0, Trust::Medium,
            &ScopeFilter::Repo { repo_id: "r".into() },
        );
        let old = score(
            0.7, 0.5, 90.0, 0, Trust::Medium,
            &ScopeFilter::Repo { repo_id: "r".into() },
        );
        assert!(recent > old);
    }

    #[test]
    fn test_merge_rrf_combines_sources() {
        let vec_results = vec![(1, 0.9f32), (2, 0.7), (3, 0.5)];
        let fts_results = vec![(2, 10.0f64), (4, 8.0), (1, 5.0)];
        let merged = merge_rrf(&vec_results, &fts_results, 60);

        // Both sources contribute to id=1 and id=2, so they should rank high
        assert!(!merged.is_empty());
        // id=2 appears in both lists at good ranks
        let id2 = merged.iter().find(|m| m.0 == 2).unwrap();
        let id4 = merged.iter().find(|m| m.0 == 4).unwrap();
        assert!(id2.1 > id4.1, "id=2 (in both) should beat id=4 (FTS only)");
    }

    #[test]
    fn test_format_memories_empty() {
        assert_eq!(format_memories_for_prompt(&[]), "");
    }

    #[test]
    fn test_format_memories_output() {
        let memories = vec![ScoredMemory {
            id: 1,
            content: "test memory".into(),
            content_hash: "abc".into(),
            scope_level: 2,
            scope_path: "repo:abc".into(),
            repo_id: Some("abc".into()),
            module_path: None,
            memory_type: MemoryType::Factual,
            trust: Trust::Medium,
            importance: 0.5,
            access_count: 0,
            created_at: "2025-01-01".into(),
            updated_at: "2025-01-01".into(),
            accessed_at: None,
            tag: None,
            namespace: "default".into(),
            key: "k".into(),
            raw_similarity: 0.8,
            fts_rank: None,
            final_score: 0.75,
        }];
        let output = format_memories_for_prompt(&memories);
        assert!(output.contains("[repo:factual]"));
        assert!(output.contains("test memory"));
        assert!(output.contains("0.75"));
    }
}
