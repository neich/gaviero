//! Configuration for in-process API tool-agent providers
//! (DeepSeek V4 Pro plan, Unit 2 — docs/plans/deepseek_v4_pro_provider.md).
//!
//! Resolves the per-provider runtime config once at session construction:
//! API key (env first, then a gitignored `.gaviero/secrets.toml`), base URL,
//! and the token price table. Deliberately NOT threaded through
//! `RuntimeConfig` / `SessionConstruction` as yet another `*_base_url` field —
//! the `ollama_base_url` sprawl is the anti-pattern this single struct avoids.

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// API key wrapper with a redacting `Debug` so the secret never lands in logs
/// or panic messages.
///
/// Dependency-free stand-in for `secrecy::SecretString`: the plan named
/// `secrecy`, but it is not yet in the workspace lockfile, and PR-1 stays
/// offline-buildable. Swap to `secrecy` in a later hardening pass if desired.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Expose the raw token for the `Authorization: Bearer` header.
    /// Call sites must not log the return value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***redacted***)")
    }
}

/// Per-1M-token USD prices. DeepSeek bills cache-hit and cache-miss input
/// tokens at different rates, so all three are tracked.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PriceTable {
    /// USD per 1M input tokens that hit the context cache.
    pub cache_hit_in: f64,
    /// USD per 1M input tokens that missed the context cache.
    pub cache_miss_in: f64,
    /// USD per 1M output tokens.
    pub out: f64,
}

impl Default for PriceTable {
    fn default() -> Self {
        // DeepSeek V4 Pro list prices (USD / 1M tokens). Overridable via
        // settings.json `providers.deepseek.pricing`.
        Self {
            cache_hit_in: 0.07,
            cache_miss_in: 0.56,
            out: 1.68,
        }
    }
}

impl PriceTable {
    /// Cost in USD for one turn's token counts.
    pub fn cost_usd(&self, cache_hit_in: u64, cache_miss_in: u64, out: u64) -> f64 {
        (cache_hit_in as f64 * self.cache_hit_in
            + cache_miss_in as f64 * self.cache_miss_in
            + out as f64 * self.out)
            / 1_000_000.0
    }
}

pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

/// Resolved config handed to an [`super::ApiClient`].
#[derive(Clone, Debug)]
pub struct ApiClientConfig {
    pub base_url: String,
    pub api_key: ApiKey,
    pub pricing: PriceTable,
}

#[derive(Deserialize, Default)]
struct SecretsToml {
    deepseek: Option<SecretsSection>,
}

#[derive(Deserialize, Default)]
struct SecretsSection {
    api_key: Option<String>,
}

impl ApiClientConfig {
    /// Resolve DeepSeek config. API key: `DEEPSEEK_API_KEY` env first, then
    /// `<workspace_root>/.gaviero/secrets.toml` `[deepseek] api_key`. `base_url`
    /// and `pricing` come from the caller (settings cascade); pass `None` for
    /// the documented defaults.
    pub fn resolve_deepseek(
        workspace_root: &Path,
        base_url: Option<String>,
        pricing: Option<PriceTable>,
    ) -> Result<Self> {
        let env_val = std::env::var(DEEPSEEK_API_KEY_ENV).ok();
        let secrets_path = workspace_root.join(".gaviero").join("secrets.toml");
        let api_key = pick_api_key(env_val, &secrets_path)?;
        Ok(Self {
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_DEEPSEEK_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key,
            pricing: pricing.unwrap_or_default(),
        })
    }
}

/// Pure key-selection logic, factored out of `resolve_deepseek` so tests do not
/// touch process-global env. Env value wins when non-empty; otherwise the
/// `[deepseek] api_key` from `secrets_path` (if the file exists); otherwise a
/// loud error naming both sources.
fn pick_api_key(env_val: Option<String>, secrets_path: &Path) -> Result<ApiKey> {
    if let Some(k) = env_val {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(ApiKey::new(k));
        }
    }
    if secrets_path.exists() {
        let body = std::fs::read_to_string(secrets_path)
            .with_context(|| format!("reading {}", secrets_path.display()))?;
        let parsed: SecretsToml = toml::from_str(&body)
            .with_context(|| format!("parsing {}", secrets_path.display()))?;
        if let Some(k) = parsed.deepseek.and_then(|d| d.api_key) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Ok(ApiKey::new(k));
            }
        }
    }
    anyhow::bail!(
        "no DeepSeek API key: set {DEEPSEEK_API_KEY_ENV} or add a `[deepseek]` \
         section with `api_key` to {}",
        secrets_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pick_env_key_wins() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("secrets.toml");
        let k = pick_api_key(Some("env-key".into()), &secrets).unwrap();
        assert_eq!(k.expose(), "env-key");
    }

    #[test]
    fn pick_secrets_fallback_when_no_env() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("secrets.toml");
        std::fs::write(&secrets, "[deepseek]\napi_key = \"file-key\"\n").unwrap();
        let k = pick_api_key(None, &secrets).unwrap();
        assert_eq!(k.expose(), "file-key");
    }

    #[test]
    fn pick_blank_env_falls_through_to_secrets() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("secrets.toml");
        std::fs::write(&secrets, "[deepseek]\napi_key = \"file-key\"\n").unwrap();
        let k = pick_api_key(Some("   ".into()), &secrets).unwrap();
        assert_eq!(k.expose(), "file-key");
    }

    #[test]
    fn pick_missing_both_errors() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("absent.toml");
        let err = pick_api_key(None, &secrets).unwrap_err();
        assert!(err.to_string().contains("DEEPSEEK_API_KEY"));
    }

    #[test]
    fn price_table_cost_is_sum_of_three_rates() {
        let p = PriceTable {
            cache_hit_in: 0.07,
            cache_miss_in: 0.56,
            out: 1.68,
        };
        let c = p.cost_usd(80, 20, 10);
        let expected = (80.0 * 0.07 + 20.0 * 0.56 + 10.0 * 1.68) / 1_000_000.0;
        assert!((c - expected).abs() < 1e-12);
    }

    #[test]
    fn api_key_debug_is_redacted() {
        let k = ApiKey::new("super-secret");
        assert_eq!(format!("{k:?}"), "ApiKey(***redacted***)");
        assert!(!format!("{k:?}").contains("super-secret"));
    }
}
