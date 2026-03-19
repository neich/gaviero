use anyhow::Result;

/// Trait for text embedding models.
///
/// Implementations must be Send + Sync so they can be shared across async tasks.
pub trait Embedder: Send + Sync {
    /// Embed a single text string into a vector of floats.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts. Default implementation calls `embed` in a loop.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// The dimensionality of the embedding vectors.
    fn dimensions(&self) -> usize;

    /// A string identifier for the model (e.g. "e5-small-v2").
    fn model_id(&self) -> &str;
}
