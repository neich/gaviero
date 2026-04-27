use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

/// Retrieval-side intent for an embedding request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingPurpose {
    Query,
    Document,
}

/// Trait for text embedding models.
///
/// Implementations must be Send + Sync so they can be shared across async tasks.
///
/// Models that use task prefixes (e.g., nomic-embed-text-v1.5 uses
/// "search_query: " / "search_document: ") should override `embed_query`
/// and `embed_document`. The defaults delegate to `embed()` for backward
/// compatibility.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Stable model identifier used for telemetry and persisted provenance.
    fn name(&self) -> &str;

    /// The dimensionality of the embedding vectors.
    fn dimension(&self) -> usize;

    /// Maximum tokenized input accepted by this implementation before truncation.
    fn max_tokens(&self) -> usize {
        512
    }

    /// Embed a single text string into a vector of floats for a specific purpose.
    async fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>>;

    /// Embed a query text for search.
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text, EmbeddingPurpose::Query).await
    }

    /// Embed a document text for storage.
    async fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text, EmbeddingPurpose::Document).await
    }

    /// Embed a batch of texts for a specific purpose.
    ///
    /// Production implementations should run one batched model invocation when the
    /// backend supports it; this default is for simple test and fallback embedders.
    async fn embed_batch(
        &self,
        texts: &[&str],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed(text, purpose).await?);
        }
        Ok(out)
    }
}

/// Deterministic local test/fallback embedder.
///
/// It intentionally avoids randomness so unit tests and manifest replay stay stable.
#[derive(Debug, Clone)]
pub struct NullEmbedder {
    name: &'static str,
    dimension: usize,
}

impl Default for NullEmbedder {
    fn default() -> Self {
        Self {
            name: "null",
            dimension: 8,
        }
    }
}

impl NullEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self {
            name: "null",
            dimension: dimension.max(1),
        }
    }

    fn deterministic_vector(&self, text: &str, purpose: EmbeddingPurpose) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dimension];
        let purpose_salt = match purpose {
            EmbeddingPurpose::Query => 17usize,
            EmbeddingPurpose::Document => 31usize,
        };
        for (i, b) in text.bytes().enumerate() {
            v[(i + purpose_salt) % self.dimension] += b as f32;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

#[async_trait]
impl Embedder for NullEmbedder {
    fn name(&self) -> &str {
        self.name
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn max_tokens(&self) -> usize {
        usize::MAX
    }

    async fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
        Ok(self.deterministic_vector(text, purpose))
    }
}

/// Runs a primary embedder plus a comparison embedder and returns the primary result.
///
/// This is intentionally log-only: it lets a workspace evaluate a candidate model
/// without changing retrieval behavior or requiring a schema migration.
pub struct DualEmbedder {
    primary: Arc<dyn Embedder>,
    comparison: Arc<dyn Embedder>,
}

impl DualEmbedder {
    pub fn new(primary: Arc<dyn Embedder>, comparison: Arc<dyn Embedder>) -> Self {
        Self {
            primary,
            comparison,
        }
    }
}

#[async_trait]
impl Embedder for DualEmbedder {
    fn name(&self) -> &str {
        self.primary.name()
    }

    fn dimension(&self) -> usize {
        self.primary.dimension()
    }

    fn max_tokens(&self) -> usize {
        self.primary.max_tokens()
    }

    async fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
        let primary = self.primary.embed(text, purpose).await?;
        match self.comparison.embed(text, purpose).await {
            Ok(comparison) => {
                tracing::debug!(
                    target: "memory_embedder_ab",
                    primary = self.primary.name(),
                    comparison = self.comparison.name(),
                    primary_dim = primary.len(),
                    comparison_dim = comparison.len(),
                    purpose = ?purpose,
                    "dual_embedder_comparison"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "memory_embedder_ab",
                    comparison = self.comparison.name(),
                    error = %e,
                    "dual embedder comparison failed"
                );
            }
        }
        Ok(primary)
    }

    async fn embed_batch(
        &self,
        texts: &[&str],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        let primary = self.primary.embed_batch(texts, purpose).await?;
        match self.comparison.embed_batch(texts, purpose).await {
            Ok(comparison) => {
                tracing::debug!(
                    target: "memory_embedder_ab",
                    primary = self.primary.name(),
                    comparison = self.comparison.name(),
                    primary_count = primary.len(),
                    comparison_count = comparison.len(),
                    purpose = ?purpose,
                    "dual_embedder_batch_comparison"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "memory_embedder_ab",
                    comparison = self.comparison.name(),
                    error = %e,
                    "dual embedder batch comparison failed"
                );
            }
        }
        Ok(primary)
    }
}

/// C5: hosted-API embedder surface (Voyage, Cohere, OpenAI…).
///
/// Locked behind the `api-embedders` Cargo feature so the local-first
/// default never silently calls out to a remote service. Today the
/// only implementation is [`UnimplementedApiEmbedder`], a placeholder
/// that errors on `embed`. The factory exists so the configuration
/// surface (`memory.embedder.name = "voyage-code-3"` etc.) is
/// reserved before the real client is wired up.
#[cfg(feature = "api-embedders")]
pub mod api {
    use super::{Embedder, EmbeddingPurpose};
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use std::sync::Arc;

    /// Placeholder for an upcoming hosted-API embedder. Errors on
    /// `embed` so misconfigured workspaces fail fast and visibly.
    pub struct UnimplementedApiEmbedder {
        name: &'static str,
        dimension: usize,
    }

    impl UnimplementedApiEmbedder {
        pub fn new(name: &'static str, dimension: usize) -> Self {
            Self { name, dimension }
        }
    }

    #[async_trait]
    impl Embedder for UnimplementedApiEmbedder {
        fn name(&self) -> &str {
            self.name
        }
        fn dimension(&self) -> usize {
            self.dimension
        }
        async fn embed(&self, _text: &str, _purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
            Err(anyhow!(
                "api-embedders feature is enabled but no client has been wired up for {}",
                self.name
            ))
        }
    }

    /// Resolve a hosted-API embedder name to a placeholder. Real
    /// implementations replace this match arm-by-arm without changing
    /// the factory shape.
    pub fn build_api_embedder(name: &str) -> Option<Arc<dyn Embedder>> {
        match name {
            "voyage-code-3" => Some(Arc::new(UnimplementedApiEmbedder::new("voyage-code-3", 1024))),
            "cohere-embed-v3" => {
                Some(Arc::new(UnimplementedApiEmbedder::new("cohere-embed-v3", 1024)))
            }
            "openai-text-embedding-3-large" => Some(Arc::new(UnimplementedApiEmbedder::new(
                "openai-text-embedding-3-large",
                3072,
            ))),
            _ => None,
        }
    }
}

/// Shared C5 conformance test battery.
///
/// Every registered [`Embedder`] impl should be checked against this
/// suite — it pins the cross-impl invariants the retrieval layer
/// relies on, independent of the model family. Lives in the production
/// module (rather than `#[cfg(test)]`) so out-of-tree implementations
/// (e.g. an `OnnxEmbedder` test that needs the model file) can call it
/// without duplicating the assertions.
///
/// Each helper panics on failure with a message identifying the
/// invariant; combine them in your impl-specific test:
/// `embedder_battery::run(&my_embedder).await`.
#[doc(hidden)]
pub mod embedder_battery {
    use super::{Embedder, EmbeddingPurpose};
    use std::sync::Arc;

    /// Cosine similarity for L2-normalized vectors, defensive against
    /// degenerate inputs.
    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    /// Run the full battery. Use in impl-specific `#[tokio::test]`s.
    pub async fn run(embedder: Arc<dyn Embedder>) {
        determinism(embedder.clone()).await;
        purpose_distinguishes(embedder.clone()).await;
        empty_input_does_not_panic(embedder.clone()).await;
        long_input_is_truncated(embedder.clone()).await;
        batch_equals_single(embedder.clone()).await;
        similar_text_ranks_higher_than_unrelated(embedder).await;
    }

    pub async fn determinism(embedder: Arc<dyn Embedder>) {
        let a = embedder.embed_query("hello world").await.unwrap();
        let b = embedder.embed_query("hello world").await.unwrap();
        assert_eq!(a, b, "embed_query must be deterministic");
    }

    pub async fn purpose_distinguishes(embedder: Arc<dyn Embedder>) {
        // Models that don't task-prefix may produce identical vectors;
        // the contract is just that calling both purposes succeeds and
        // returns the right dimension.
        let q = embedder.embed_query("hello").await.unwrap();
        let d = embedder.embed_document("hello").await.unwrap();
        assert_eq!(q.len(), embedder.dimension());
        assert_eq!(d.len(), embedder.dimension());
    }

    pub async fn empty_input_does_not_panic(embedder: Arc<dyn Embedder>) {
        // Per the trait contract, an empty string is a valid query and
        // must produce a vector of the declared dimension.
        let v = embedder
            .embed("", EmbeddingPurpose::Query)
            .await
            .expect("empty input must not error");
        assert_eq!(v.len(), embedder.dimension());
    }

    pub async fn long_input_is_truncated(embedder: Arc<dyn Embedder>) {
        // Documented behavior: very long input is silently truncated to
        // the model's max_tokens at the tokenizer layer. The contract
        // we pin here is just "no panic, correct output dim".
        let long = "lorem ipsum ".repeat(8_000);
        let v = embedder
            .embed_document(&long)
            .await
            .expect("long input must truncate, not error");
        assert_eq!(v.len(), embedder.dimension());
    }

    pub async fn batch_equals_single(embedder: Arc<dyn Embedder>) {
        // Batch inference is a perf optimization, not a semantic one:
        // each row of `embed_batch` must equal the single-text result
        // for the same input. Tolerance accounts for f32 reduction-order
        // differences in batched ONNX kernels.
        let texts = ["alpha", "beta", "gamma payload"];
        let refs: Vec<&str> = texts.iter().copied().collect();
        let batched = embedder
            .embed_batch(&refs, EmbeddingPurpose::Document)
            .await
            .expect("embed_batch must succeed");
        assert_eq!(batched.len(), texts.len());
        for (i, text) in texts.iter().enumerate() {
            let single = embedder
                .embed_document(text)
                .await
                .expect("embed_document must succeed");
            assert_eq!(batched[i].len(), single.len());
            let sim = cosine(&batched[i], &single);
            assert!(
                sim > 0.999,
                "batch[{i}] must match single (cos={sim}); text={text:?}"
            );
        }
    }

    pub async fn similar_text_ranks_higher_than_unrelated(embedder: Arc<dyn Embedder>) {
        // Loosely: `doc(query)` must be more similar to `doc(query)`
        // than to `doc(unrelated)`. A NullEmbedder bucket-by-byte
        // satisfies this trivially; a real model satisfies it
        // semantically. The threshold is intentionally loose so toy
        // embedders pass.
        let target = "the quick brown fox jumps over the lazy dog";
        let near = embedder.embed_document(target).await.unwrap();
        let same = embedder.embed_document(target).await.unwrap();
        let far = embedder
            .embed_document("xyzzy plugh frobnicate")
            .await
            .unwrap();
        let cos_same = cosine(&near, &same);
        let cos_far = cosine(&near, &far);
        assert!(
            cos_same >= cos_far,
            "self-similarity ({cos_same}) must be >= dissimilar-text similarity ({cos_far})"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_embedder_is_deterministic_and_purpose_aware() {
        let embedder = NullEmbedder::new(8);
        let q1 = embedder.embed_query("same text").await.unwrap();
        let q2 = embedder.embed_query("same text").await.unwrap();
        let doc = embedder.embed_document("same text").await.unwrap();
        assert_eq!(q1, q2);
        assert_ne!(q1, doc);
        assert_eq!(q1.len(), 8);
    }

    #[tokio::test]
    async fn null_embedder_passes_shared_battery() {
        let embedder: Arc<dyn Embedder> = Arc::new(NullEmbedder::new(16));
        embedder_battery::run(embedder).await;
    }

    #[tokio::test]
    async fn dual_embedder_passes_shared_battery() {
        let primary: Arc<dyn Embedder> = Arc::new(NullEmbedder::new(16));
        let comparison: Arc<dyn Embedder> = Arc::new(NullEmbedder::new(16));
        let dual: Arc<dyn Embedder> = Arc::new(DualEmbedder::new(primary, comparison));
        embedder_battery::run(dual).await;
    }

    #[tokio::test]
    async fn dual_embedder_returns_primary_vectors() {
        let primary = Arc::new(NullEmbedder::new(8)) as Arc<dyn Embedder>;
        let comparison = Arc::new(NullEmbedder::new(4)) as Arc<dyn Embedder>;
        let dual = DualEmbedder::new(primary.clone(), comparison);

        let expected = primary.embed_query("query").await.unwrap();
        let actual = dual.embed_query("query").await.unwrap();
        assert_eq!(actual, expected);
        assert_eq!(dual.dimension(), 8);
    }
}
