use std::sync::Mutex;

use anyhow::{Context, Result};
use ndarray::Array2;
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::Tokenizer;

use super::embedder::Embedder;
use super::model_manager::{ModelInfo, ModelManager};

/// ONNX-based text embedder using E5 or similar models.
///
/// Uses interior mutability (Mutex) because `ort::Session::run()` requires `&mut self`
/// but the `Embedder` trait uses `&self` for Send + Sync compatibility.
pub struct OnnxEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    dimensions: usize,
    model_id: String,
}

impl OnnxEmbedder {
    /// Create an embedder from a model info descriptor.
    /// Downloads the model if not cached.
    pub fn from_model(model: &ModelInfo) -> Result<Self> {
        let manager = ModelManager::new();
        manager.ensure_downloaded(model)?;
        Self::load(&manager.onnx_path(model), &manager.tokenizer_path(model), model.dimensions, model.id)
    }

    /// Load an ONNX model and tokenizer from paths.
    pub fn load(
        onnx_path: &std::path::Path,
        tokenizer_path: &std::path::Path,
        dimensions: usize,
        model_id: &str,
    ) -> Result<Self> {
        let session = Session::builder()
            .map_err(|e| anyhow::anyhow!("creating ONNX session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!("setting thread count: {e}"))?
            .commit_from_file(onnx_path)
            .map_err(|e| anyhow::anyhow!("loading ONNX model from {}: {e}", onnx_path.display()))?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading tokenizer: {}", e))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dimensions,
            model_id: model_id.to_string(),
        })
    }

    /// Run inference on a batch of texts and return normalized embeddings.
    fn run_inference(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Tokenize all texts
        let encodings = self.tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {}", e))?;

        let batch_size = encodings.len();
        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);

        // Build input tensors: input_ids and attention_mask
        let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
        let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));

        for (i, encoding) in encodings.iter().enumerate() {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            for (j, (&id, &m)) in ids.iter().zip(mask.iter()).enumerate() {
                input_ids[[i, j]] = id as i64;
                attention_mask[[i, j]] = m as i64;
            }
        }

        // Create tensor references for ONNX
        let ids_tensor = TensorRef::from_array_view(&input_ids)
            .map_err(|e| anyhow::anyhow!("creating input_ids tensor: {e}"))?;
        let mask_tensor = TensorRef::from_array_view(&attention_mask)
            .map_err(|e| anyhow::anyhow!("creating attention_mask tensor: {e}"))?;

        // Run ONNX inference (requires &mut session)
        let mut session = self.session.lock().map_err(|e| anyhow::anyhow!("session lock poisoned: {e}"))?;
        let outputs = session.run(ort::inputs![
            "input_ids" => ids_tensor,
            "attention_mask" => mask_tensor,
        ]).map_err(|e| anyhow::anyhow!("ONNX inference failed: {e}"))?;

        // Extract output: [batch, seq_len, hidden_dim]
        let output_array = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| anyhow::anyhow!("extracting output tensor: {e}"))?;
        let shape = output_array.shape();
        let hidden_dim = shape[2];

        // Mean pooling over sequence dimension, masked by attention_mask
        let mut embeddings = Vec::with_capacity(batch_size);

        for i in 0..batch_size {
            let mut sum = vec![0.0f32; hidden_dim];
            let mut count = 0.0f32;

            for j in 0..max_len {
                if attention_mask[[i, j]] == 1 {
                    for k in 0..hidden_dim {
                        sum[k] += output_array[[i, j, k]];
                    }
                    count += 1.0;
                }
            }

            if count > 0.0 {
                for v in &mut sum {
                    *v /= count;
                }
            }

            // L2 normalize
            let norm: f32 = sum.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut sum {
                    *v /= norm;
                }
            }

            embeddings.push(sum);
        }

        Ok(embeddings)
    }
}

impl Embedder for OnnxEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.run_inference(&[text])?;
        results.into_iter().next().context("empty inference result")
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.run_inference(texts)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires ONNX model to be downloaded
    fn test_onnx_embedder() {
        let embedder = OnnxEmbedder::from_model(&super::super::model_manager::E5_SMALL_V2)
            .expect("Failed to load model");
        let embedding = embedder.embed("query: hello world").unwrap();
        assert_eq!(embedding.len(), 384);
        // Verify L2 normalization
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }
}
