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
