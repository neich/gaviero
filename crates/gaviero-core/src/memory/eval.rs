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

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::store::MemoryStore;
use super::{
    MemoryScope, RerankConfig, Reranker, RetrievalConfig, ScopeFilter, ScoredMemory,
    retrieve_ranked,
};

/// One pinned eval case.
///
/// The legacy schema (pre-T1.3) was just `(query, expected_memory_id)`.
/// T1.3 keeps that schema loadable unchanged — `expected_memory_id`
/// becomes `Option`, all new fields default to `None`/empty so existing
/// `tier1.jsonl` fixtures (incl. the empty template) load and run via
/// `serde`'s `default` handling. Bootstrap fixtures
/// (`bootstrap_from_manifests`) also keep the legacy shape.
///
/// New (T1.3) fields enable code-prompt blast-radius metrics: gold sets
/// (must / neutral / forbid), graded relevance, and an expected scope
/// path for blast-leakage detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub query: String,
    /// Legacy single-answer pin. `None` is allowed when the case
    /// expresses ground truth via `gold_must` / `gold_neutral`
    /// / `gold_forbid` instead (typical for code prompts that don't
    /// reduce to a single memory id).
    #[serde(default)]
    pub expected_memory_id: Option<i64>,
    /// Scope hint passed to retrieval. Free-form string parsed by
    /// [`parse_scope_hint`]; any unparsable value falls back to `Repo`.
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub tags: Vec<String>,

    // ── T1.3 additive ──────────────────────────────────────────────

    /// Optional code-prompt taxonomy: refactor / bugfix / feature / explain.
    #[serde(default)]
    pub kind: Option<CaseKind>,
    /// Items that retrieval **must** surface for the case to be
    /// considered correct. Recall is gated on this set only.
    #[serde(default)]
    pub gold_must: Vec<GoldRef>,
    /// Items that may legitimately appear but are not required.
    #[serde(default)]
    pub gold_neutral: Vec<GoldRef>,
    /// Items that **must not** appear. Drives `forbid_hit_rate`.
    #[serde(default)]
    pub gold_forbid: Vec<GoldRef>,
    /// Per-item graded relevance (0..3) keyed by stringified `GoldRef`
    /// (`"File:..."` / `"Symbol:..."` / `"Memory:..."` / `"MemoryTag:..."`).
    /// Default grading when an item is absent: `must = 3`, `neutral = 1`,
    /// `forbid = 0`, unmentioned = 0.
    #[serde(default)]
    pub graded: BTreeMap<String, u8>,
    /// Path-prefix scope where the prompt's edit/answer is expected to
    /// land. Drives `blast_leakage`: any retrieved item outside the
    /// parent scopes of this path is leakage. `None` opts out of the
    /// blast-leakage metric for this case.
    #[serde(default)]
    pub expected_scope_path: Option<String>,
}

/// Code-prompt taxonomy used for diagnostics. Carries no scoring
/// semantics on its own.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    Refactor,
    Bugfix,
    Feature,
    Explain,
}

/// Reference to a gold-set item. Files use repo-relative paths
/// (a trailing `/` matches a directory prefix); symbols use a
/// `module::Type::method` qualifier; memories pin a row id or a tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GoldRef {
    File(String),
    Symbol(String),
    Memory(i64),
    MemoryTag(String),
}

impl GoldRef {
    /// Stable string key used to look up `graded[]` entries. Format
    /// matches the schema documented on `EvalCase::graded`.
    pub fn graded_key(&self) -> String {
        match self {
            GoldRef::File(p) => format!("File:{p}"),
            GoldRef::Symbol(s) => format!("Symbol:{s}"),
            GoldRef::Memory(id) => format!("Memory:{id}"),
            GoldRef::MemoryTag(t) => format!("MemoryTag:{t}"),
        }
    }
}

/// Eval result for one case: where did the expected id rank?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub id: String,
    pub query: String,
    /// Legacy single-id pin; `None` when the case is gold-set-only.
    #[serde(default)]
    pub expected_memory_id: Option<i64>,
    /// 1-indexed rank of the expected memory in the candidate pool.
    /// `None` means it didn't appear (or the case had no `expected_memory_id`).
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
///
/// T1.3 additive fields (precision_at_5/10, ndcg_at_5/10, blast_leakage,
/// over_retrieval, under_retrieval, forbid_hit_rate) default to 0.0 when
/// the underlying outcomes carried no gold sets — the legacy single-id
/// path keeps producing meaningful Recall@K / MRR alongside zero
/// gold-set metrics. Aggregation is the unweighted mean across cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub total: usize,
    pub recall_at_1: f32,
    pub recall_at_5: f32,
    pub recall_at_10: f32,
    pub mrr: f32,
    pub per_tag: HashMap<String, TagStats>,
    pub outcomes: Vec<CaseOutcome>,

    // ── T1.3 additive ──────────────────────────────────────────────

    #[serde(default)]
    pub precision_at_5: f32,
    #[serde(default)]
    pub precision_at_10: f32,
    #[serde(default)]
    pub ndcg_at_5: f32,
    #[serde(default)]
    pub ndcg_at_10: f32,
    /// Mean fraction of retrieved items whose path is outside the
    /// parent scopes of `expected_scope_path` (per case). Cases without
    /// `expected_scope_path` contribute 0.0 to the mean denominator.
    #[serde(default)]
    pub blast_leakage: f32,
    /// Mean over-retrieval rate: `|R \ (must ∪ neutral)| / |R|`.
    #[serde(default)]
    pub over_retrieval: f32,
    /// Mean under-retrieval rate: `|must \ R| / |must|`.
    #[serde(default)]
    pub under_retrieval: f32,
    /// Mean forbid hit rate: `|R ∩ forbid| / |R|`.
    #[serde(default)]
    pub forbid_hit_rate: f32,
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
/// `case`. Rank is 1-indexed; `None` means the expected id was absent
/// or the case carries no `expected_memory_id`.
pub fn outcome_from_pool(case: &EvalCase, pool: &[ScoredMemory]) -> CaseOutcome {
    let rank = case
        .expected_memory_id
        .and_then(|id| pool.iter().position(|m| m.id == id))
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
///
/// Legacy callers (no gold sets) get Recall@K / MRR / per-tag stats and
/// the new T1.3 fields all zero — there's nothing to score against.
/// Use [`build_report_with_pools`] when you have per-case retrieved
/// pools and gold sets to populate Precision / NDCG / blast metrics.
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
        precision_at_5: 0.0,
        precision_at_10: 0.0,
        ndcg_at_5: 0.0,
        ndcg_at_10: 0.0,
        blast_leakage: 0.0,
        over_retrieval: 0.0,
        under_retrieval: 0.0,
        forbid_hit_rate: 0.0,
    }
}

/// Build a full report including T1.3 gold-set metrics.
///
/// Pairs each `case` with the retrieved pool from that case's run.
/// Cases without gold sets contribute 0.0 to the gold-set numerators
/// and are still counted in the denominator — the metric semantics are
/// "mean across all cases", consistent with how Recall@K aggregates.
pub fn build_report_with_pools(
    cases: &[EvalCase],
    pools: &[Vec<ScoredMemory>],
) -> EvalReport {
    let outcomes: Vec<CaseOutcome> = cases
        .iter()
        .zip(pools.iter())
        .map(|(c, p)| outcome_from_pool(c, p))
        .collect();
    let mut report = build_report(outcomes);

    let n = cases.len();
    if n == 0 {
        return report;
    }
    let denom = n as f32;

    let mut sum_p5 = 0.0_f32;
    let mut sum_p10 = 0.0_f32;
    let mut sum_ndcg5 = 0.0_f32;
    let mut sum_ndcg10 = 0.0_f32;
    let mut sum_leak = 0.0_f32;
    let mut sum_over = 0.0_f32;
    let mut sum_under = 0.0_f32;
    let mut sum_forbid = 0.0_f32;

    for (case, pool) in cases.iter().zip(pools.iter()) {
        sum_p5 += precision_at_k(case, pool, 5);
        sum_p10 += precision_at_k(case, pool, 10);
        sum_ndcg5 += ndcg_at_k(case, pool, 5);
        sum_ndcg10 += ndcg_at_k(case, pool, 10);
        sum_leak += blast_leakage_for(case, pool);
        sum_over += over_retrieval_for(case, pool);
        sum_under += under_retrieval_for(case, pool);
        sum_forbid += forbid_hit_rate_for(case, pool);
    }

    report.precision_at_5 = sum_p5 / denom;
    report.precision_at_10 = sum_p10 / denom;
    report.ndcg_at_5 = sum_ndcg5 / denom;
    report.ndcg_at_10 = sum_ndcg10 / denom;
    report.blast_leakage = sum_leak / denom;
    report.over_retrieval = sum_over / denom;
    report.under_retrieval = sum_under / denom;
    report.forbid_hit_rate = sum_forbid / denom;
    report
}

// ── T1.3 metric helpers ─────────────────────────────────────────────

fn membership_test(refs: &[GoldRef], m: &ScoredMemory) -> bool {
    for r in refs {
        match r {
            GoldRef::Memory(id) => {
                if m.id == *id {
                    return true;
                }
            }
            GoldRef::MemoryTag(t) => {
                if m.tag.as_deref() == Some(t.as_str()) {
                    return true;
                }
            }
            GoldRef::File(p) => {
                // Files match memory rows whose content references the
                // path. Trailing `/` matches a directory prefix; bare
                // path is a substring match (fuzziness intentional —
                // memory rows reference files in prose).
                if p.ends_with('/') {
                    if m.content.contains(p) {
                        return true;
                    }
                } else if m.content.contains(p) {
                    return true;
                }
            }
            GoldRef::Symbol(s) => {
                if m.content.contains(s) {
                    return true;
                }
            }
        }
    }
    false
}

fn precision_at_k(case: &EvalCase, pool: &[ScoredMemory], k: usize) -> f32 {
    if case.gold_must.is_empty() && case.gold_neutral.is_empty() {
        return 0.0;
    }
    if k == 0 {
        return 0.0;
    }
    let top = pool.iter().take(k);
    let mut hits = 0usize;
    for m in top {
        if membership_test(&case.gold_must, m) || membership_test(&case.gold_neutral, m) {
            hits += 1;
        }
    }
    hits as f32 / k as f32
}

fn relevance_for(case: &EvalCase, m: &ScoredMemory) -> u8 {
    // Per-item override wins.
    let candidate_keys: Vec<String> = match m.tag.as_deref() {
        Some(t) => vec![
            format!("Memory:{}", m.id),
            format!("MemoryTag:{t}"),
        ],
        None => vec![format!("Memory:{}", m.id)],
    };
    for k in &candidate_keys {
        if let Some(v) = case.graded.get(k) {
            return *v;
        }
    }
    // Defaults from set membership.
    if membership_test(&case.gold_must, m) {
        3
    } else if membership_test(&case.gold_neutral, m) {
        1
    } else {
        0
    }
}

fn ndcg_at_k(case: &EvalCase, pool: &[ScoredMemory], k: usize) -> f32 {
    if case.gold_must.is_empty() && case.gold_neutral.is_empty() && case.graded.is_empty() {
        return 0.0;
    }
    if k == 0 {
        return 0.0;
    }
    let mut dcg = 0.0_f64;
    for (i, m) in pool.iter().take(k).enumerate() {
        let rel = relevance_for(case, m) as f64;
        if rel > 0.0 {
            // log2(i+2) — i is 0-indexed; rank = i+1.
            dcg += (2f64.powf(rel) - 1.0) / ((i as f64 + 2.0).log2());
        }
    }
    // Ideal DCG: sort all relevances in descending order and take top-k.
    let mut rels: Vec<u8> = pool
        .iter()
        .map(|m| relevance_for(case, m))
        .filter(|&r| r > 0)
        .collect();
    rels.sort_by(|a, b| b.cmp(a));
    let mut idcg = 0.0_f64;
    for (i, rel) in rels.iter().take(k).enumerate() {
        let rel = *rel as f64;
        idcg += (2f64.powf(rel) - 1.0) / ((i as f64 + 2.0).log2());
    }
    if idcg <= 0.0 {
        0.0
    } else {
        (dcg / idcg) as f32
    }
}

fn blast_leakage_for(case: &EvalCase, pool: &[ScoredMemory]) -> f32 {
    let Some(target) = case.expected_scope_path.as_deref() else {
        return 0.0;
    };
    if pool.is_empty() {
        return 0.0;
    }
    let parents = parent_scopes(target);
    let leaks = pool
        .iter()
        .filter(|m| !is_within_target_scope(m, &parents))
        .count();
    leaks as f32 / pool.len() as f32
}

/// True when `m` lives at a scope level / path that is a parent (or
/// equal) of the case's `expected_scope_path`.
///
/// - Global / Workspace rows are universally legitimate parents
///   regardless of `scope_path`.
/// - Repo / Module / Run rows must match one of the path-keyed parent
///   prefixes (`parents`, with the empty-string sentinel skipped).
fn is_within_target_scope(m: &ScoredMemory, parents: &[String]) -> bool {
    use crate::memory::scope::{SCOPE_GLOBAL, SCOPE_WORKSPACE};
    if m.scope_level == SCOPE_GLOBAL || m.scope_level == SCOPE_WORKSPACE {
        return true;
    }
    parents
        .iter()
        .filter(|p| !p.is_empty())
        .any(|p| m.scope_path.contains(p.as_str()))
}

fn over_retrieval_for(case: &EvalCase, pool: &[ScoredMemory]) -> f32 {
    if case.gold_must.is_empty() && case.gold_neutral.is_empty() {
        return 0.0;
    }
    if pool.is_empty() {
        return 0.0;
    }
    let outside = pool
        .iter()
        .filter(|m| {
            !membership_test(&case.gold_must, m) && !membership_test(&case.gold_neutral, m)
        })
        .count();
    outside as f32 / pool.len() as f32
}

fn under_retrieval_for(case: &EvalCase, pool: &[ScoredMemory]) -> f32 {
    if case.gold_must.is_empty() {
        return 0.0;
    }
    let missed = case
        .gold_must
        .iter()
        .filter(|r| !pool.iter().any(|m| membership_test(std::slice::from_ref(*r), m)))
        .count();
    missed as f32 / case.gold_must.len() as f32
}

fn forbid_hit_rate_for(case: &EvalCase, pool: &[ScoredMemory]) -> f32 {
    if case.gold_forbid.is_empty() {
        return 0.0;
    }
    if pool.is_empty() {
        return 0.0;
    }
    let hits = pool
        .iter()
        .filter(|m| membership_test(&case.gold_forbid, m))
        .count();
    hits as f32 / pool.len() as f32
}

/// Parent scope chain for blast-leakage: a file path
/// `crates/gaviero-core/src/memory` produces
/// `["crates/gaviero-core/src/memory", "crates/gaviero-core/src",
///   "crates/gaviero-core", ""]`. The walk stops at the two-segment
/// crate-root prefix (`crates/<name>`) so a single bare top segment
/// (`crates`) does not match every `crates/*` path. The empty string
/// matches workspace-level rows whose `scope_path` lacks a folder
/// prefix.
pub fn parent_scopes(path: &str) -> Vec<String> {
    let trimmed = path.trim_end_matches('/');
    let mut out = vec![trimmed.to_string()];
    let mut cur = trimmed;
    while let Some(idx) = cur.rfind('/') {
        let parent = &cur[..idx];
        // Stop ascending once we'd produce a single-segment prefix
        // that has no `/` of its own — that would over-match (e.g.
        // "crates" would falsely accept "crates/other-crate/...").
        if !parent.contains('/') {
            break;
        }
        out.push(parent.to_string());
        cur = parent;
    }
    out.push(String::new());
    out
}


/// T1.3: scope-tightening matrix runner.
///
/// For each scope label in `scopes` (e.g. `["repo", "module", "run"]`),
/// runs the same `cases` against `store` with that scope label
/// substituted into every case's `scope`. Returns one
/// `(scope_label, EvalReport)` pair per scope so callers can compare
/// Precision@K across scope tightening — the central composite-scoring
/// hypothesis ("does narrower scope improve Precision@k?").
///
/// Differs from [`run_live`] in that the report is built with
/// `build_report_with_pools`, so the new T1.3 metrics are populated
/// when cases carry gold sets.
pub async fn run_scope_matrix(
    store: &Arc<MemoryStore>,
    scope_ctx: &MemoryScope,
    cases: &[EvalCase],
    scopes: &[String],
    retrieval_cfg: Option<&RetrievalConfig>,
    reranker: Option<&dyn Reranker>,
    rerank_cfg: Option<&RerankConfig>,
) -> Result<Vec<(String, EvalReport)>> {
    let default_cfg = RetrievalConfig::default();
    let cfg = retrieval_cfg.unwrap_or(&default_cfg);
    let stores = super::stores::MemoryStores::from_single_store(store.clone());
    let mut out = Vec::with_capacity(scopes.len());
    for hint in scopes {
        let mut pools: Vec<Vec<ScoredMemory>> = Vec::with_capacity(cases.len());
        let mut adjusted: Vec<EvalCase> = Vec::with_capacity(cases.len());
        for case in cases {
            let mut adj = case.clone();
            adj.scope = hint.clone();
            let scope = scope_for_eval(&adj.scope, scope_ctx);
            let result = retrieve_ranked(&stores, &scope, &adj.query, 50, cfg, reranker, rerank_cfg)
                .await
                .with_context(|| {
                    format!("retrieving for case {} at scope `{hint}`", adj.id)
                })?;
            pools.push(result.items);
            adjusted.push(adj);
        }
        let report = build_report_with_pools(&adjusted, &pools);
        out.push((hint.clone(), report));
    }
    Ok(out)
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
    // Retain each case's retrieved pool so the report carries the T1.3
    // gold-set metrics (precision/ndcg/leakage) — not just the legacy
    // `expected_memory_id` Recall@K. Code fixtures (`code_prompts.jsonl`)
    // express ground truth via `gold_must`/`gold_neutral` only, so a
    // reranker ablation that scored Recall@K alone would report Δ0; the
    // gold-set metrics are where a rerank change actually shows up.
    let mut pools = Vec::with_capacity(cases.len());
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
        pools.push(out.items);
    }
    Ok(build_report_with_pools(cases, &pools))
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
    let rank = case
        .expected_memory_id
        .and_then(|id| entries.iter().position(|(eid, _)| *eid == id))
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
            expected_memory_id: Some(expected_memory_id),
            scope,
            tags: vec!["bootstrap".to_string()],
            kind: None,
            gold_must: Vec::new(),
            gold_neutral: Vec::new(),
            gold_forbid: Vec::new(),
            graded: BTreeMap::new(),
            expected_scope_path: None,
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

    fn legacy_case(id: &str, expected: i64, tags: &[&str]) -> EvalCase {
        EvalCase {
            id: id.into(),
            query: "q".into(),
            expected_memory_id: Some(expected),
            scope: "repo".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            kind: None,
            gold_must: Vec::new(),
            gold_neutral: Vec::new(),
            gold_forbid: Vec::new(),
            graded: BTreeMap::new(),
            expected_scope_path: None,
        }
    }

    #[test]
    fn outcome_records_rank() {
        let pool = vec![dummy(10), dummy(42), dummy(7)];
        let case = legacy_case("c1", 42, &["foo"]);
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
        let case = legacy_case("c", 99, &[]);
        let o = outcome_from_pool(&case, &pool);
        assert_eq!(o.rank, None);
        assert_eq!(o.reciprocal_rank(), 0.0);
    }

    #[test]
    fn legacy_fixture_loads_with_optional_expected_id() {
        // Pre-T1.3 schema: just (id, query, expected_memory_id, scope, tags).
        let line = r#"{"id":"q-1","query":"how do worktrees clean up","expected_memory_id":4321,"scope":"repo","tags":["worktrees"]}"#;
        let case: EvalCase = serde_json::from_str(line).expect("legacy load");
        assert_eq!(case.expected_memory_id, Some(4321));
        assert!(case.gold_must.is_empty());
        assert!(case.gold_neutral.is_empty());
        assert!(case.gold_forbid.is_empty());
        assert!(case.kind.is_none());
        assert!(case.expected_scope_path.is_none());
    }

    #[test]
    fn case_kind_serializes_snake_case() {
        let case = EvalCase {
            id: "c".into(),
            query: "q".into(),
            expected_memory_id: None,
            scope: "repo".into(),
            tags: vec![],
            kind: Some(CaseKind::Bugfix),
            gold_must: vec![GoldRef::File("crates/foo.rs".into())],
            gold_neutral: vec![],
            gold_forbid: vec![],
            graded: BTreeMap::new(),
            expected_scope_path: Some("crates/foo".into()),
        };
        let s = serde_json::to_string(&case).unwrap();
        assert!(s.contains("\"kind\":\"bugfix\""), "got {s}");
        assert!(s.contains("\"kind\":\"file\""), "got {s}");
        // Round-trip
        let back: EvalCase = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, Some(CaseKind::Bugfix));
    }

    #[test]
    fn parent_scopes_handles_typical_path() {
        let p = parent_scopes("crates/gaviero-core/src/memory");
        assert_eq!(
            p,
            vec![
                "crates/gaviero-core/src/memory".to_string(),
                "crates/gaviero-core/src".to_string(),
                "crates/gaviero-core".to_string(),
                String::new(),
            ]
        );
    }

    #[test]
    fn parent_scopes_handles_root() {
        // A single-segment path emits itself + workspace marker.
        let p = parent_scopes("crates");
        assert_eq!(p, vec!["crates".to_string(), String::new()]);
    }

    #[test]
    fn parent_scopes_two_segment_path_stops_above_self() {
        // Two-segment path is already the crate root: emit self + workspace.
        let p = parent_scopes("crates/foo");
        assert_eq!(p, vec!["crates/foo".to_string(), String::new()]);
    }

    #[test]
    fn precision_at_k_uses_must_or_neutral() {
        let mut a = dummy(1);
        a.content = "memory mentions crates/cache.rs".into();
        let b = dummy(2);
        let c = dummy(3);
        let pool = vec![a, b, c];
        let mut case = legacy_case("p1", 0, &[]);
        case.expected_memory_id = None;
        case.gold_must = vec![GoldRef::File("crates/cache.rs".into())];
        let p = precision_at_k(&case, &pool, 5);
        // 1 hit / 5 = 0.2
        assert!((p - 0.2).abs() < 1e-6, "p={p}");
    }

    #[test]
    fn over_under_retrieval_no_gold_returns_zero() {
        let case = legacy_case("c", 1, &[]);
        let pool = vec![dummy(1), dummy(2)];
        assert_eq!(over_retrieval_for(&case, &pool), 0.0);
        assert_eq!(under_retrieval_for(&case, &pool), 0.0);
    }

    #[test]
    fn under_retrieval_counts_missing_must() {
        let mut hit = dummy(1);
        hit.content = "matches X marker".into();
        let pool = vec![hit];
        let mut case = legacy_case("c", 0, &[]);
        case.gold_must = vec![
            GoldRef::Symbol("X marker".into()),
            GoldRef::Symbol("Y marker".into()),
        ];
        // 1 of 2 must items missing → 0.5
        assert!((under_retrieval_for(&case, &pool) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn forbid_hit_rate_counts_forbidden_in_pool() {
        let mut bad = dummy(1);
        bad.content = "the FORBIDDEN_TOKEN landed here".into();
        let pool = vec![bad, dummy(2), dummy(3)];
        let mut case = legacy_case("c", 0, &[]);
        case.gold_forbid = vec![GoldRef::Symbol("FORBIDDEN_TOKEN".into())];
        // 1 forbid hit / 3 pool = 0.333…
        let f = forbid_hit_rate_for(&case, &pool);
        assert!((f - (1.0 / 3.0)).abs() < 1e-6, "f={f}");
    }

    #[test]
    fn blast_leakage_zero_when_no_expected_scope() {
        let case = legacy_case("c", 1, &[]);
        let pool = vec![dummy(1)];
        assert_eq!(blast_leakage_for(&case, &pool), 0.0);
    }

    #[test]
    fn blast_leakage_flags_outside_parents() {
        let mut a = dummy(1);
        a.scope_path = "module:crates/gaviero-core/src/memory".into();
        let mut b = dummy(2);
        b.scope_path = "module:crates/other/src".into();
        let pool = vec![a, b];
        let mut case = legacy_case("c", 0, &[]);
        case.expected_scope_path = Some("crates/gaviero-core/src/memory".into());
        // a is inside parents (its path contains "crates/gaviero-core/src/memory");
        // b is outside (no parent of the target appears in its scope_path).
        let l = blast_leakage_for(&case, &pool);
        assert!((l - 0.5).abs() < 1e-6, "leak={l}");
    }

    #[test]
    fn t2_code_prompts_corpus_loads_with_expected_distribution() {
        // Tier T2 corpus: 30 hand-graded code prompts. The fixture's
        // structural integrity is asserted here so a typo in a path or
        // a count drift fails fast at `cargo test --lib`.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("eval")
            .join("code_prompts.jsonl");
        let cases = load_fixture(&path).expect("code_prompts.jsonl loads");
        assert_eq!(cases.len(), 30, "corpus must have exactly 30 cases");

        // Distribution: 8 refactor / 7 bugfix / 8 feature / 7 explain.
        let mut by_kind: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for c in &cases {
            let k = match c.kind {
                Some(CaseKind::Refactor) => "refactor",
                Some(CaseKind::Bugfix) => "bugfix",
                Some(CaseKind::Feature) => "feature",
                Some(CaseKind::Explain) => "explain",
                None => "missing",
            };
            *by_kind.entry(k.into()).or_default() += 1;
        }
        assert_eq!(by_kind.get("refactor").copied().unwrap_or(0), 8);
        assert_eq!(by_kind.get("bugfix").copied().unwrap_or(0), 7);
        assert_eq!(by_kind.get("feature").copied().unwrap_or(0), 8);
        assert_eq!(by_kind.get("explain").copied().unwrap_or(0), 7);
        assert_eq!(by_kind.get("missing").copied().unwrap_or(0), 0);

        // Every case must carry at least one gold_must reference.
        for c in &cases {
            assert!(
                !c.gold_must.is_empty(),
                "case {} has empty gold_must",
                c.id
            );
        }
    }

    #[test]
    fn t2_code_prompts_corpus_file_refs_resolve_on_disk() {
        // Repo root: walk up from gaviero-core until we see crates/.
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest
            .parent() // crates/
            .and_then(|p| p.parent()) // workspace root
            .expect("workspace root from manifest");
        let path = manifest.join("eval").join("code_prompts.jsonl");
        let cases = load_fixture(&path).expect("code_prompts.jsonl loads");
        for c in &cases {
            for r in c.gold_must.iter().chain(c.gold_neutral.iter()) {
                if let GoldRef::File(p) = r {
                    let abs = repo_root.join(p);
                    assert!(
                        abs.exists(),
                        "case {}: file ref `{p}` does not exist on disk (looked at {})",
                        c.id,
                        abs.display()
                    );
                }
            }
        }
    }

    #[test]
    fn t2_code_prompts_corpus_symbol_refs_resolve_in_workspace() {
        // For every Symbol gold ref, confirm the identifier appears
        // as a definition site somewhere under crates/. Cheap regex
        // (substring) over the file set — not a full tree-sitter
        // parse, but rejects typos and renames at low cost.
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("eval").join("code_prompts.jsonl");
        let cases = load_fixture(&path).expect("code_prompts.jsonl loads");

        let mut symbols: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for c in &cases {
            for r in c.gold_must.iter().chain(c.gold_neutral.iter()) {
                if let GoldRef::Symbol(s) = r {
                    symbols.insert(s.clone());
                }
            }
        }

        // Slurp every .rs under crates/gaviero-core/src once.
        let crate_src = manifest.join("src");
        let mut haystacks: Vec<String> = Vec::new();
        for entry in walk_rs_files(&crate_src) {
            if let Ok(s) = std::fs::read_to_string(&entry) {
                haystacks.push(s);
            }
        }
        let blob = haystacks.join("\n\n");

        for sym in &symbols {
            // Match a definition site: pub fn / pub struct / pub enum /
            // pub trait / pub const / pub async fn followed by the
            // symbol name. This is corpus-narrow; tighter than a raw
            // substring match because it rejects mere call sites.
            let patterns = [
                format!("pub fn {sym}"),
                format!("pub async fn {sym}"),
                format!("pub struct {sym}"),
                format!("pub enum {sym}"),
                format!("pub trait {sym}"),
                format!("pub const {sym}"),
                format!("fn {sym}"),
                format!("struct {sym}"),
                format!("enum {sym}"),
                format!("trait {sym}"),
            ];
            let found = patterns.iter().any(|p| blob.contains(p));
            assert!(
                found,
                "symbol ref `{sym}` not found as a definition site under crates/gaviero-core/src",
            );
        }
    }

    fn walk_rs_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(root) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    out.extend(walk_rs_files(&p));
                } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
        out
    }

    #[test]
    fn empty_template_fixture_loads_to_zero_cases_with_finite_report() {
        // The checked-in tier1.jsonl is comments-only by design.
        // load_fixture must return an empty Vec; build_report on it
        // must produce a NaN-free zero report.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("eval")
            .join("tier1.jsonl");
        let cases = load_fixture(&path).expect("tier1.jsonl loads");
        assert!(cases.is_empty(), "template should be header-only");
        let r = build_report_with_pools(&cases, &[]);
        assert_eq!(r.total, 0);
        for v in [
            r.recall_at_1,
            r.recall_at_5,
            r.recall_at_10,
            r.mrr,
            r.precision_at_5,
            r.precision_at_10,
            r.ndcg_at_5,
            r.ndcg_at_10,
            r.blast_leakage,
            r.over_retrieval,
            r.under_retrieval,
            r.forbid_hit_rate,
        ] {
            assert!(v.is_finite(), "metric must be finite, got {v}");
        }
    }

    #[test]
    fn build_report_with_pools_zero_cases_is_safe() {
        let r = build_report_with_pools(&[], &[]);
        assert_eq!(r.total, 0);
        assert_eq!(r.recall_at_5, 0.0);
        assert_eq!(r.precision_at_5, 0.0);
        // No NaNs.
        assert!(r.recall_at_5.is_finite());
        assert!(r.ndcg_at_10.is_finite());
        assert!(r.blast_leakage.is_finite());
    }

    #[test]
    fn report_aggregates_recall() {
        let outcomes = vec![
            CaseOutcome {
                id: "a".into(),
                query: "".into(),
                expected_memory_id: Some(1),
                rank: Some(1),
                pool_size: 10,
                tags: vec!["t1".into()],
            },
            CaseOutcome {
                id: "b".into(),
                query: "".into(),
                expected_memory_id: Some(2),
                rank: Some(7),
                pool_size: 10,
                tags: vec!["t1".into()],
            },
            CaseOutcome {
                id: "c".into(),
                query: "".into(),
                expected_memory_id: Some(3),
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
            expected_memory_id: Some(1),
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
