use anyhow::Result;

/// Trait for text embedding models.
///
/// Implementations must be Send + Sync so they can be shared across async tasks.
///
/// Models that use task prefixes (e.g., nomic-embed-text-v1.5 uses
/// "search_query: " / "search_document: ") should override `embed_query`
/// and `embed_document`. The defaults delegate to `embed()` for backward
/// compatibility.
pub trait Embedder: Send + Sync {
    /// Embed a single text string into a vector of floats.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a query text (for search). Applies query-specific prefix if the model requires it.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text)
    }

    /// Embed a document text (for storage). Applies document-specific prefix if the model requires it.
    fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text)
    }

    /// Embed a batch of texts. Default implementation calls `embed` in a loop.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// The dimensionality of the embedding vectors.
    fn dimensions(&self) -> usize;

    /// A string identifier for the model (e.g. "nomic-embed-text-v1.5").
    fn model_id(&self) -> &str;
}
