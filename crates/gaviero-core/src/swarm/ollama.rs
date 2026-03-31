#![allow(deprecated)]
//! Ollama backend: local LLM execution via the Ollama HTTP API.
//!
//! This module talks to Ollama's `/api/generate` endpoint.
//! No ACP subprocess — direct HTTP calls to keep the dependency surface minimal.

use anyhow::{Context, Result};

/// Ollama HTTP client for local model execution (non-streaming).
///
/// **Deprecated:** Use [`super::backend::ollama::OllamaStreamBackend`] which
/// implements the [`super::backend::AgentBackend`] trait with streaming support.
#[deprecated(note = "Use backend::ollama::OllamaStreamBackend instead")]
#[derive(Debug, Clone)]
pub struct OllamaBackend {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Generate a completion from the local model.
    ///
    /// Streaming is disabled — waits for the full response.
    pub async fn generate(
        &self,
        prompt: &str,
        system: &str,
    ) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "system": system,
            "stream": false,
        });

        let response = self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("sending request to Ollama")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {}: {}", status, text);
        }

        let json: serde_json::Value = response
            .json()
            .await
            .context("parsing Ollama response")?;

        json.get("response")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Ollama response missing 'response' field"))
    }

    /// Health check — returns false if Ollama is unreachable.
    pub async fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Check if a specific model is available locally.
    pub async fn has_model(&self, model: &str) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return false,
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(_) => return false,
        };

        json.get("models")
            .and_then(|m| m.as_array())
            .map(|models| {
                models.iter().any(|m| {
                    m.get("name")
                        .and_then(|n| n.as_str())
                        .map(|n| n.starts_with(model))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Extract `<file>` blocks from an Ollama response.
///
/// Reuses the same protocol as ACP file block detection.
/// Returns (path, content) pairs.
pub fn extract_file_blocks(response: &str) -> Vec<(std::path::PathBuf, String)> {
    crate::acp::protocol::parse_file_blocks(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_file_blocks() {
        let response = r#"Here are the changes:
<file path="src/auth.rs">
fn validate() {}
</file>
<file path="src/lib.rs">
mod auth;
</file>
Done."#;

        let blocks = extract_file_blocks(response);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, std::path::PathBuf::from("src/auth.rs"));
        assert!(blocks[0].1.contains("fn validate()"));
        assert_eq!(blocks[1].0, std::path::PathBuf::from("src/lib.rs"));
        assert!(blocks[1].1.contains("mod auth;"));
    }

    #[test]
    fn test_extract_file_blocks_empty() {
        let response = "No file changes needed.";
        let blocks = extract_file_blocks(response);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_ollama_backend_new() {
        let backend = OllamaBackend::new("http://localhost:11434", "qwen2.5-coder:7b");
        assert_eq!(backend.model(), "qwen2.5-coder:7b");
        assert_eq!(backend.base_url(), "http://localhost:11434");
    }

    #[test]
    fn test_ollama_backend_strips_trailing_slash() {
        let backend = OllamaBackend::new("http://localhost:11434/", "model");
        assert_eq!(backend.base_url(), "http://localhost:11434");
    }
}
