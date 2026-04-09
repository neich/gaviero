//! Tier routing: maps ModelTier + PrivacyLevel to concrete backend configuration.

use crate::types::{ModelTier, PrivacyLevel};
use super::models::WorkUnit;

/// Configuration for tier-based model routing.
#[derive(Debug, Clone)]
pub struct TierConfig {
    pub cheap_model: String,
    pub cheap_max_parallel: usize,
    pub expensive_model: String,
    pub expensive_max_parallel: usize,
    pub local: LocalConfig,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            cheap_model: "haiku".into(),
            cheap_max_parallel: 6,
            expensive_model: "sonnet".into(),
            expensive_max_parallel: 3,
            local: LocalConfig::default(),
        }
    }
}

/// Configuration for the optional local (Ollama) backend.
#[derive(Debug, Clone)]
pub struct LocalConfig {
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    pub max_parallel: usize,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "qwen2.5-coder:7b".into(),
            base_url: "http://localhost:11434".into(),
            max_parallel: 8,
        }
    }
}

/// Resolved backend for dispatching a work unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedBackend {
    /// Route to Claude API with the specified model.
    Claude { model: String },
    /// Route to Ollama local model.
    Ollama { model: String, base_url: String },
    /// Blocked — cannot dispatch (privacy constraint, unavailable backend).
    Blocked { reason: String },
}

/// Routes work units to concrete backends based on tier + privacy + config.
#[derive(Debug, Clone)]
pub struct TierRouter {
    config: TierConfig,
    ollama_available: bool,
}

impl TierRouter {
    pub fn new(config: TierConfig, ollama_available: bool) -> Self {
        Self { config, ollama_available }
    }

    /// Resolve a WorkUnit to a concrete backend + model.
    ///
    /// Resolution order:
    /// 1. If `unit.model` is `Some` → use it directly (privacy-checked)
    /// 2. Else → route by (tier, privacy, ollama_available)
    pub fn resolve(&self, unit: &WorkUnit) -> ResolvedBackend {
        // Model override takes precedence
        if let Some(ref model) = unit.model {
            return self.resolve_model_override(unit, model);
        }

        routing_match(unit.tier, unit.privacy, self.ollama_available, &self.config)
    }

    /// Resolve a WorkUnit to a trait-object backend.
    ///
    /// Calls `resolve()` internally, then maps `ResolvedBackend` variants
    /// to concrete `AgentBackend` trait implementations.
    pub fn resolve_backend(
        &self,
        unit: &WorkUnit,
    ) -> Result<Box<dyn super::backend::AgentBackend>, String> {
        match self.resolve(unit) {
            ResolvedBackend::Claude { model } => {
                Ok(Box::new(
                    super::backend::claude_code::ClaudeCodeBackend::new(&model),
                ))
            }
            ResolvedBackend::Ollama { model, base_url } => {
                Ok(Box::new(
                    super::backend::ollama::OllamaStreamBackend::new(&base_url, &model),
                ))
            }
            ResolvedBackend::Blocked { reason } => Err(reason),
        }
    }

    /// Handle escalation after subtask failure.
    ///
    /// Returns `None` if the unit has no escalation tier (max tier reached).
    pub fn escalate(&self, unit: &WorkUnit) -> Option<ResolvedBackend> {
        let escalation_tier = unit.escalation_tier?;
        Some(self.resolve_tier(escalation_tier, unit.privacy))
    }

    /// Resolve a specific tier + privacy combination directly.
    ///
    /// Used by the escalation path in the retry loop where the caller already
    /// knows the escalation tier and just needs a concrete backend.
    pub fn resolve_from_tier(&self, tier: ModelTier, privacy: PrivacyLevel) -> ResolvedBackend {
        self.resolve_tier(tier, privacy)
    }

    /// Resolve a specific tier + privacy combination.
    fn resolve_tier(&self, tier: ModelTier, privacy: PrivacyLevel) -> ResolvedBackend {
        routing_match(tier, privacy, self.ollama_available, &self.config)
    }

    /// Resolve a model override, checking privacy constraints.
    fn resolve_model_override(&self, unit: &WorkUnit, model: &str) -> ResolvedBackend {
        // Privacy check: LocalOnly units cannot use API models
        if unit.privacy == PrivacyLevel::LocalOnly {
            // Allow only if the override points to the local backend
            if model == self.config.local.model {
                return ResolvedBackend::Ollama {
                    model: model.to_string(),
                    base_url: self.config.local.base_url.clone(),
                };
            }
            return ResolvedBackend::Blocked {
                reason: format!(
                    "model override '{}' on LocalOnly unit — API models prohibited",
                    model
                ),
            };
        }

        ResolvedBackend::Claude { model: model.to_string() }
    }

    /// Update Ollama availability (call after health check).
    pub fn set_ollama_available(&mut self, available: bool) {
        self.ollama_available = available;
    }

    pub fn ollama_available(&self) -> bool {
        self.ollama_available
    }

    pub fn config(&self) -> &TierConfig {
        &self.config
    }
}

/// Core routing logic: maps (tier, privacy, ollama_available) to a concrete backend.
///
/// Extracted to avoid duplication between `resolve()` and `resolve_tier()`.
fn routing_match(
    tier: ModelTier,
    privacy: PrivacyLevel,
    ollama_available: bool,
    config: &TierConfig,
) -> ResolvedBackend {
    match (tier, privacy, ollama_available) {
        // Privacy-sensitive: force local regardless of tier
        (_, PrivacyLevel::LocalOnly, true) => ResolvedBackend::Ollama {
            model: config.local.model.clone(),
            base_url: config.local.base_url.clone(),
        },
        (_, PrivacyLevel::LocalOnly, false) => ResolvedBackend::Blocked {
            reason: "local model required but Ollama unavailable".into(),
        },
        // Cheap: use cheap_model (Haiku or local if configured)
        (ModelTier::Cheap, _, true) if config.local.enabled => ResolvedBackend::Ollama {
            model: config.local.model.clone(),
            base_url: config.local.base_url.clone(),
        },
        (ModelTier::Cheap, _, _) => ResolvedBackend::Claude {
            model: config.cheap_model.clone(),
        },
        // Expensive: always API
        (ModelTier::Expensive, _, _) => ResolvedBackend::Claude {
            model: config.expensive_model.clone(),
        },
    }
}

/// Validate that a work unit's model override doesn't violate privacy.
///
/// Returns `Ok(())` if valid, or an error message if the combination is invalid.
pub fn validate_privacy(unit: &WorkUnit) -> Result<(), String> {
    if unit.privacy == PrivacyLevel::LocalOnly {
        if let Some(ref model) = unit.model {
            // Only local models are acceptable for LocalOnly units
            if !model.contains("local") && !model.contains("ollama") && !model.contains("qwen") {
                return Err(format!(
                    "unit '{}': LocalOnly privacy with API model override '{}'",
                    unit.id, model
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use std::collections::HashMap;

    fn test_unit(tier: ModelTier, privacy: PrivacyLevel, model: Option<&str>) -> WorkUnit {
        WorkUnit {
            id: "test".into(),
            description: "test task".into(),
            scope: FileScope {
                owned_paths: vec!["src/".into()],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            backend: Default::default(),
            model: model.map(|s| s.to_string()),
            tier,
            privacy,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
        }
    }

    #[test]
    fn test_model_override_bypasses_tier() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, Some("opus"));
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "opus".into() }
        );
    }

    #[test]
    fn test_local_only_blocks_without_ollama() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::LocalOnly, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Blocked { .. }));
    }

    #[test]
    fn test_local_only_routes_to_ollama() {
        let router = TierRouter::new(TierConfig::default(), true);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::LocalOnly, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Ollama { .. }));
    }

    #[test]
    fn test_expensive_tier_routes_to_sonnet() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Expensive, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "sonnet".into() }
        );
    }

    #[test]
    fn test_cheap_tier_routes_to_haiku() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "haiku".into() }
        );
    }

    #[test]
    fn test_cheap_falls_back_to_haiku_when_local_disabled() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "haiku".into() }
        );
    }

    #[test]
    fn test_cheap_routes_to_ollama_when_enabled() {
        let mut config = TierConfig::default();
        config.local.enabled = true;
        let router = TierRouter::new(config, true);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Ollama { .. }));
    }

    #[test]
    fn test_local_only_model_override_blocked() {
        let router = TierRouter::new(TierConfig::default(), true);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::LocalOnly, Some("sonnet"));
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Blocked { .. }));
    }

    #[test]
    fn test_escalation_chain() {
        let router = TierRouter::new(TierConfig::default(), false);
        let mut unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);

        // Cheap → Expensive
        unit.escalation_tier = Some(ModelTier::Expensive);
        let esc = router.escalate(&unit).unwrap();
        assert_eq!(esc, ResolvedBackend::Claude { model: "sonnet".into() });

        // Expensive → None (no escalation)
        unit.tier = ModelTier::Expensive;
        unit.escalation_tier = None;
        assert!(router.escalate(&unit).is_none());
    }

    #[test]
    fn test_expensive_tier_routes_to_sonnet_via_resolve() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Expensive, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "sonnet".into() }
        );
    }

    // Test 21: resolve_backend returns correct trait objects
    #[test]
    fn test_resolve_backend_returns_trait_objects() {
        let router = TierRouter::new(TierConfig::default(), false);

        // Expensive tier → claude backend
        let unit = test_unit(ModelTier::Expensive, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("claude"));

        // Expensive → claude:sonnet
        let unit = test_unit(ModelTier::Expensive, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("claude"));
        assert!(backend.name().contains("sonnet"));

        // Cheap with Ollama enabled → ollama backend
        let mut config = TierConfig::default();
        config.local.enabled = true;
        let router = TierRouter::new(config, true);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("ollama"));

        // Blocked → returns Err
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Cheap, PrivacyLevel::LocalOnly, None);
        assert!(router.resolve_backend(&unit).is_err());
    }
}
