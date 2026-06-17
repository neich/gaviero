//! Tier B / B2: Cross-encoder reranker stage for retrieval.
//!
//! Inserts a rerank step between hybrid retrieval and the final ranking.
//! A cross-encoder jointly scores `(query, candidate.text)` pairs and
//! returns a per-candidate relevance score; the caller blends it with
//! the existing composite score (default `0.6 * rerank + 0.4 * composite`).
//!
//! Disabled by default until the B2f ablation gate confirms gain. When
//! the model file is missing or the configured tier is `"none"`,
//! [`build_reranker`] returns `None` and retrieval falls back silently
//! to composite-only ranking.

use anyhow::Result;
use async_trait::async_trait;
use ndarray::Array2;
use ort::session::Session;
use ort::value::TensorRef;
use std::sync::Arc;
use std::sync::Mutex;
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

use super::model_manager::{ModelInfo, ModelManager};

/// Cap on tokenized pair length. Cross-encoders score `[CLS] query
/// [SEP] doc [SEP]`; 512 covers the modernbert reranker's window
/// comfortably without OOM risk.
const MAX_PAIR_TOKENS: usize = 512;

/// Sub-batch size for reranker ONNX inference. The candidate pool (up to
/// `RerankConfig::pool_size`, default 50) is scored in chunks of this many
/// `(query, candidate)` pairs, capping the worst-case resident activation
/// set at ~`RERANK_SUB_BATCH` sequences instead of the whole pool.
///
/// This is a **secondary** bound. The primary OOM cause was the ModernBERT
/// tokenizer's fixed 8000-token padding — disabled in
/// [`ModernBertReranker::load`]; once real sequence length is capped at
/// `MAX_PAIR_TOKENS`, a full `50 × 512` batch fits comfortably. Chunking
/// just keeps the peak small on memory-constrained workstations. Cross-encoder
/// logits are per-pair independent, so chunking changes neither the scores
/// nor their order.
const RERANK_SUB_BATCH: usize = 8;

/// Apply the bounded-length tokenizer config the reranker relies on for
/// memory safety: truncate every `(query, candidate)` pair to
/// `MAX_PAIR_TOKENS`, and **disable the tokenizer's own padding**.
///
/// The shipped gte-reranker-modernbert `tokenizer.json` pins padding to a
/// *fixed* 8000 tokens (`padding.strategy = Fixed(8000)`). Left in place,
/// `encode_batch` re-pads every pair back up to 8000 even after truncation
/// caps the real tokens at `MAX_PAIR_TOKENS`; ModernBERT then runs attention
/// at seq=8000 (O(seq²)), spiking ONNX activation memory hard enough to OOM
/// the process. `run_inference` builds the rectangular input tensor and
/// zero-pads to the batch max itself, so the tokenizer must not pad.
fn configure_reranker_tokenizer(tokenizer: &mut Tokenizer) -> Result<()> {
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: MAX_PAIR_TOKENS,
            strategy: TruncationStrategy::LongestFirst,
            stride: 0,
            direction: TruncationDirection::Right,
        }))
        .map_err(|e| anyhow::anyhow!("configuring reranker tokenizer truncation: {e}"))?;
    tokenizer.with_padding(None);
    Ok(())
}

/// Trait for query/candidate cross-encoder rerankers.
///
/// Implementations must be `Send + Sync`. Reranking runs **outside**
/// the SQLite mutex; the caller passes already-materialized text.
#[async_trait]
pub trait Reranker: Send + Sync {
    /// Stable model identifier used for telemetry and manifest provenance.
    fn name(&self) -> &str {
        self.model_id()
    }

    /// Maximum tokenized pair length accepted before truncation.
    fn max_tokens(&self) -> usize {
        MAX_PAIR_TOKENS
    }

    /// Score each `(query, candidate)` pair on a comparable scale.
    /// Returns one f32 per candidate, in the same order as the input.
    async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>>;

    /// Stable model identifier for telemetry and manifest provenance.
    fn model_id(&self) -> &str;

    /// Run a single dummy `(query, candidate)` pair through the model
    /// to amortise the first-load cost (~200ms for ONNX session setup
    /// + tokenizer warmup) at workspace-open time rather than on the
    /// first real query. Errors are not fatal — the call site logs
    /// them and continues; rerank itself is the only mandatory path.
    async fn warmup(&self) -> Result<()> {
        let _ = self.rerank("warmup", &["warmup"]).await?;
        Ok(())
    }
}

/// Deterministic testing/fallback reranker.
///
/// It preserves the input ordering by returning equal scores for every candidate.
#[derive(Debug, Default, Clone)]
pub struct NullReranker;

#[async_trait]
impl Reranker for NullReranker {
    fn name(&self) -> &str {
        "null"
    }

    async fn rerank(&self, _query: &str, candidates: &[&str]) -> Result<Vec<f32>> {
        Ok(vec![0.0; candidates.len()])
    }

    fn model_id(&self) -> &str {
        "null"
    }

    async fn warmup(&self) -> Result<()> {
        Ok(())
    }
}

/// Configuration for the rerank stage.
#[derive(Debug, Clone)]
pub struct RerankConfig {
    pub enabled: bool,
    /// Maximum number of candidates submitted to the reranker per
    /// query. Anything beyond is truncated by composite rank before
    /// rerank — the reranker is the *expensive* stage.
    pub pool_size: usize,
    /// Weight placed on the rerank score in the final blend
    /// (`final = w * rerank + (1 - w) * composite`). 0.0 = pure
    /// composite, 1.0 = pure rerank.
    pub blend_weight: f32,
    /// Advisory soft latency budget in ms. This is **not** a kill-switch:
    /// exceeding it logs a `memory_rerank` warning ("applied this turn")
    /// and the rerank blend is applied anyway for that query, with no
    /// effect on subsequent queries (see `retrieval.rs`). Tune the
    /// `enabled` flip against measured p95 latency, since there is no
    /// runtime fallback once rerank is on.
    pub max_latency_ms: u64,
    /// Intra-op thread count for the reranker's ONNX session. `0` = auto
    /// (the machine's available parallelism, clamped to
    /// `RERANK_MAX_AUTO_THREADS`). The cross-encoder is the
    /// latency-dominant retrieval stage; on a multi-core CPU the historical
    /// single-thread session left most of the speedup on the table. This
    /// knob is reranker-only — embedder inference stays single-threaded.
    pub threads: usize,
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // Int8 + multi-thread make a wider pool affordable, but a
            // smaller pool is still the cheapest latency lever; 20 keeps
            // enough candidates for the top-k blend to reorder.
            pool_size: 20,
            blend_weight: 0.6,
            max_latency_ms: 200,
            threads: 0,
        }
    }
}

/// Calibrate a raw cross-encoder logit into `[0, 1]` via the logistic
/// sigmoid. Composite scores already live near `[0, 1]`; calibrating
/// the rerank score makes the blend in [`blend_rerank`] dimensionally
/// honest, preserving (rather than washing out) scope/trust multipliers
/// applied earlier.
///
/// Non-finite logits fall through to `0.0` — a misbehaving reranker
/// then has zero blend weight in `blend_rerank` and the composite score
/// dominates.
pub fn sigmoid_calibrate(logit: f32) -> f32 {
    if !logit.is_finite() {
        return 0.0;
    }
    // Saturate exponent to keep the math stable without branching.
    let z = logit.clamp(-30.0, 30.0);
    1.0 / (1.0 + (-z).exp())
}

/// Final-blend the rerank score with the existing composite score.
///
/// **The first argument must be the calibrated rerank score** (produced
/// by [`sigmoid_calibrate`]) — `blend_rerank` no longer normalises
/// internally. Blending raw logits would let the rerank dominate by
/// magnitude (logits are in roughly `[-10, 10]`, composites in `[0,
/// 1]`), erasing scope/trust semantics. Callers should always go
/// through [`apply_reranker_blend`] which sigmoids before calling this.
///
/// `w` is clamped to `[0.0, 1.0]`. NaN / infinite rerank scores fall
/// through to the composite-only result so a misbehaving reranker
/// never poisons the ordering.
pub fn blend_rerank(rerank_calibrated: f32, composite: f32, w: f32) -> f32 {
    if !rerank_calibrated.is_finite() {
        return composite;
    }
    let w = w.clamp(0.0, 1.0);
    w * rerank_calibrated + (1.0 - w) * composite
}

/// Tier B / B2: Alibaba-NLP/gte-reranker-modernbert-base. Apache-2.0,
/// ~150M params. Same tokenizer family as `gte-modernbert-base`.
pub const GTE_RERANKER_MODERNBERT_BASE: ModelInfo = ModelInfo {
    id: "gte-reranker-modernbert-base",
    onnx_url: "https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/resolve/main/tokenizer.json",
    dimensions: 1, // single-logit output, not an embedding
};

/// Int8-quantized gte-reranker-modernbert (`onnx/model_int8.onnx`). Same
/// tokenizer and single-logit output as the fp32 variant, ~4× smaller on
/// disk and markedly faster on CPU at a small accuracy cost. A distinct
/// `id` gives it a separate cache dir, so it downloads alongside — not
/// over — the fp32 model. The quality-ceiling option; ~4 s/query on CPU
/// (thread-capped ~8 intra-op), so prefer [`MS_MARCO_MINILM_L6_V2_INT8`]
/// (the default) for always-on use, and this one at chat frequency / GPU.
pub const GTE_RERANKER_MODERNBERT_BASE_INT8: ModelInfo = ModelInfo {
    id: "gte-reranker-modernbert-base-int8",
    onnx_url: "https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/resolve/main/onnx/model_int8.onnx",
    tokenizer_url: "https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/resolve/main/tokenizer.json",
    dimensions: 1,
};

/// Int8 ms-marco-MiniLM-L6-v2 cross-encoder (Xenova ONNX export). ~22M
/// params — an order of magnitude smaller than ModernBERT-150M — with a
/// BERT WordPiece tokenizer and the same single-logit relevance output.
/// **The shipped reranker default**: on the B2f code gold set it matched
/// the int8 ModernBERT on ndcg@5 to within noise (+0.261 vs +0.270) at
/// ~8× lower latency (~540 ms vs ~4 s/query, pool 20, 8 threads), making
/// it the CPU-viable always-on choice. Caveat: ms-marco is web-passage
/// trained — validate transfer on non-code corpora before relying on it.
pub const MS_MARCO_MINILM_L6_V2_INT8: ModelInfo = ModelInfo {
    id: "ms-marco-minilm-l6-v2-int8",
    onnx_url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/onnx/model_int8.onnx",
    tokenizer_url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer.json",
    dimensions: 1,
};

/// Cap on the auto-resolved intra-op thread count (`threads = 0`). Beyond
/// roughly the physical P-core count, ModernBERT reranking sees
/// diminishing — sometimes negative — returns as it spills onto SMT
/// siblings / E-cores, so auto stops here. Explicit settings may exceed it.
const RERANK_MAX_AUTO_THREADS: usize = 8;

/// Resolve a configured reranker thread count to a concrete intra-op
/// value. `0` means "auto": the machine's available parallelism, clamped
/// to `[1, RERANK_MAX_AUTO_THREADS]`. Any explicit `>= 1` is honoured
/// verbatim so an operator can push past the auto cap on a big box.
fn resolve_intra_threads(configured: usize) -> usize {
    if configured >= 1 {
        return configured;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, RERANK_MAX_AUTO_THREADS)
}

/// Resolve a settings string to a reranker model. `"none"` (or unknown)
/// returns `None`, which the factory treats as "rerank disabled".
pub fn resolve_reranker_model(name: &str) -> Option<&'static ModelInfo> {
    match name.trim().to_ascii_lowercase().as_str() {
        "" | "none" => None,
        "gte-reranker-modernbert" | "gte-reranker-modernbert-base" => {
            Some(&GTE_RERANKER_MODERNBERT_BASE)
        }
        "gte-reranker-modernbert-int8" | "gte-reranker-modernbert-base-int8" => {
            Some(&GTE_RERANKER_MODERNBERT_BASE_INT8)
        }
        "minilm" | "ms-marco-minilm" | "ms-marco-minilm-l6" | "ms-marco-minilm-l6-v2-int8" => {
            Some(&MS_MARCO_MINILM_L6_V2_INT8)
        }
        _ => None,
    }
}

/// Build a reranker from a settings string. Returns `Ok(None)` when
/// rerank is disabled / unknown so callers cleanly degrade to
/// composite-only ranking. Errors are reserved for "model resolved but
/// could not be loaded".
/// `threads` is the intra-op thread count for the ONNX session (`0` =
/// auto; see [`RerankConfig::threads`] / [`resolve_intra_threads`]).
pub fn build_reranker(name: &str, threads: usize) -> Result<Option<Arc<dyn Reranker>>> {
    match name.trim().to_ascii_lowercase().as_str() {
        "null" => Ok(Some(Arc::new(NullReranker) as Arc<dyn Reranker>)),
        _ => match resolve_reranker_model(name) {
            Some(info) => Ok(Some(
                Arc::new(ModernBertReranker::from_model(info, threads)?) as Arc<dyn Reranker>
            )),
            None => Ok(None),
        },
    }
}

/// ONNX cross-encoder reranker. The model produces a single logit per
/// `(query, candidate)` pair which we use directly as the relevance
/// score (logit-space; higher = more relevant). Sigmoid is unnecessary
/// because we only care about ordering.
///
/// The session and tokenizer live behind an `Arc<Inner>` so `rerank`
/// can hand a clone to `spawn_blocking` and run the synchronous ONNX
/// call on a blocking thread without holding any lock across an
/// `await` or stalling the tokio executor (CLAUDE.md lock discipline).
pub struct ModernBertReranker {
    inner: Arc<RerankerInner>,
    model_id: String,
}

struct RerankerInner {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl ModernBertReranker {
    pub fn from_model(model: &ModelInfo, threads: usize) -> Result<Self> {
        let manager = ModelManager::new();
        manager.ensure_downloaded(model)?;
        Self::load(
            &manager.onnx_path(model),
            &manager.tokenizer_path(model),
            model.id,
            threads,
        )
    }

    pub fn load(
        onnx_path: &std::path::Path,
        tokenizer_path: &std::path::Path,
        model_id: &str,
        threads: usize,
    ) -> Result<Self> {
        let intra_threads = resolve_intra_threads(threads);
        let session = Session::builder()
            .map_err(|e| anyhow::anyhow!("creating ONNX session builder: {e}"))?
            .with_intra_threads(intra_threads)
            .map_err(|e| anyhow::anyhow!("setting thread count: {e}"))?
            .commit_from_file(onnx_path)
            .map_err(|e| {
                anyhow::anyhow!("loading reranker model from {}: {e}", onnx_path.display())
            })?;

        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading reranker tokenizer: {e}"))?;
        configure_reranker_tokenizer(&mut tokenizer)?;

        Ok(Self {
            inner: Arc::new(RerankerInner {
                session: Mutex::new(session),
                tokenizer,
            }),
            model_id: model_id.to_string(),
        })
    }

}

impl RerankerInner {
    /// Run inference on a batch of `(query, candidate)` pairs and return
    /// one logit per pair. Inputs are paired via the tokenizer's
    /// `encode_batch` with `add_special_tokens = true`.
    fn run_inference(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Build (query, candidate) pairs. Tokenizer handles the special-token
        // assembly: `[CLS] query [SEP] candidate [SEP]`.
        let pairs: Vec<(String, String)> = candidates
            .iter()
            .map(|c| (query.to_string(), c.to_string()))
            .collect();

        let encodings = self
            .tokenizer
            .encode_batch(pairs, true)
            .map_err(|e| anyhow::anyhow!("reranker tokenization failed: {e}"))?;

        // Score in fixed-size sub-batches, reusing one session arena across
        // chunks. This caps the worst-case resident set (see RERANK_SUB_BATCH);
        // the actual OOM fix is disabling the tokenizer's fixed 8000-token
        // padding in `load`. Cross-encoder logits are per-pair independent, so
        // chunking yields the same scores and order as one big batch.
        let mut session = self
            .session
            .lock()
            .map_err(|e| anyhow::anyhow!("reranker session lock poisoned: {e}"))?;
        let needs_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");

        let mut scores = Vec::with_capacity(encodings.len());
        for chunk in encodings.chunks(RERANK_SUB_BATCH) {
            let batch = chunk.len();
            // Pad only to the longest sequence in *this* chunk, not the
            // whole pool — a chunk of short docs stays cheap.
            let max_len = chunk
                .iter()
                .map(|e| e.get_ids().len())
                .max()
                .unwrap_or(0);
            let mut input_ids = Array2::<i64>::zeros((batch, max_len));
            let mut attention_mask = Array2::<i64>::zeros((batch, max_len));
            let mut token_type_ids = Array2::<i64>::zeros((batch, max_len));
            for (i, enc) in chunk.iter().enumerate() {
                for (j, ((&id, &m), &ty)) in enc
                    .get_ids()
                    .iter()
                    .zip(enc.get_attention_mask().iter())
                    .zip(enc.get_type_ids().iter())
                    .enumerate()
                {
                    input_ids[[i, j]] = id as i64;
                    attention_mask[[i, j]] = m as i64;
                    token_type_ids[[i, j]] = ty as i64;
                }
            }

            let ids_t = TensorRef::from_array_view(&input_ids)
                .map_err(|e| anyhow::anyhow!("creating input_ids tensor: {e}"))?;
            let mask_t = TensorRef::from_array_view(&attention_mask)
                .map_err(|e| anyhow::anyhow!("creating attention_mask tensor: {e}"))?;
            let ty_t = TensorRef::from_array_view(&token_type_ids)
                .map_err(|e| anyhow::anyhow!("creating token_type_ids tensor: {e}"))?;

            let outputs = if needs_token_type_ids {
                session.run(ort::inputs![
                    "input_ids" => ids_t,
                    "attention_mask" => mask_t,
                    "token_type_ids" => ty_t,
                ])
            } else {
                session.run(ort::inputs![
                    "input_ids" => ids_t,
                    "attention_mask" => mask_t,
                ])
            }
            .map_err(|e| anyhow::anyhow!("reranker inference failed: {e}"))?;

            // Output shape: [batch, 1] (single logit) or [batch, 2]
            // (binary classifier — relevance is logit[1] - logit[0]).
            let arr = outputs[0]
                .try_extract_array::<f32>()
                .map_err(|e| anyhow::anyhow!("extracting reranker output: {e}"))?;
            let shape = arr.shape();
            match shape.len() {
                2 if shape[1] == 1 => {
                    for i in 0..batch {
                        scores.push(arr[[i, 0]]);
                    }
                }
                2 if shape[1] >= 2 => {
                    for i in 0..batch {
                        scores.push(arr[[i, 1]] - arr[[i, 0]]);
                    }
                }
                _ => anyhow::bail!(
                    "unexpected reranker output shape: {:?} (expected [batch, 1] or [batch, 2])",
                    shape
                ),
            }
        }
        Ok(scores)
    }
}

#[async_trait]
impl Reranker for ModernBertReranker {
    fn name(&self) -> &str {
        &self.model_id
    }

    async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>> {
        // ONNX inference is CPU-bound and the session lives behind a
        // sync `Mutex`. Hand a clone of the inner `Arc` to a blocking
        // thread so we never hold the lock across an `await` and never
        // park a tokio worker on synchronous CPU work.
        let inner = Arc::clone(&self.inner);
        let q = query.to_string();
        let cs: Vec<String> = candidates.iter().map(|s| s.to_string()).collect();
        tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = cs.iter().map(String::as_str).collect();
            inner.run_inference(&q, &refs)
        })
        .await
        .map_err(|e| anyhow::anyhow!("rerank join: {e}"))?
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// Apply rerank scores to a candidate pool: sigmoid-calibrate each raw
/// logit, blend with the composite score, and re-sort by the blended
/// value. Modifies `pool` in-place. Returns one
/// `(raw_logit, calibrated, blended)` triple per candidate in the new
/// ordering — used by the manifest writer to record exactly what the
/// blend stage saw.
pub fn apply_reranker_blend(
    pool: &mut [super::ScoredMemory],
    rerank_scores: &[f32],
    blend_weight: f32,
) -> Vec<(f32, f32, f32)> {
    debug_assert_eq!(pool.len(), rerank_scores.len());
    let mut blended: Vec<(usize, f32, f32, f32)> = pool
        .iter()
        .zip(rerank_scores.iter())
        .enumerate()
        .map(|(i, (m, &r))| {
            let cal = sigmoid_calibrate(r);
            let b = blend_rerank(cal, m.final_score, blend_weight);
            (i, r, cal, b)
        })
        .collect();
    blended.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

    let new_order: Vec<usize> = blended.iter().map(|(i, _, _, _)| *i).collect();
    let new_meta: Vec<(f32, f32, f32)> = blended.iter().map(|(_, r, c, b)| (*r, *c, *b)).collect();

    let original: Vec<super::ScoredMemory> = pool.to_vec();
    for (dest, src) in new_order.iter().enumerate() {
        pool[dest] = original[*src].clone();
    }
    for (i, (_, _, blended)) in new_meta.iter().enumerate() {
        pool[i].final_score = *blended;
    }
    new_meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_pure_rerank_when_w_is_1() {
        // First arg is the calibrated rerank score (already in [0,1]).
        assert_eq!(blend_rerank(0.8, 0.3, 1.0), 0.8);
    }

    #[test]
    fn blend_pure_composite_when_w_is_0() {
        assert_eq!(blend_rerank(0.8, 0.3, 0.0), 0.3);
    }

    #[test]
    fn blend_clamps_w() {
        // w outside [0,1] is clamped, so the blend is always sane.
        assert_eq!(blend_rerank(1.0, 0.0, -1.0), 0.0);
        assert_eq!(blend_rerank(1.0, 0.0, 2.0), 1.0);
    }

    #[test]
    fn blend_falls_through_on_non_finite_rerank() {
        assert_eq!(blend_rerank(f32::NAN, 0.42, 0.6), 0.42);
        assert_eq!(blend_rerank(f32::INFINITY, 0.42, 0.6), 0.42);
    }

    #[test]
    fn sigmoid_calibrate_is_bounded_and_monotone() {
        let lo = sigmoid_calibrate(-10.0);
        let mid = sigmoid_calibrate(0.0);
        let hi = sigmoid_calibrate(10.0);
        assert!(lo > 0.0 && lo < 0.001);
        assert!((mid - 0.5).abs() < 1e-6);
        assert!(hi > 0.999 && hi < 1.0);
        assert_eq!(sigmoid_calibrate(f32::NAN), 0.0);
    }

    #[test]
    fn calibration_keeps_blend_in_unit_range() {
        // Even a 10x logit can't dominate a composite of 0.5 with w=0.6
        // — calibrated to ~1, blend tops out at 0.6 + 0.4*0.5 = 0.8.
        let cal = sigmoid_calibrate(10.0);
        let blended = blend_rerank(cal, 0.5, 0.6);
        assert!(blended <= 1.0);
        assert!(blended > 0.5, "rerank must still pull above composite");
    }

    #[test]
    fn resolve_reranker_model_handles_aliases() {
        assert!(resolve_reranker_model("none").is_none());
        assert!(resolve_reranker_model("").is_none());
        assert!(resolve_reranker_model("unknown-thing").is_none());
        assert!(resolve_reranker_model("gte-reranker-modernbert").is_some());
        assert!(resolve_reranker_model("gte-reranker-modernbert-base").is_some());
    }

    #[test]
    fn resolve_reranker_model_handles_int8_and_minilm_aliases() {
        assert_eq!(
            resolve_reranker_model("gte-reranker-modernbert-int8").unwrap().id,
            "gte-reranker-modernbert-base-int8"
        );
        assert_eq!(
            resolve_reranker_model("minilm").unwrap().id,
            "ms-marco-minilm-l6-v2-int8"
        );
        assert_eq!(
            resolve_reranker_model("ms-marco-minilm-l6").unwrap().id,
            "ms-marco-minilm-l6-v2-int8"
        );
    }

    #[test]
    fn resolve_intra_threads_honours_explicit_and_caps_auto() {
        // Explicit values pass through untouched (even past the auto cap).
        assert_eq!(resolve_intra_threads(1), 1);
        assert_eq!(resolve_intra_threads(16), 16);
        // Auto (0) is clamped to [1, RERANK_MAX_AUTO_THREADS].
        let auto = resolve_intra_threads(0);
        assert!((1..=RERANK_MAX_AUTO_THREADS).contains(&auto), "auto={auto}");
    }

    #[tokio::test]
    async fn null_reranker_preserves_candidate_count() {
        let reranker = NullReranker;
        let scores = reranker.rerank("q", &["a", "b", "c"]).await.unwrap();
        assert_eq!(scores, vec![0.0, 0.0, 0.0]);
        reranker.warmup().await.unwrap();
    }

    #[test]
    fn build_reranker_accepts_null_and_none() {
        assert!(build_reranker("none", 0).unwrap().is_none());
        let rr = build_reranker("null", 0).unwrap().expect("null reranker");
        assert_eq!(rr.model_id(), "null");
    }

    /// Regression guard for the fixed-padding OOM: the gte-reranker-modernbert
    /// tokenizer ships `padding.strategy = Fixed(8000)`, which would re-pad
    /// every pair to 8000 tokens after truncation and OOM the ONNX forward.
    /// [`configure_reranker_tokenizer`] must disable padding so a huge
    /// candidate still encodes to `<= MAX_PAIR_TOKENS`. Requires the model to
    /// be downloaded (skips cleanly otherwise).
    #[test]
    #[ignore = "requires downloaded gte-reranker-modernbert tokenizer"]
    fn reranker_tokenizer_caps_pair_length() {
        let manager = ModelManager::new();
        let path = manager.tokenizer_path(&GTE_RERANKER_MODERNBERT_BASE);
        if !path.exists() {
            eprintln!("skip: tokenizer not downloaded at {}", path.display());
            return;
        }
        let mut tokenizer = Tokenizer::from_file(&path).expect("load tokenizer");
        configure_reranker_tokenizer(&mut tokenizer).expect("configure");

        let huge = "lorem ipsum dolor sit amet ".repeat(5000);
        let enc = tokenizer
            .encode(("what is the write gate", huge.as_str()), true)
            .expect("encode pair");
        assert!(
            enc.get_ids().len() <= MAX_PAIR_TOKENS,
            "pair encoded to {} tokens; padding not disabled (expected <= {MAX_PAIR_TOKENS})",
            enc.get_ids().len(),
        );
    }
}
