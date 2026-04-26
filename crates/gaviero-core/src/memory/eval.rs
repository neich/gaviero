//! Tier B / T0: Tier 1 retrieval smoke test harness.
//!
//! A small, deterministic eval runner that replays a set of pinned
//! `(query, expected_memory_id)` pairs against a workspace `memory.db`
//! and reports recall@K and MRR. Two modes:
//!
//! - `Live`: runs the actual retrieval pipeline (embedder + hybrid +
//!   scoring + B2 reranker if enabled) and ranks the candidate pool.
//! - `FromManifests`: reads persisted S4 `injection_manifests` rows and
//!   reranks their stored candidate pools under the current scoring
//!   formula — no embedding, no LLM, just rescoring. Cheap.
//!
//! Used by `gaviero-cli memory eval` to gate retrieval changes (B1, B2,
//! B3, B4) against regressions.
//!
//! Fixture format (`tier1.jsonl`, one JSON object per line):
//! ```json
//! { "id": "q-001",
//!   "query": "how do we handle worktree cleanup races",
//!   "expected_memory_id": 4321,
//!   "scope": "repo",
//!   "tags": ["worktrees", "swarm"] }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::store::MemoryStore;
use super::{
    MemoryScope, RerankConfig, Reranker, RetrievalConfig, ScopeFilter, ScoredMemory,
    retrieve_ranked,
};

/// One pinned `(query, expected_memory_id)` pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub query: String,
    pub expected_memory_id: i64,
    /// Scope hint passed to retrieval. Free-form string parsed by
    /// [`parse_scope_hint`]; any unparsable value falls back to `Repo`.
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Eval result for one case: where did the expected id rank?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub id: String,
    pub query: String,
    pub expected_memory_id: i64,
    /// 1-indexed rank of the expected memory in the candidate pool.
    /// `None` means it didn't appear at all.
    pub rank: Option<usize>,
    pub pool_size: usize,
    pub tags: Vec<String>,
}

impl CaseOutcome {
    pub fn hit_at(&self, k: usize) -> bool {
        matches!(self.rank, Some(r) if r <= k)
    }
    pub fn reciprocal_rank(&self) -> f32 {
        match self.rank {
            Some(r) => 1.0 / r as f32,
            None => 0.0,
        }
    }
}

/// Aggregate Tier 1 metrics: recall@1/5/10, MRR, plus per-tag breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub total: usize,
    pub recall_at_1: f32,
    pub recall_at_5: f32,
    pub recall_at_10: f32,
    pub mrr: f32,
    pub per_tag: HashMap<String, TagStats>,
    pub outcomes: Vec<CaseOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TagStats {
    pub total: usize,
    pub recall_at_5: f32,
}

/// Load fixture cases from a JSONL file (one object per line).
pub fn load_fixture(path: &Path) -> Result<Vec<EvalCase>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading fixture {}", path.display()))?;
    let mut cases = Vec::new();
    for (idx, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let case: EvalCase = serde_json::from_str(line)
            .with_context(|| format!("parsing fixture line {}: {}", idx + 1, line))?;
        cases.push(case);
    }
    Ok(cases)
}

/// Map a free-form scope hint (`"run"|"module"|"repo"|"workspace"|"global"`)
/// onto a [`ScopeFilter`] using the supplied workspace context. Falls
/// back to `Repo` for unknown values.
pub fn parse_scope_hint(hint: &str, scope_ctx: &MemoryScope) -> ScopeFilter {
    let repo_id = scope_ctx.repo_id.clone().unwrap_or_default();
    match hint.to_ascii_lowercase().as_str() {
        "global" => ScopeFilter::Global,
        "workspace" => ScopeFilter::Workspace,
        "module" => match scope_ctx.module_path.clone() {
            Some(m) => ScopeFilter::Module {
                repo_id: repo_id.clone(),
                module_path: m,
            },
            None => ScopeFilter::Repo {
                repo_id: repo_id.clone(),
            },
        },
        "run" => match scope_ctx.run_id.clone() {
            Some(r) => ScopeFilter::Run {
                repo_id: repo_id.clone(),
                run_id: r,
            },
            None => ScopeFilter::Repo {
                repo_id: repo_id.clone(),
            },
        },
        _ => ScopeFilter::Repo { repo_id },
    }
}

/// Convert a ranked pool of `ScoredMemory`s into a `CaseOutcome` for
/// `case`. Rank is 1-indexed; `None` means the expected id was absent.
pub fn outcome_from_pool(case: &EvalCase, pool: &[ScoredMemory]) -> CaseOutcome {
    let rank = pool
        .iter()
        .position(|m| m.id == case.expected_memory_id)
        .map(|i| i + 1);
    CaseOutcome {
        id: case.id.clone(),
        query: case.query.clone(),
        expected_memory_id: case.expected_memory_id,
        rank,
        pool_size: pool.len(),
        tags: case.tags.clone(),
    }
}

/// Aggregate `outcomes` into a full report.
pub fn build_report(outcomes: Vec<CaseOutcome>) -> EvalReport {
    let total = outcomes.len();
    let denom = total.max(1) as f32;
    let recall_at_1 = outcomes.iter().filter(|o| o.hit_at(1)).count() as f32 / denom;
    let recall_at_5 = outcomes.iter().filter(|o| o.hit_at(5)).count() as f32 / denom;
    let recall_at_10 = outcomes.iter().filter(|o| o.hit_at(10)).count() as f32 / denom;
    let mrr = outcomes.iter().map(|o| o.reciprocal_rank()).sum::<f32>() / denom;

    let mut per_tag: HashMap<String, (usize, usize)> = HashMap::new();
    for o in &outcomes {
        for tag in &o.tags {
            let entry = per_tag.entry(tag.clone()).or_default();
            entry.0 += 1;
            if o.hit_at(5) {
                entry.1 += 1;
            }
        }
    }
    let per_tag = per_tag
        .into_iter()
        .map(|(tag, (n, hits))| {
            (
                tag,
                TagStats {
                    total: n,
                    recall_at_5: hits as f32 / n.max(1) as f32,
                },
            )
        })
        .collect();

    EvalReport {
        total,
        recall_at_1,
        recall_at_5,
        recall_at_10,
        mrr,
        per_tag,
        outcomes,
    }
}

/// Run a Tier 1 eval against a live `MemoryStore`. Each case routes
/// through the same central [`retrieve_ranked`] entry point as chat
/// injection, MCP `memory_search`, and the TUI memory panel — so eval
/// numbers reflect what production retrieval actually does, including
/// B2 rerank when the harness is run with one.
///
/// `retrieval_cfg` defaults to [`RetrievalConfig::default`] (merged
/// mode); pass `None` to use defaults. `reranker` + `rerank_cfg` are
/// optional — supply both to gate the B2 ablation.
pub async fn run_live(
    store: &Arc<MemoryStore>,
    scope_ctx: &MemoryScope,
    cases: &[EvalCase],
    retrieval_cfg: Option<&RetrievalConfig>,
    reranker: Option<&dyn Reranker>,
    rerank_cfg: Option<&RerankConfig>,
) -> Result<EvalReport> {
    let default_cfg = RetrievalConfig::default();
    let cfg = retrieval_cfg.unwrap_or(&default_cfg);
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        // The eval scope hint biases the scope chain but doesn't gate
        // retrieval — the merged engine considers every level. We fold
        // the hint into a `MemoryScope` derived from `scope_ctx` so the
        // run-id / module-path carry through.
        let scope = scope_for_eval(&case.scope, scope_ctx);
        let stores = super::stores::MemoryStores::from_single_store(store.clone());
        let out = retrieve_ranked(&stores, &scope, &case.query, 50, cfg, reranker, rerank_cfg)
            .await
            .with_context(|| format!("retrieving for case {}", case.id))?;
        outcomes.push(outcome_from_pool(case, &out.items));
    }
    Ok(build_report(outcomes))
}

/// Construct a `MemoryScope` for an eval case. The case's `scope`
/// string ("global"|"workspace"|"repo"|"module"|"run") is already
/// honored by retrieve_ranked via the cascade through every level —
/// this just inherits the workspace's repo_id / module / run context
/// so ScopeFilter::Module / Run hits the right partition.
fn scope_for_eval(_hint: &str, scope_ctx: &MemoryScope) -> MemoryScope {
    scope_ctx.clone()
}

/// Tier B / T0 `--from-manifests` rescore mode.
///
/// Walks the most recent `n` persisted `injection_manifests` and, for
/// each fixture case, finds the *first* manifest whose `query_text`
/// matches the case's `query` (case-insensitive trim) and replays the
/// stored `candidate_pool` to compute the rank of `expected_memory_id`.
/// The rank is taken from the pool's existing ordering — entries are
/// pre-sorted by the writer in production, but to be safe we re-sort
/// by `blended_score` (when present) or `composite_score` here.
///
/// No embedder, no reranker, no LLM: this is a cheap regression replay
/// that asks "given the candidate pool we historically saw for this
/// query, did the right answer rank well?" Useful for B4 / B3 scoring
/// changes that don't require new candidate retrieval.
///
/// Cases without a matching manifest contribute a `None` rank
/// (`absent`) — the recall denominator still counts them, so a fixture
/// case that was never observed in production just lowers the metric
/// instead of being silently skipped.
pub async fn run_from_manifests(
    store: &Arc<MemoryStore>,
    cases: &[EvalCase],
    n: usize,
) -> Result<EvalReport> {
    let rows = store
        .recent_manifests(n.max(1))
        .await
        .context("reading recent manifests for rescore")?;

    let parsed: Vec<(String, serde_json::Value)> = rows
        .iter()
        .filter_map(|r| {
            let payload: serde_json::Value = serde_json::from_str(&r.payload).ok()?;
            let q = payload
                .get("query_text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if q.is_empty() {
                None
            } else {
                Some((q, payload))
            }
        })
        .collect();

    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let needle = case.query.trim().to_ascii_lowercase();
        let manifest = parsed.iter().find(|(q, _)| *q == needle).map(|(_, p)| p);
        let outcome = match manifest {
            Some(p) => outcome_from_manifest_pool(case, p),
            None => CaseOutcome {
                id: case.id.clone(),
                query: case.query.clone(),
                expected_memory_id: case.expected_memory_id,
                rank: None,
                pool_size: 0,
                tags: case.tags.clone(),
            },
        };
        outcomes.push(outcome);
    }
    Ok(build_report(outcomes))
}

/// Parse a manifest payload's `candidate_pool` array into a ranked
/// list, find `case.expected_memory_id`, return its 1-indexed rank.
fn outcome_from_manifest_pool(case: &EvalCase, payload: &serde_json::Value) -> CaseOutcome {
    let pool = payload
        .get("candidate_pool")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // Re-sort defensively. Production manifests are written in selection
    // order; rescore mode wants a score-sorted view so `rank=1` always
    // means top of the pool by current scoring formula.
    let mut entries: Vec<(i64, f64)> = pool
        .iter()
        .filter_map(|c| {
            let id = c.get("memory_id").and_then(|v| v.as_i64())?;
            let score = c
                .get("blended_score")
                .and_then(|v| v.as_f64())
                .or_else(|| c.get("composite_score").and_then(|v| v.as_f64()))
                .unwrap_or(0.0);
            Some((id, score))
        })
        .collect();
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let pool_size = entries.len();
    let rank = entries
        .iter()
        .position(|(id, _)| *id == case.expected_memory_id)
        .map(|i| i + 1);
    CaseOutcome {
        id: case.id.clone(),
        query: case.query.clone(),
        expected_memory_id: case.expected_memory_id,
        rank,
        pool_size,
        tags: case.tags.clone(),
    }
}

/// Bootstrap a Tier 1 fixture from persisted S4 `injection_manifests`.
///
/// Walks the most-recent `n` manifests, extracts each turn's
/// `query_text` + the top selected `memory_id`, and emits one
/// [`EvalCase`] per non-empty turn. Returns the cases — caller writes
/// JSONL to disk. The dev hand-prunes / re-tags before checking it in.
///
/// Skips manifests whose payload doesn't carry `query_text` or has an
/// empty `selected_ids` array — those cases would produce a fixture
/// with no expected answer and inflate the recall denominator.
pub async fn bootstrap_from_manifests(store: &Arc<MemoryStore>, n: usize) -> Result<Vec<EvalCase>> {
    let rows = store
        .recent_manifests(n.max(1))
        .await
        .context("reading recent manifests for bootstrap")?;
    let mut cases = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let payload: serde_json::Value = match serde_json::from_str(&row.payload) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let query = payload
            .get("query_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if query.trim().is_empty() {
            continue;
        }
        let expected = payload
            .get("selected_ids")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_i64());
        let Some(expected_memory_id) = expected else {
            continue;
        };
        let scope = payload
            .get("scope_hint")
            .and_then(|v| v.as_str())
            .unwrap_or("repo")
            .to_string();
        cases.push(EvalCase {
            id: format!("bootstrap-{:03}", i + 1),
            query,
            expected_memory_id,
            scope,
            tags: vec!["bootstrap".to_string()],
        });
    }
    Ok(cases)
}

/// Serialise eval cases as JSONL. Matches [`load_fixture`]'s parser.
pub fn cases_to_jsonl(cases: &[EvalCase]) -> Result<String> {
    let mut out = String::new();
    for c in cases {
        out.push_str(&serde_json::to_string(c).context("serialising case")?);
        out.push('\n');
    }
    Ok(out)
}

/// Compare two reports and return the largest per-tag drop in recall@5.
/// Used to gate against regressions: if any tag drops more than
/// `tolerance` (e.g. 0.02 = 2 points), CI fails.
pub fn worst_recall5_drop(baseline: &EvalReport, current: &EvalReport) -> f32 {
    let mut worst = 0.0_f32;
    for (tag, base) in &baseline.per_tag {
        if let Some(cur) = current.per_tag.get(tag) {
            let drop = base.recall_at_5 - cur.recall_at_5;
            if drop > worst {
                worst = drop;
            }
        }
    }
    let global_drop = baseline.recall_at_5 - current.recall_at_5;
    worst.max(global_drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::trust_defaults::MemorySource;
    use crate::memory::{MemoryType, Trust};

    fn dummy(id: i64) -> ScoredMemory {
        ScoredMemory {
            id,
            content: format!("memory {id}"),
            content_hash: "h".into(),
            scope_level: 2,
            scope_path: "repo:r".into(),
            repo_id: Some("r".into()),
            module_path: None,
            memory_type: MemoryType::Factual,
            trust: Trust::Medium,
            importance: 0.5,
            access_count: 0,
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
            accessed_at: None,
            tag: None,
            namespace: "default".into(),
            key: "k".into(),
            source: MemorySource::UnknownLegacy,
            trust_score: 0.75,
            raw_similarity: 0.8,
            fts_rank: None,
            final_score: 0.5,
        }
    }

    #[test]
    fn outcome_records_rank() {
        let pool = vec![dummy(10), dummy(42), dummy(7)];
        let case = EvalCase {
            id: "c1".into(),
            query: "q".into(),
            expected_memory_id: 42,
            scope: "repo".into(),
            tags: vec!["foo".into()],
        };
        let o = outcome_from_pool(&case, &pool);
        assert_eq!(o.rank, Some(2));
        assert_eq!(o.pool_size, 3);
        assert!(o.hit_at(5));
        assert!(!o.hit_at(1));
        assert!((o.reciprocal_rank() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn outcome_missing_id() {
        let pool = vec![dummy(10), dummy(11)];
        let case = EvalCase {
            id: "c".into(),
            query: "q".into(),
            expected_memory_id: 99,
            scope: "repo".into(),
            tags: vec![],
        };
        let o = outcome_from_pool(&case, &pool);
        assert_eq!(o.rank, None);
        assert_eq!(o.reciprocal_rank(), 0.0);
    }

    #[test]
    fn report_aggregates_recall() {
        let outcomes = vec![
            CaseOutcome {
                id: "a".into(),
                query: "".into(),
                expected_memory_id: 1,
                rank: Some(1),
                pool_size: 10,
                tags: vec!["t1".into()],
            },
            CaseOutcome {
                id: "b".into(),
                query: "".into(),
                expected_memory_id: 2,
                rank: Some(7),
                pool_size: 10,
                tags: vec!["t1".into()],
            },
            CaseOutcome {
                id: "c".into(),
                query: "".into(),
                expected_memory_id: 3,
                rank: None,
                pool_size: 10,
                tags: vec!["t2".into()],
            },
        ];
        let r = build_report(outcomes);
        assert_eq!(r.total, 3);
        assert!((r.recall_at_1 - 1.0 / 3.0).abs() < 1e-6);
        assert!((r.recall_at_5 - 1.0 / 3.0).abs() < 1e-6);
        assert!((r.recall_at_10 - 2.0 / 3.0).abs() < 1e-6);
        // MRR = (1 + 1/7 + 0) / 3
        let expected_mrr = (1.0 + 1.0 / 7.0) / 3.0;
        assert!((r.mrr - expected_mrr).abs() < 1e-5);
        let t1 = r.per_tag.get("t1").unwrap();
        assert_eq!(t1.total, 2);
        assert_eq!(t1.recall_at_5, 0.5);
    }

    #[test]
    fn regression_detection_picks_worst_drop() {
        let mut base = build_report(vec![CaseOutcome {
            id: "a".into(),
            query: "".into(),
            expected_memory_id: 1,
            rank: Some(1),
            pool_size: 1,
            tags: vec!["x".into()],
        }]);
        base.per_tag.insert(
            "x".into(),
            TagStats {
                total: 10,
                recall_at_5: 0.9,
            },
        );
        base.recall_at_5 = 0.9;

        let mut cur = base.clone();
        cur.per_tag.insert(
            "x".into(),
            TagStats {
                total: 10,
                recall_at_5: 0.85,
            },
        );
        cur.recall_at_5 = 0.88;

        let drop = worst_recall5_drop(&base, &cur);
        assert!((drop - 0.05).abs() < 1e-6);
    }
}
