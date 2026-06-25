//! Centralised memory retrieval (Tier S / S1 + Tier B / B2 + B3).
//!
//! Every retrieval surface — chat injection, MCP `memory_search`, the TUI
//! memory panel's search bar, and the eval harness — funnels through
//! [`retrieve_ranked`]. The chat path adds prompt-rendering on top via
//! [`retrieve_for_chat_with_reranker`]; the other surfaces consume the
//! ranked pool directly.
//!
//! Pipeline (single source of truth):
//!
//! 1. Run [`MemoryStore::search_scoped`] with the configured retrieval
//!    mode (`merged` per B3, or `cascade` behind a kill switch).
//! 2. If a reranker is supplied and enabled, score `(query, candidate)`
//!    pairs jointly, **calibrate** the raw logits via sigmoid into
//!    `[0, 1]`, and blend with the composite score (`w * cal + (1-w) *
//!    composite`). Calibration is required so the blend stays in a
//!    comparable range — raw logits would otherwise dominate.
//! 3. Return the ranked top-K plus a per-candidate pool trace
//!    ([`CandidatePoolEntry`]) recording raw + calibrated + blended
//!    scores for the S4 manifest writer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;

use super::scope::{MemoryScope, SCOPE_GLOBAL, SCOPE_MODULE, SCOPE_REPO, SCOPE_WORKSPACE};
use super::scoring::{ScoredMemory, SearchConfig};
use super::stores::MemoryStores;

/// Settings block for chat-turn memory injection.
///
/// Parsed from `.gaviero/settings.json` key `memory.chatInjection`. Defaults
/// match the Tier S plan: Workspace ∪ Repo ∪ Module, Global off, 8 items,
/// 1000-token budget, similarity floor 0.3.
#[derive(Debug, Clone)]
pub struct ChatInjectionConfig {
    /// Master switch. When false, `retrieve_for_chat` returns `None` without
    /// touching the store. Matches `memory.chatInjection.enabled`.
    pub enabled: bool,
    /// Which scope levels to admit into the candidate pool. Items outside
    /// this set are dropped with `exclusion_reason = "scope_filter"`.
    pub scopes: ScopeMix,
    /// Hard cap on memories injected (post-budget trim).
    pub max_items: usize,
    /// Approx token budget for the rendered block (4 chars/token heuristic;
    /// plan allows conservative under-count).
    pub token_budget: usize,
    /// Similarity floor; items below are dropped with
    /// `exclusion_reason = "below_min_similarity"`.
    pub min_similarity: f32,
}

impl Default for ChatInjectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scopes: ScopeMix::default(),
            max_items: 5,
            token_budget: 1000,
            min_similarity: 0.3,
        }
    }
}

/// Which scope levels participate in chat injection.
#[derive(Debug, Clone, Copy)]
pub struct ScopeMix {
    pub workspace: bool,
    pub repo: bool,
    pub module: bool,
    pub global: bool,
}

impl Default for ScopeMix {
    fn default() -> Self {
        Self {
            workspace: true,
            repo: true,
            module: true,
            global: false,
        }
    }
}

impl ScopeMix {
    /// Parse a slice of user-provided scope names. Unknown names are ignored
    /// so adding new scopes later doesn't error-out old settings files.
    pub fn from_names(names: &[String]) -> Self {
        let mut mix = Self {
            workspace: false,
            repo: false,
            module: false,
            global: false,
        };
        let mut seen = HashSet::new();
        for n in names {
            match n.to_ascii_lowercase().as_str() {
                "workspace" => {
                    mix.workspace = true;
                    seen.insert("workspace");
                }
                "repo" => {
                    mix.repo = true;
                    seen.insert("repo");
                }
                "module" => {
                    mix.module = true;
                    seen.insert("module");
                }
                "global" => {
                    mix.global = true;
                    seen.insert("global");
                }
                _ => {}
            }
        }
        if seen.is_empty() {
            Self::default()
        } else {
            mix
        }
    }

    fn admits(&self, scope_level: i32) -> bool {
        match scope_level {
            l if l == SCOPE_GLOBAL => self.global,
            l if l == SCOPE_WORKSPACE => self.workspace,
            l if l == SCOPE_REPO => self.repo,
            l if l == SCOPE_MODULE => self.module,
            _ => false,
        }
    }
}

/// Structured outcome of a chat-turn retrieval. Consumed by the prompt
/// assembler (block + items) and by S4's manifest writer (pool).
#[derive(Debug, Clone)]
pub struct ChatInjection {
    /// Memories actually spliced into the prompt, in injection order.
    pub items: Vec<ScoredMemory>,
    /// Rendered `<project_memory>` block. Empty string when `items` is empty
    /// — callers should still test before splicing.
    pub block: String,
    /// Approximate tokens consumed by `block` (chars / 4).
    pub tokens_used: usize,
    /// Configured budget at the time of retrieval.
    pub token_budget: usize,
    /// Full candidate pool with per-item disposition. Written to the S4
    /// manifest verbatim; callers should treat this as opaque.
    pub pool: Vec<CandidatePoolEntry>,
}

/// One row of the S4 candidate-pool trace.
///
/// B2: when the cross-encoder reranker is enabled and the candidate
/// participated in rerank, three rerank fields are populated:
/// `rerank_score` (raw logit, kept for debugging only), `rerank_calibrated`
/// (sigmoid into `[0, 1]`, what the blend actually used), and
/// `blended_score` (`w * cal + (1 - w) * composite`). Otherwise all
/// three are `None` so manifests stay backward-compatible with v1.
#[derive(Debug, Clone)]
pub struct CandidatePoolEntry {
    pub memory_id: i64,
    pub scope_label: String,
    pub namespace: String,
    pub raw_similarity: f32,
    pub composite_score: f32,
    pub selected: bool,
    pub exclusion_reason: Option<String>,
    /// B2: raw cross-encoder logit. `None` when reranker disabled or
    /// the candidate was excluded before reranking ran.
    pub rerank_score: Option<f32>,
    /// B2: sigmoid-calibrated rerank score in `[0, 1]`. Stored
    /// separately from the raw logit so the manifest is interpretable
    /// across model swaps and the blend math is transparent.
    pub rerank_calibrated: Option<f32>,
    /// B2: `w * rerank_calibrated + (1 - w) * composite`. `None` mirrors
    /// `rerank_score`'s semantics.
    pub blended_score: Option<f32>,
}

/// B3: which retrieval algorithm to use. `Merged` is the default; the
/// legacy `Cascade` path is gated behind `memory.retrieval.mode` for
/// one release as a kill switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalMode {
    /// All scope levels in one parallel pool (B3 default).
    Merged,
    /// Pre-B3 cascade-with-early-exit kill switch.
    Cascade,
}

impl RetrievalMode {
    pub fn parse(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "cascade" => Self::Cascade,
            _ => Self::Merged,
        }
    }
}

/// Settings block for the retrieval engine itself (mode + pool sizing).
///
/// Distinct from [`ChatInjectionConfig`] (which is a chat-only filter
/// layer) and [`super::RerankConfig`] (B2). Resolved from
/// `memory.retrieval.*` at workspace open and threaded into every call.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    pub mode: RetrievalMode,
    /// Per-scope hybrid candidate cap (B3). Caps the pool fed to the
    /// merge stage so a single noisy scope can't dominate.
    pub per_scope_top_k: usize,
    /// Cap on the merged pool *before* rerank (B3). Composite-rank
    /// truncates to this size so the reranker never sees more.
    pub max_merged_pool: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            mode: RetrievalMode::Merged,
            per_scope_top_k: 20,
            max_merged_pool: 50,
        }
    }
}

/// Output of the central [`retrieve_ranked`] engine.
#[derive(Debug, Clone, Default)]
pub struct RetrievalOutput {
    /// Top-K ranked memories after rerank-blend (when applicable).
    pub items: Vec<ScoredMemory>,
    /// Per-candidate trace covering everything inspected by the engine
    /// (including those excluded by the similarity floor or trimmed
    /// after rerank). Used by the S4 manifest writer verbatim.
    pub pool: Vec<CandidatePoolEntry>,
}

/// Central retrieval engine.
///
/// The single entry point used by chat injection, MCP `memory_search`,
/// the TUI memory panel, and the eval harness. Surface-specific
/// filtering (scope mix, min-similarity floor, token budget, prompt
/// rendering) layers on top — never inside this function.
///
/// Returns the ranked pool plus a candidate-pool trace; callers that
/// only need the items can ignore `pool`. On rerank failure (model
/// error, latency budget exceeded), retrieval silently degrades to
/// composite-only ranking — never errors.
pub async fn retrieve_ranked(
    stores: &Arc<MemoryStores>,
    memory_scope: &MemoryScope,
    query: &str,
    limit: usize,
    retrieval_cfg: &RetrievalConfig,
    reranker: Option<&dyn crate::memory::Reranker>,
    rerank_cfg: Option<&crate::memory::RerankConfig>,
) -> Result<RetrievalOutput> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(RetrievalOutput::default());
    }

    // Oversample target: when the reranker is on, we want enough
    // candidates to actually reorder the top-K; otherwise grow modestly
    // beyond `limit` so post-trim doesn't lose good hits.
    let rerank_active = reranker.is_some() && rerank_cfg.map(|c| c.enabled).unwrap_or(false);
    let pool_target = if rerank_active {
        rerank_cfg
            .map(|c| c.pool_size.max(limit))
            .unwrap_or(retrieval_cfg.max_merged_pool)
    } else {
        (limit * 4).max(20)
    };
    let oversample = pool_target.min(retrieval_cfg.max_merged_pool.max(limit));

    let search_cfg = SearchConfig::new(trimmed, memory_scope.clone())
        .with_max_results(oversample)
        .with_per_level_limit(retrieval_cfg.per_scope_top_k);
    let raw = match retrieval_cfg.mode {
        RetrievalMode::Merged => stores.multi_scope_retrieve(&search_cfg).await?,
        #[allow(deprecated)]
        RetrievalMode::Cascade => {
            // Cascade is the legacy single-DB kill-switch. The
            // workspace store is the closest equivalent of the
            // pre-multi-DB behaviour.
            stores
                .workspace()
                .search_scoped_cascade(&search_cfg)
                .await?
        }
    };

    let mut pool: Vec<CandidatePoolEntry> = raw
        .iter()
        .map(|m| CandidatePoolEntry {
            memory_id: m.id,
            scope_label: format_scope_label(m),
            namespace: m.namespace.clone(),
            raw_similarity: m.raw_similarity,
            composite_score: m.final_score,
            selected: true,
            exclusion_reason: None,
            rerank_score: None,
            rerank_calibrated: None,
            blended_score: None,
        })
        .collect();
    let mut kept: Vec<ScoredMemory> = raw;

    // ── B2: rerank stage (calibrated blend) ───────────────────────
    if rerank_active && !kept.is_empty() {
        let cfg = rerank_cfg.expect("rerank_active implies rerank_cfg");
        let pool_size = cfg.pool_size.max(1).min(retrieval_cfg.max_merged_pool);
        kept.truncate(pool_size);

        let texts: Vec<&str> = kept.iter().map(|m| m.content.as_str()).collect();
        let rr = reranker.expect("rerank_active implies reranker");

        let started = std::time::Instant::now();
        let result = rr.rerank(trimmed, &texts).await;
        let elapsed = started.elapsed();

        match result {
            Ok(scores) if scores.len() == kept.len() => {
                if elapsed.as_millis() as u64 > cfg.max_latency_ms {
                    tracing::warn!(
                        target: "memory_rerank",
                        elapsed_ms = elapsed.as_millis() as u64,
                        budget_ms = cfg.max_latency_ms,
                        "rerank exceeded latency budget — applied this turn"
                    );
                }
                let blend_meta =
                    crate::memory::apply_reranker_blend(&mut kept, &scores, cfg.blend_weight);
                let mut by_id: HashMap<i64, (f32, f32, f32)> = HashMap::new();
                for (m, (r, cal, b)) in kept.iter().zip(blend_meta.iter()) {
                    by_id.insert(m.id, (*r, *cal, *b));
                }
                for entry in pool.iter_mut() {
                    if let Some((r, cal, b)) = by_id.get(&entry.memory_id) {
                        entry.rerank_score = Some(*r);
                        entry.rerank_calibrated = Some(*cal);
                        entry.blended_score = Some(*b);
                    }
                }
            }
            Ok(scores) => {
                tracing::warn!(
                    target: "memory_rerank",
                    expected = kept.len(),
                    got = scores.len(),
                    "rerank wrong score count; falling back to composite"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "memory_rerank",
                    error = %e,
                    "rerank failed; falling back to composite"
                );
            }
        }
    }

    // Trim to the caller's requested top-K. Mark trimmed candidates in
    // the pool trace as `selected = false` with reason `top_k_trim` so
    // the manifest reflects exactly what the caller saw.
    if kept.len() > limit {
        kept.truncate(limit);
    }
    let kept_ids: HashSet<i64> = kept.iter().map(|m| m.id).collect();
    for entry in pool.iter_mut() {
        if entry.selected && !kept_ids.contains(&entry.memory_id) {
            entry.selected = false;
            entry.exclusion_reason = Some("top_k_trim".to_string());
        }
    }

    Ok(RetrievalOutput { items: kept, pool })
}

/// Retrieve memory for a single chat turn.
///
/// Returns `None` when injection is disabled or the query is empty. Returns
/// `Some(ChatInjection)` with `items.is_empty()` when the pool has no
/// admissible candidates — callers can distinguish "injection off" from
/// "nothing found" by inspecting `items` / `pool`.
///
/// This is the no-reranker variant kept for backward compatibility.
/// See [`retrieve_for_chat_with_reranker`] for the B2-enabled path.
pub async fn retrieve_for_chat(
    stores: &Arc<MemoryStores>,
    memory_scope: &MemoryScope,
    query: &str,
    config: &ChatInjectionConfig,
) -> Result<Option<ChatInjection>> {
    retrieve_for_chat_with_reranker(
        stores,
        memory_scope,
        query,
        config,
        &RetrievalConfig::default(),
        None,
        None,
    )
    .await
}

/// B2: retrieve memory for chat with an optional reranker.
///
/// Thin wrapper around the central [`retrieve_ranked`] engine that
/// applies the chat-specific filter layer (scope mix, min-similarity,
/// max-items, token budget) and renders the `<project_memory>` block.
pub async fn retrieve_for_chat_with_reranker(
    stores: &Arc<MemoryStores>,
    memory_scope: &MemoryScope,
    query: &str,
    config: &ChatInjectionConfig,
    retrieval_cfg: &RetrievalConfig,
    reranker: Option<&dyn crate::memory::Reranker>,
    rerank_cfg: Option<&crate::memory::RerankConfig>,
) -> Result<Option<ChatInjection>> {
    if !config.enabled {
        return Ok(None);
    }
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // Pull a pool large enough to survive the chat filters; the engine
    // caps at `max_merged_pool` so we never blow up unbounded.
    let engine_limit = (config.max_items * 4).max(20);
    let out = retrieve_ranked(
        stores,
        memory_scope,
        trimmed,
        engine_limit,
        retrieval_cfg,
        reranker,
        rerank_cfg,
    )
    .await?;

    let RetrievalOutput {
        items: ranked,
        mut pool,
    } = out;

    // Chat-side filtering: scope mix, min-similarity floor.
    let mut kept: Vec<ScoredMemory> = Vec::with_capacity(ranked.len());
    let mut filter_reasons: HashMap<i64, &'static str> = HashMap::new();
    for m in ranked {
        if !config.scopes.admits(m.scope_level) {
            filter_reasons.insert(m.id, "scope_filter");
            continue;
        }
        if m.raw_similarity < config.min_similarity {
            filter_reasons.insert(m.id, "below_min_similarity");
            continue;
        }
        kept.push(m);
    }

    // Apply max_items first, then token budget via render_block.
    kept.truncate(config.max_items);
    let (block, items, tokens_used) = render_block(&kept, config.token_budget);

    // Reflect chat filters / budget trims in the pool trace so the
    // S4 manifest matches what the caller actually saw.
    let kept_ids: HashSet<i64> = items.iter().map(|m| m.id).collect();
    for entry in pool.iter_mut() {
        if let Some(reason) = filter_reasons.get(&entry.memory_id) {
            entry.selected = false;
            entry.exclusion_reason = Some((*reason).to_string());
        } else if entry.selected && !kept_ids.contains(&entry.memory_id) {
            entry.selected = false;
            entry.exclusion_reason = Some("token_budget".to_string());
        }
    }

    Ok(Some(ChatInjection {
        items,
        block,
        tokens_used,
        token_budget: config.token_budget,
        pool,
    }))
}

/// Render the `<project_memory>` block, stopping once the token budget is
/// exhausted. Returns the block text, the items actually emitted, and the
/// approximate tokens consumed.
fn render_block(kept: &[ScoredMemory], token_budget: usize) -> (String, Vec<ScoredMemory>, usize) {
    if kept.is_empty() {
        return (String::new(), Vec::new(), 0);
    }
    const CHARS_PER_TOKEN: usize = 4;
    let char_budget = token_budget.saturating_mul(CHARS_PER_TOKEN);

    let header = "<project_memory>\n";
    let footer = "</project_memory>";
    let mut body = String::new();
    let mut emitted: Vec<ScoredMemory> = Vec::new();
    let overhead = header.len() + footer.len();

    for m in kept {
        let line = format!(
            "- [{}] {}: {}\n",
            format_scope_label(m),
            m.memory_type.as_str(),
            m.content.trim()
        );
        if overhead + body.len() + line.len() > char_budget && !emitted.is_empty() {
            break;
        }
        body.push_str(&line);
        emitted.push(m.clone());
    }

    if emitted.is_empty() {
        return (String::new(), Vec::new(), 0);
    }

    let mut out = String::with_capacity(overhead + body.len());
    out.push_str(header);
    out.push_str(&body);
    out.push_str(footer);
    let tokens_used = out.len().div_ceil(CHARS_PER_TOKEN);
    (out, emitted, tokens_used)
}

fn format_scope_label(m: &ScoredMemory) -> String {
    match m.scope_level {
        SCOPE_GLOBAL => "global".to_string(),
        SCOPE_WORKSPACE => "workspace".to_string(),
        SCOPE_REPO => "repo".to_string(),
        SCOPE_MODULE => m
            .module_path
            .clone()
            .map(|p| format!("module:{p}"))
            .unwrap_or_else(|| "module".to_string()),
        _ => m.scope_path.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_mix_defaults_cover_plan_recommendation() {
        let m = ScopeMix::default();
        assert!(m.workspace && m.repo && m.module);
        assert!(!m.global, "plan defaults Global off");
    }

    #[test]
    fn scope_mix_from_names_parses_known_values() {
        let m = ScopeMix::from_names(&["repo".to_string(), "Global".to_string()]);
        assert!(m.repo && m.global);
        assert!(!m.workspace && !m.module);
    }

    #[test]
    fn scope_mix_from_empty_returns_defaults() {
        let m = ScopeMix::from_names(&[]);
        let d = ScopeMix::default();
        assert_eq!(m.workspace, d.workspace);
        assert_eq!(m.global, d.global);
    }

    #[test]
    fn render_block_empty_when_no_items() {
        let (block, items, tokens) = render_block(&[], 1000);
        assert!(block.is_empty());
        assert!(items.is_empty());
        assert_eq!(tokens, 0);
    }

    #[test]
    fn retrieval_mode_parses_known_aliases() {
        assert!(matches!(
            RetrievalMode::parse("merged"),
            RetrievalMode::Merged
        ));
        assert!(matches!(
            RetrievalMode::parse("MERGED"),
            RetrievalMode::Merged
        ));
        assert!(matches!(
            RetrievalMode::parse("cascade"),
            RetrievalMode::Cascade
        ));
        // Anything else falls back to Merged so a typo never silently
        // re-enables the kill switch.
        assert!(matches!(RetrievalMode::parse("foo"), RetrievalMode::Merged));
        assert!(matches!(RetrievalMode::parse(""), RetrievalMode::Merged));
    }

    #[test]
    fn retrieval_config_defaults_match_plan() {
        let c = RetrievalConfig::default();
        assert!(matches!(c.mode, RetrievalMode::Merged));
        assert_eq!(c.per_scope_top_k, 20);
        assert_eq!(c.max_merged_pool, 50);
    }
}
