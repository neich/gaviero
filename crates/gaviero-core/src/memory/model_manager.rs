use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Known embedding models and their download URLs.
pub struct ModelInfo {
    pub id: &'static str,
    pub onnx_url: &'static str,
    pub tokenizer_url: &'static str,
    pub dimensions: usize,
}

pub const E5_SMALL_V2: ModelInfo = ModelInfo {
    id: "e5-small-v2",
    onnx_url: "https://huggingface.co/intfloat/e5-small-v2/resolve/main/model.onnx",
    tokenizer_url: "https://huggingface.co/intfloat/e5-small-v2/resolve/main/tokenizer.json",
    dimensions: 384,
};

pub const NOMIC_EMBED_TEXT_V1_5: ModelInfo = ModelInfo {
    id: "nomic-embed-text-v1.5",
    onnx_url: "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/tokenizer.json",
    dimensions: 768,
};

/// Tier B / B1: Alibaba-NLP/gte-modernbert-base — Apache-2.0, ~149M
/// parameters, 768-dim (same as nomic so no schema change), 8192-token
/// context. Uses `"search_query: "` / `"search_document: "` prefixes.
pub const GTE_MODERNBERT_BASE: ModelInfo = ModelInfo {
    id: "gte-modernbert-base",
    onnx_url: "https://huggingface.co/Alibaba-NLP/gte-modernbert-base/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/Alibaba-NLP/gte-modernbert-base/resolve/main/tokenizer.json",
    dimensions: 768,
};

/// Tier B / S3.1: jinaai/jina-embeddings-v2-base-code — Apache-2.0,
/// ~161M parameters, **768-dim (same as nomic/gte so no vector
/// migration)**, 8192-token context, mean-pool + L2 (an exact match for
/// [`OnnxEmbedder`]'s pooling). Code-specialized (CoIR) and the third B1
/// candidate; uses **no** task prefix — adding one regresses code
/// retrieval. CPU-only inference here (`ort` has no GPU EP wired). The
/// ONNX export is self-contained (JinaBERT / ALiBi) and loads under the
/// generic `ort` session; it omits `token_type_ids`, which
/// `run_inference` already handles conditionally.
pub const JINA_EMBEDDINGS_V2_BASE_CODE: ModelInfo = ModelInfo {
    id: "jina-embeddings-v2-base-code",
    onnx_url: "https://huggingface.co/jinaai/jina-embeddings-v2-base-code/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
    dimensions: 768,
};

/// Resolve a settings string (`"nomic" | "gte-modernbert" | "jina-code"
/// | ""`) to a `ModelInfo`. Empty / unknown values fall back to the
/// configured default (currently `"nomic"`). The embedder-flip authority
/// for `jina-code` lives in PR-4 (S3.1), gated on the code-recall +
/// CPU-latency ablation.
pub fn resolve_embedder_model(name: &str) -> &'static ModelInfo {
    match name.trim().to_ascii_lowercase().as_str() {
        "gte-modernbert" | "gte-modernbert-base" => &GTE_MODERNBERT_BASE,
        "jina-code" | "jina" | "jina-v2-code" | "jina-embeddings-v2-base-code" => {
            &JINA_EMBEDDINGS_V2_BASE_CODE
        }
        "nomic" | "nomic-v15" | "nomic-v1.5" | "nomic-embed-text-v1.5" => &NOMIC_EMBED_TEXT_V1_5,
        "e5" | "e5-small-v2" => &E5_SMALL_V2,
        _ => &NOMIC_EMBED_TEXT_V1_5,
    }
}

/// Manages download and caching of embedding model files.
pub struct ModelManager {
    cache_dir: PathBuf,
}

impl ModelManager {
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gaviero")
            .join("models");
        Self { cache_dir }
    }

    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Check if a model is already downloaded.
    pub fn is_downloaded(&self, model: &ModelInfo) -> bool {
        self.model_dir(model).join("model.onnx").exists()
            && self.model_dir(model).join("tokenizer.json").exists()
    }

    /// Get the local directory for a model.
    pub fn model_dir(&self, model: &ModelInfo) -> PathBuf {
        self.cache_dir.join(model.id)
    }

    /// Get the path to the ONNX model file.
    pub fn onnx_path(&self, model: &ModelInfo) -> PathBuf {
        self.model_dir(model).join("model.onnx")
    }

    /// Get the path to the tokenizer file.
    pub fn tokenizer_path(&self, model: &ModelInfo) -> PathBuf {
        self.model_dir(model).join("tokenizer.json")
    }

    /// Download a model if not already cached.
    /// Prints progress to stderr.
    pub fn ensure_downloaded(&self, model: &ModelInfo) -> Result<()> {
        if self.is_downloaded(model) {
            return Ok(());
        }

        let dir = self.model_dir(model);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating model dir: {}", dir.display()))?;

        eprintln!("[gaviero] Downloading {} model...", model.id);

        Self::download_file(model.onnx_url, &self.onnx_path(model))
            .with_context(|| format!("downloading ONNX model from {}", model.onnx_url))?;

        Self::download_file(model.tokenizer_url, &self.tokenizer_path(model))
            .with_context(|| format!("downloading tokenizer from {}", model.tokenizer_url))?;

        eprintln!("[gaviero] Model {} ready.", model.id);
        Ok(())
    }

    fn download_file(url: &str, dest: &Path) -> Result<()> {
        let status = std::process::Command::new("curl")
            .args(["-fSL", "-o"])
            .arg(dest)
            .arg(url)
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("running curl (is curl installed?)")?;

        if !status.success() {
            anyhow::bail!("curl failed with status {}", status);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_embedder_model_maps_known_aliases() {
        assert_eq!(resolve_embedder_model("nomic").id, NOMIC_EMBED_TEXT_V1_5.id);
        assert_eq!(
            resolve_embedder_model("gte-modernbert").id,
            GTE_MODERNBERT_BASE.id
        );
        assert_eq!(resolve_embedder_model("e5").id, E5_SMALL_V2.id);
    }

    #[test]
    fn resolve_embedder_model_maps_jina_code_aliases() {
        for alias in [
            "jina-code",
            "jina",
            "jina-v2-code",
            "jina-embeddings-v2-base-code",
            "  JINA-CODE  ",
        ] {
            assert_eq!(
                resolve_embedder_model(alias).id,
                JINA_EMBEDDINGS_V2_BASE_CODE.id,
                "alias `{alias}` should resolve to jina"
            );
        }
    }

    #[test]
    fn resolve_embedder_model_unknown_falls_back_to_nomic() {
        assert_eq!(
            resolve_embedder_model("does-not-exist").id,
            NOMIC_EMBED_TEXT_V1_5.id
        );
        assert_eq!(resolve_embedder_model("").id, NOMIC_EMBED_TEXT_V1_5.id);
    }

    #[test]
    fn jina_is_768d_no_vector_migration() {
        // 768-dim keeps the same sqlite-vec column as nomic/gte → no
        // schema migration when symbol/memory vectors adopt jina.
        assert_eq!(JINA_EMBEDDINGS_V2_BASE_CODE.dimensions, 768);
        assert_eq!(
            JINA_EMBEDDINGS_V2_BASE_CODE.dimensions,
            NOMIC_EMBED_TEXT_V1_5.dimensions
        );
    }
}
