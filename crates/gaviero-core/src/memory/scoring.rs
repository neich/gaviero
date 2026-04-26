//! Retrieval scoring for hierarchical memory.
//!
//! Combines vector similarity, FTS keyword rank, scope weight, trust,
//! recency, and importance into a single composite score.
//! Uses Reciprocal Rank Fusion (RRF) to merge vector and FTS results.

use std::collections::HashMap;

use super::scope::{MemoryScope, MemoryType, ScopeFilter, Trust};

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
    /// **Deprecated (B3).** Pre-B3 cascade early-exit threshold.
    /// Ignored by [`MemoryStore::multi_scope_retrieve`]; honored only
    /// by the legacy `search_scoped_cascade` kill-switch path. Will
    /// be removed in the next minor cycle.
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

    pub fn with_per_level_limit(mut self, n: usize) -> Self {
        self.per_level_limit = n;
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
    /// A3: typed write-origin parsed from the `source` column. Defaults
    /// to `UnknownLegacy` for pre-A3 rows.
    pub source: super::trust_defaults::MemorySource,
    /// A3: per-memory trust multiplier in [0.0, 1.0]. Replaces the role
    /// of the coarse `Trust` enum in composite scoring.
    pub trust_score: f32,

    // Scoring components
    pub raw_similarity: f32,
    pub fts_rank: Option<f32>,
    pub final_score: f32,
}

// ── Scope weights ─────────────────────────────────────────────

/// Default scope-multiplier values. B3 lowers them dramatically vs
/// the pre-merged-retrieval defaults so the cascade's old steep
/// gradient doesn't masquerade as a hard gate now that every scope
/// participates in the merged pool.
///
/// Pre-B3: `1.8 / 1.5 / 1.2 / 1.0 / 0.8` — these effectively guaranteed
/// a Run-scope candidate would beat any Repo candidate at similar
/// similarity. With merged retrieval that's exactly the "Run drowns
/// Repo" failure mode B3 is built to fix, so the gradient is now
/// gentle and cross-scope ordering is dominated by raw quality
/// (similarity, importance, trust) rather than scope.
pub const DEFAULT_RUN_MULTIPLIER: f32 = 1.10;
pub const DEFAULT_MODULE_MULTIPLIER: f32 = 1.05;
pub const DEFAULT_REPO_MULTIPLIER: f32 = 1.00;
pub const DEFAULT_WORKSPACE_MULTIPLIER: f32 = 0.95;
pub const DEFAULT_GLOBAL_MULTIPLIER: f32 = 0.85;

/// Default scope weights (B3-tuned). Narrower scopes still get a
/// modest boost; the gradient is tight enough that a 10-percent
/// similarity gap easily flips the ranking.
pub fn scope_weight(level: &ScopeFilter) -> f32 {
    match level {
        ScopeFilter::Run { .. } => DEFAULT_RUN_MULTIPLIER,
        ScopeFilter::Module { .. } => DEFAULT_MODULE_MULTIPLIER,
        ScopeFilter::Repo { .. } => DEFAULT_REPO_MULTIPLIER,
        ScopeFilter::Workspace => DEFAULT_WORKSPACE_MULTIPLIER,
        ScopeFilter::Global => DEFAULT_GLOBAL_MULTIPLIER,
    }
}

// ── Scoring function ──────────────────────────────────────────

/// B4: default decay floor. A non-exempt memory's recency contribution
/// never drops below this value, so an old high-importance memory stays
/// retrievable instead of silently fading. Tunable via
/// `memory.scoring.recencyFloor`.
pub const DEFAULT_RECENCY_FLOOR: f32 = 0.35;

/// B4: default decay-exempt memory types. Reference facts (decisions,
/// conventions, invariants, explicit user preferences, gotchas) keep
/// `recency = 1.0` regardless of age. Event-like types (factual,
/// procedural, lesson, error, pattern) decay normally with the floor.
pub const DEFAULT_DECAY_EXEMPT_TYPES: &[MemoryType] = &[
    MemoryType::Decision,
    MemoryType::Convention,
    MemoryType::Invariant,
    MemoryType::Preference,
    MemoryType::Gotcha,
];

/// B4: compute the recency contribution for a memory.
///
/// Returns `1.0` for types in `exempt_types` (no decay applies);
/// otherwise `max(floor, exp(-ln(2)/30 * days_since_access))` so old
/// non-exempt memories remain at least floor-retrievable rather than
/// decaying to zero.
pub fn recency_factor(
    days_since_access: f64,
    memory_type: MemoryType,
    floor: f32,
    exempt_types: &[MemoryType],
) -> f32 {
    if exempt_types.contains(&memory_type) {
        return 1.0;
    }
    let raw = (-0.023 * days_since_access).exp() as f32;
    raw.max(floor.clamp(0.0, 1.0))
}

/// Compute the final retrieval score for a candidate memory (legacy A2-era API).
///
/// Combines:
/// - `similarity` (0.50 weight): cosine similarity from vector search
/// - `importance` (0.20 weight): stored importance value
/// - `recency` (0.15 weight): exponential decay with 30-day half-life
///   (B4: clamped to `DEFAULT_RECENCY_FLOOR` for non-exempt types,
///   `1.0` for exempt types).
/// - base existence score (0.15)
///
/// Multiplied by scope weight and coarse `Trust` enum weight.
///
/// **New code should prefer [`score_with_trust_score`]**, which uses the
/// fine-grained per-row `trust_score` (A3) instead of the three-level
/// `Trust` enum. Kept for tests and any pre-A3 caller that hasn't
/// migrated.
pub fn score(
    similarity: f32,
    importance: f32,
    days_since_access: f64,
    access_count: i32,
    trust: Trust,
    level: &ScopeFilter,
) -> f32 {
    score_with_recency(
        similarity,
        importance,
        days_since_access,
        access_count,
        trust,
        level,
        MemoryType::Factual,
        DEFAULT_RECENCY_FLOOR,
        DEFAULT_DECAY_EXEMPT_TYPES,
    )
}

/// Like [`score`] but accepts the B4 recency configuration explicitly.
#[allow(clippy::too_many_arguments)]
pub fn score_with_recency(
    similarity: f32,
    importance: f32,
    days_since_access: f64,
    access_count: i32,
    trust: Trust,
    level: &ScopeFilter,
    memory_type: MemoryType,
    recency_floor: f32,
    exempt_types: &[MemoryType],
) -> f32 {
    let sw = scope_weight(level);
    let tw = trust.weight();
    let recency = recency_factor(days_since_access, memory_type, recency_floor, exempt_types);
    let reinforcement = (1.0 + access_count as f32 * 0.05).min(2.0);
    let raw = similarity * 0.50 + importance * 0.20 + recency * 0.15 + 0.15;
    raw * sw * tw * reinforcement
}

/// A3 scoring: same formula as [`score`] but with a continuous
/// `trust_score` multiplier in [0.0, 1.0]. This is the authoritative
/// scoring function for all A3+ writes; `load_scoped_memory` calls it
/// with the value from the `memories.trust_score` column.
///
/// A user memory at `trust_score = 1.0` and similarity 0.9 outranks an
/// LLM-extracted memory at `trust_score = 0.6` and similarity 0.92 —
/// which is exactly the "user's own note surfaces first" property the
/// plan asks for.
///
/// B4: applies the recency floor / exemption rules from
/// [`recency_factor`].
#[allow(clippy::too_many_arguments)]
pub fn score_with_trust_score(
    similarity: f32,
    importance: f32,
    days_since_access: f64,
    access_count: i32,
    trust_score: f32,
    level: &ScopeFilter,
    memory_type: MemoryType,
    recency_floor: f32,
    exempt_types: &[MemoryType],
) -> f32 {
    let sw = scope_weight(level);
    let t = trust_score.clamp(0.0, 1.0);

    let recency = recency_factor(days_since_access, memory_type, recency_floor, exempt_types);
    let reinforcement = (1.0 + access_count as f32 * 0.05).min(2.0);

    let raw = similarity * 0.50 + importance * 0.20 + recency * 0.15 + 0.15;
    raw * sw * t * reinforcement
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
            scope_label,
            m.memory_type.as_str(),
            m.content,
            m.final_score
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
            0.8,
            0.5,
            1.0,
            0,
            Trust::Medium,
            &ScopeFilter::Module {
                repo_id: "r".into(),
                module_path: "m".into(),
            },
        );
        let global_score = score(0.8, 0.5, 1.0, 0, Trust::Medium, &ScopeFilter::Global);
        assert!(module_score > global_score);
    }

    #[test]
    fn test_score_high_trust_wins() {
        let high = score(
            0.7,
            0.5,
            1.0,
            0,
            Trust::High,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
        );
        let low = score(
            0.7,
            0.5,
            1.0,
            0,
            Trust::Low,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
        );
        assert!(high > low);
    }

    #[test]
    fn a3_user_trust_outranks_llm_at_comparable_similarity() {
        // Plan §A3 acceptance criterion #1: a user-authored memory at
        // trust 1.0 must outrank an LLM-extracted memory at trust 0.6
        // when similarity is comparable.
        let user = score_with_trust_score(
            0.94,
            0.5,
            1.0,
            0,
            1.0,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
            MemoryType::Factual,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        let llm = score_with_trust_score(
            0.96,
            0.5,
            1.0,
            0,
            0.6,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
            MemoryType::Factual,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        assert!(
            user > llm,
            "user@trust=1.0/sim=0.94 ({user}) should outrank llm@trust=0.6/sim=0.96 ({llm})"
        );
    }

    #[test]
    fn test_score_recency_matters() {
        let recent = score(
            0.7,
            0.5,
            0.0,
            0,
            Trust::Medium,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
        );
        let old = score(
            0.7,
            0.5,
            90.0,
            0,
            Trust::Medium,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
        );
        assert!(recent > old);
    }

    #[test]
    fn b4_recency_factor_floors_non_exempt() {
        // 180-day-old non-exempt memory: raw decay would be ~0.015,
        // floor at 0.35 should pin it.
        let r = recency_factor(
            180.0,
            MemoryType::Factual,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        assert!((r - 0.35).abs() < 1e-6, "expected floor 0.35, got {r}");
    }

    #[test]
    fn b4_recency_factor_exempts_decision() {
        let r = recency_factor(
            180.0,
            MemoryType::Decision,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        assert_eq!(r, 1.0);
    }

    #[test]
    fn b4_recency_factor_fresh_non_exempt_above_floor() {
        let r = recency_factor(
            7.0,
            MemoryType::Factual,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        assert!(r > DEFAULT_RECENCY_FLOOR);
        assert!(r <= 1.0);
    }

    #[test]
    fn b4_old_decision_outranks_recent_observation() {
        // Plan §B4 acceptance: 180-day-old decision (sim 0.85) must
        // outrank 7-day-old observation (sim 0.86).
        let old_decision = score_with_trust_score(
            0.85,
            0.5,
            180.0,
            0,
            0.75,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
            MemoryType::Decision,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        let recent_factual = score_with_trust_score(
            0.86,
            0.5,
            7.0,
            0,
            0.75,
            &ScopeFilter::Repo {
                repo_id: "r".into(),
            },
            MemoryType::Factual,
            DEFAULT_RECENCY_FLOOR,
            DEFAULT_DECAY_EXEMPT_TYPES,
        );
        assert!(
            old_decision > recent_factual,
            "old decision ({old_decision}) should outrank recent factual ({recent_factual})"
        );
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
            source: crate::memory::trust_defaults::MemorySource::UnknownLegacy,
            trust_score: 0.75,
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
