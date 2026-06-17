use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use ndarray::Array2;
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

use super::embedder::{Embedder, EmbeddingPurpose};
use super::model_manager::{ModelInfo, ModelManager};

/// Upper bound for tokenized sequence length. Caps ONNX memory (attention
/// scales as O(seq²)) so oversized inputs cannot OOM the process. 512 is the
/// standard retrieval-embedder context; nomic-embed-text-v1.5 and e5-small-v2
/// both accept far less than this (the former tops out at 8192, the latter at
/// 512), so 512 is safe for all currently-supported models.
pub const MAX_SEQUENCE_LENGTH: usize = 512;

/// ONNX-based text embedder using E5, nomic-embed-text, or similar models.
///
/// Uses interior mutability (Mutex) because `ort::Session::run()` requires `&mut self`
/// but the `Embedder` trait uses `&self` for Send + Sync compatibility.
///
/// Supports optional task prefixes for models that require them (e.g.,
/// nomic-embed-text-v1.5 uses "search_query: " and "search_document: ").
pub struct OnnxEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    dimensions: usize,
    model_id: String,
    prefix_query: Option<String>,
    prefix_document: Option<String>,
}

impl OnnxEmbedder {
    /// Create an embedder from a model info descriptor.
    /// Downloads the model if not cached.
    pub fn from_model(model: &ModelInfo) -> Result<Self> {
        let manager = ModelManager::new();
        manager.ensure_downloaded(model)?;
        let mut embedder = Self::load(
            &manager.onnx_path(model),
            &manager.tokenizer_path(model),
            model.dimensions,
            model.id,
        )?;

        // Set task prefixes based on model
        match model.id {
            "nomic-embed-text-v1.5" => {
                embedder.prefix_query = Some("search_query: ".to_string());
                embedder.prefix_document = Some("search_document: ".to_string());
            }
            "e5-small-v2" => {
                embedder.prefix_query = Some("query: ".to_string());
                embedder.prefix_document = Some("passage: ".to_string());
            }
            "gte-modernbert-base" => {
                // gte-modernbert is trained with the same query/document
                // prefixes as nomic; mismatched prefixes regress recall
                // noticeably on the gte ablation set.
                embedder.prefix_query = Some("search_query: ".to_string());
                embedder.prefix_document = Some("search_document: ".to_string());
            }
            _ => {}
        }

        Ok(embedder)
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

        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading tokenizer: {}", e))?;

        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_SEQUENCE_LENGTH,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
                direction: TruncationDirection::Right,
            }))
            .map_err(|e| anyhow::anyhow!("configuring tokenizer truncation: {}", e))?;

        // Defensive: some tokenizers (notably the Alibaba gte-modernbert family)
        // ship a *fixed* padding strategy in tokenizer.json that re-pads every
        // input back up to thousands of tokens *after* the truncation above,
        // reintroducing the O(seq²) ONNX blow-up that truncation is meant to
        // prevent. `run_inference` zero-pads to the batch max itself, so disable
        // tokenizer-side padding. (No-op for nomic/e5, whose padding is null.)
        tokenizer.with_padding(None);

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dimensions,
            model_id: model_id.to_string(),
            prefix_query: None,
            prefix_document: None,
        })
    }

    /// Run inference on a batch of texts and return normalized embeddings.
    fn run_inference(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Tokenize all texts
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {}", e))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

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

        // Some models (e.g. nomic-embed-text-v1.5) require token_type_ids.
        // Build it as all-zeros (standard for single-sequence encoding) and
        // pass it only when the model declares it as an input.
        let token_type_ids = Array2::<i64>::zeros((batch_size, max_len));
        let type_tensor = TensorRef::from_array_view(&token_type_ids)
            .map_err(|e| anyhow::anyhow!("creating token_type_ids tensor: {e}"))?;

        // Run ONNX inference (requires &mut session)
        let mut session = self
            .session
            .lock()
            .map_err(|e| anyhow::anyhow!("session lock poisoned: {e}"))?;
        let needs_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let outputs = if needs_token_type_ids {
            session.run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                "token_type_ids" => type_tensor,
            ])
        } else {
            session.run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ])
        }
        .map_err(|e| anyhow::anyhow!("ONNX inference failed: {e}"))?;

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

            // Truncate to target dimensions (Matryoshka support)
            sum.truncate(self.dimensions);

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

    fn apply_prefix(&self, text: &str, purpose: EmbeddingPurpose) -> String {
        match purpose {
            EmbeddingPurpose::Query => match &self.prefix_query {
                Some(prefix) => format!("{prefix}{text}"),
                None => text.to_string(),
            },
            EmbeddingPurpose::Document => match &self.prefix_document {
                Some(prefix) => format!("{prefix}{text}"),
                None => text.to_string(),
            },
        }
    }
}

#[async_trait]
impl Embedder for OnnxEmbedder {
    fn name(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dimensions
    }

    fn max_tokens(&self) -> usize {
        MAX_SEQUENCE_LENGTH
    }

    async fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
        let prefixed = self.apply_prefix(text, purpose);
        let results = self.run_inference(&[&prefixed])?;
        results.into_iter().next().context("empty inference result")
    }

    async fn embed_batch(
        &self,
        texts: &[&str],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts
            .iter()
            .map(|text| self.apply_prefix(text, purpose))
            .collect();
        let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
        self.run_inference(&refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires ONNX model to be downloaded
    async fn test_onnx_embedder() {
        let embedder = OnnxEmbedder::from_model(&super::super::model_manager::E5_SMALL_V2)
            .expect("Failed to load model");
        let embedding = embedder
            .embed("query: hello world", EmbeddingPurpose::Query)
            .await
            .unwrap();
        assert_eq!(embedding.len(), 384);
        // Verify L2 normalization
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    #[ignore] // Requires ONNX model to be downloaded
    async fn test_nomic_embedder() {
        let embedder =
            OnnxEmbedder::from_model(&super::super::model_manager::NOMIC_EMBED_TEXT_V1_5)
                .expect("Failed to load nomic model");

        // Test query embedding
        let query_emb = embedder.embed_query("What is Rust?").await.unwrap();
        assert_eq!(query_emb.len(), 768);
        let norm: f32 = query_emb.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);

        // Test document embedding
        let doc_emb = embedder
            .embed_document("Rust is a systems programming language")
            .await
            .unwrap();
        assert_eq!(doc_emb.len(), 768);
    }
}
