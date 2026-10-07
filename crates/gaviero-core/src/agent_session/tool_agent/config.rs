//! Configuration for in-process API tool-agent providers
//! (DeepSeek V4 Pro plan, Unit 2 — docs/plans/deepseek_v4_pro_provider.md).
//!
//! Resolves the per-provider runtime config once at session construction:
//! API key (env first, then a gitignored `.gaviero/secrets.toml`), base URL,
//! and the token price table. Deliberately NOT threaded through
//! `RuntimeConfig` / `SessionConstruction` as yet another `*_base_url` field —
//! the `ollama_base_url` sprawl is the anti-pattern this single struct avoids.

use std::fmt;
use std::path::{Path, PathBuf};

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

/// DeepSeek list prices (USD / 1M tokens), as published on
/// <https://api-docs.deepseek.com/quick_start/pricing> on 2026-10-06.
/// `providers.deepseek.pricing` overrides them wholesale.
const FLASH_OFF_PEAK: PriceTable = PriceTable {
    cache_hit_in: 0.003,
    cache_miss_in: 0.15,
    out: 0.60,
};
const FLASH_PEAK: PriceTable = PriceTable {
    cache_hit_in: 0.006,
    cache_miss_in: 0.30,
    out: 1.20,
};
const PRO_OFF_PEAK: PriceTable = PriceTable {
    cache_hit_in: 0.022,
    cache_miss_in: 0.66,
    out: 1.98,
};
const PRO_PEAK: PriceTable = PriceTable {
    cache_hit_in: 0.044,
    cache_miss_in: 1.32,
    out: 3.96,
};

/// Model ids served by V4.1-Flash: the canonical id plus the legacy names
/// DeepSeek still routes to it at Flash prices.
const FLASH_MODELS: &[&str] = &[
    "deepseek-flash",
    "deepseek-v4-flash",
    "deepseek-v4-flash-vision-exp",
];

impl PriceTable {
    /// The list price for `model` at the instant `at`.
    ///
    /// DeepSeek bills peak hours (01:00–04:00 and 06:00–10:00 UTC, Monday to
    /// Friday) at twice the off-peak rate. Chinese public holidays are
    /// off-peak upstream but are not modelled here, so a holiday turn is
    /// over-estimated — the safe direction for a spend bound. An id this table
    /// does not know is priced as V4-Pro, the dearer model, for the same reason.
    pub fn for_model(model: &str, at: chrono::DateTime<chrono::Utc>) -> Self {
        let flash = FLASH_MODELS.contains(&model.trim());
        match (flash, is_peak(at)) {
            (true, false) => FLASH_OFF_PEAK,
            (true, true) => FLASH_PEAK,
            (false, false) => PRO_OFF_PEAK,
            (false, true) => PRO_PEAK,
        }
    }

    /// Cost in USD for one turn's token counts.
    pub fn cost_usd(&self, cache_hit_in: u64, cache_miss_in: u64, out: u64) -> f64 {
        (cache_hit_in as f64 * self.cache_hit_in
            + cache_miss_in as f64 * self.cache_miss_in
            + out as f64 * self.out)
            / 1_000_000.0
    }
}

/// Whether `at` falls in DeepSeek's peak billing window.
fn is_peak(at: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::{Datelike, Timelike, Weekday};
    if matches!(at.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    matches!(at.hour(), 1..=3 | 6..=9)
}

pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

/// Resolved config handed to an [`super::ApiClient`].
#[derive(Clone, Debug)]
pub struct ApiClientConfig {
    pub base_url: String,
    pub api_key: ApiKey,
    /// Operator override (`providers.deepseek.pricing`). `None` prices each
    /// request from [`PriceTable::for_model`] at the time it is sent.
    pub pricing: Option<PriceTable>,
}

impl ApiClientConfig {
    /// The price table for one request to `model`, sent now.
    pub fn price_for(&self, model: &str) -> PriceTable {
        self.pricing
            .clone()
            .unwrap_or_else(|| PriceTable::for_model(model, chrono::Utc::now()))
    }
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
    /// Resolve DeepSeek config. API key, first match wins:
    /// 1. `DEEPSEEK_API_KEY` env;
    /// 2. `<workspace_root>/.gaviero/secrets.toml` `[deepseek] api_key`;
    /// 3. the user-level `~/.gaviero/secrets.toml` `[deepseek] api_key` — the
    ///    same directory as the user settings file, so one key serves every
    ///    workspace (and swarm worktrees, which carry no `.gaviero/` of their
    ///    own).
    ///
    /// `base_url` and `pricing` come from the caller (settings cascade); pass
    /// `None` for the documented defaults.
    pub fn resolve_deepseek(
        workspace_root: &Path,
        base_url: Option<String>,
        pricing: Option<PriceTable>,
    ) -> Result<Self> {
        let env_val = std::env::var(DEEPSEEK_API_KEY_ENV).ok();
        let api_key = pick_api_key(env_val, &secrets_candidates(workspace_root))?;
        Ok(Self {
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_DEEPSEEK_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key,
            pricing,
        })
    }
}

/// Secrets files searched for the key, in order: the workspace's, then the
/// user-level `~/.gaviero/secrets.toml` (skipped when it is the same file).
pub fn secrets_candidates(workspace_root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![workspace_root.join(".gaviero").join("secrets.toml")];
    if let Some(user) = user_secrets_path()
        && !paths.contains(&user)
    {
        paths.push(user);
    }
    paths
}

/// `~/.gaviero/secrets.toml`, beside the user settings file.
pub fn user_secrets_path() -> Option<PathBuf> {
    crate::workspace::user_settings_path().map(|p| p.with_file_name("secrets.toml"))
}

/// Pure key-selection logic, factored out of `resolve_deepseek` so tests do not
/// touch process-global env or the home directory. A non-empty env value
/// wins; otherwise the first `[deepseek] api_key` found in `secrets_paths`
/// (missing files are skipped; an unparseable one is an error, so a typo is
/// not silently masked by another file); otherwise a loud error naming every
/// source that was tried.
fn pick_api_key(env_val: Option<String>, secrets_paths: &[PathBuf]) -> Result<ApiKey> {
    if let Some(k) = env_val {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(ApiKey::new(k));
        }
    }
    for path in secrets_paths {
        if !path.exists() {
            continue;
        }
        let body = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: SecretsToml =
            toml::from_str(&body).with_context(|| format!("parsing {}", path.display()))?;
        if let Some(k) = parsed.deepseek.and_then(|d| d.api_key) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Ok(ApiKey::new(k));
            }
        }
    }
    let tried = secrets_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" or ");
    anyhow::bail!(
        "no DeepSeek API key: set {DEEPSEEK_API_KEY_ENV}, or add\n\
         [deepseek]\napi_key = \"sk-...\"\nto {tried}"
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
        let k = pick_api_key(Some("env-key".into()), &[secrets]).unwrap();
        assert_eq!(k.expose(), "env-key");
    }

    #[test]
    fn pick_secrets_fallback_when_no_env() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("secrets.toml");
        std::fs::write(&secrets, "[deepseek]\napi_key = \"file-key\"\n").unwrap();
        let k = pick_api_key(None, &[secrets]).unwrap();
        assert_eq!(k.expose(), "file-key");
    }

    #[test]
    fn pick_blank_env_falls_through_to_secrets() {
        let dir = tempdir().unwrap();
        let secrets = dir.path().join("secrets.toml");
        std::fs::write(&secrets, "[deepseek]\napi_key = \"file-key\"\n").unwrap();
        let k = pick_api_key(Some("   ".into()), &[secrets]).unwrap();
        assert_eq!(k.expose(), "file-key");
    }

    /// The user-level file is consulted when the workspace has no file, or a
    /// file without a `[deepseek]` key; the workspace's key wins when present.
    #[test]
    fn pick_falls_back_to_the_user_level_file() {
        let workspace = tempdir().unwrap();
        let home = tempdir().unwrap();
        let ws_secrets = workspace.path().join("secrets.toml");
        let user_secrets = home.path().join("secrets.toml");
        std::fs::write(&user_secrets, "[deepseek]\napi_key = \"user-key\"\n").unwrap();
        let paths = [ws_secrets.clone(), user_secrets];

        assert_eq!(pick_api_key(None, &paths).unwrap().expose(), "user-key");
        std::fs::write(&ws_secrets, "[other]\nx = 1\n").unwrap();
        assert_eq!(pick_api_key(None, &paths).unwrap().expose(), "user-key");
        std::fs::write(&ws_secrets, "[deepseek]\napi_key = \"ws-key\"\n").unwrap();
        assert_eq!(pick_api_key(None, &paths).unwrap().expose(), "ws-key");
    }

    #[test]
    fn pick_missing_everywhere_names_every_source() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a").join("secrets.toml");
        let b = dir.path().join("b").join("secrets.toml");
        let err = pick_api_key(None, &[a.clone(), b.clone()]).unwrap_err().to_string();
        assert!(err.contains("DEEPSEEK_API_KEY"), "{err}");
        assert!(err.contains(&a.display().to_string()), "{err}");
        assert!(err.contains(&b.display().to_string()), "{err}");
        assert!(err.contains("[deepseek]"), "{err}");
    }

    #[test]
    fn user_secrets_live_beside_user_settings() {
        let path = user_secrets_path().expect("home dir");
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("secrets.toml"));
        assert_eq!(
            path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()),
            Some(".gaviero")
        );
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

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn peak_window_boundaries() {
        // 2026-10-06 is a Tuesday; 2026-10-10 a Saturday.
        assert!(!is_peak(utc(2026, 10, 6, 0, 59)));
        assert!(is_peak(utc(2026, 10, 6, 1, 0)));
        assert!(is_peak(utc(2026, 10, 6, 3, 59)));
        assert!(!is_peak(utc(2026, 10, 6, 4, 0)));
        assert!(!is_peak(utc(2026, 10, 6, 5, 59)));
        assert!(is_peak(utc(2026, 10, 6, 6, 0)));
        assert!(is_peak(utc(2026, 10, 6, 9, 59)));
        assert!(!is_peak(utc(2026, 10, 6, 10, 0)));
        assert!(!is_peak(utc(2026, 10, 10, 2, 0)));
    }

    #[test]
    fn prices_follow_the_model_and_the_clock() {
        let off = utc(2026, 10, 6, 12, 0);
        let peak = utc(2026, 10, 6, 7, 0);
        assert_eq!(PriceTable::for_model("deepseek-flash", off), FLASH_OFF_PEAK);
        assert_eq!(PriceTable::for_model("deepseek-flash", peak), FLASH_PEAK);
        assert_eq!(PriceTable::for_model("deepseek-v4-pro", off), PRO_OFF_PEAK);
        assert_eq!(PriceTable::for_model("deepseek-v4-pro", peak), PRO_PEAK);
        // Legacy aliases route to Flash upstream and are billed as Flash.
        assert_eq!(PriceTable::for_model("deepseek-v4-flash", off), FLASH_OFF_PEAK);
        // An unknown id is priced as the dearer model.
        assert_eq!(PriceTable::for_model("deepseek-v5", off), PRO_OFF_PEAK);
    }

    #[test]
    fn an_override_wins_over_the_model_table() {
        let fixed = PriceTable {
            cache_hit_in: 1.0,
            cache_miss_in: 2.0,
            out: 3.0,
        };
        let cfg = ApiClientConfig {
            base_url: DEFAULT_DEEPSEEK_BASE_URL.into(),
            api_key: ApiKey::new("k"),
            pricing: Some(fixed.clone()),
        };
        assert_eq!(cfg.price_for("deepseek-flash"), fixed);
        let cfg = ApiClientConfig {
            pricing: None,
            ..cfg
        };
        let flash = cfg.price_for("deepseek-flash");
        assert!(flash == FLASH_OFF_PEAK || flash == FLASH_PEAK);
    }

    #[test]
    fn api_key_debug_is_redacted() {
        let k = ApiKey::new("super-secret");
        assert_eq!(format!("{k:?}"), "ApiKey(***redacted***)");
        assert!(!format!("{k:?}").contains("super-secret"));
    }
}
