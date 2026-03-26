//! Tier routing: maps ModelTier + PrivacyLevel to concrete backend configuration.

use crate::types::{ModelTier, PrivacyLevel};
use super::models::WorkUnit;

/// Configuration for tier-based model routing.
#[derive(Debug, Clone)]
pub struct TierConfig {
    pub reasoning_model: String,
    pub reasoning_max_parallel: usize,
    pub execution_model: String,
    pub execution_max_parallel: usize,
    pub mechanical: MechanicalConfig,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            reasoning_model: "sonnet".into(),
            reasoning_max_parallel: 3,
            execution_model: "haiku".into(),
            execution_max_parallel: 6,
            mechanical: MechanicalConfig::default(),
        }
    }
}

/// Configuration for the optional mechanical (local) tier.
#[derive(Debug, Clone)]
pub struct MechanicalConfig {
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    pub max_parallel: usize,
}

impl Default for MechanicalConfig {
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

        match (unit.tier, unit.privacy, self.ollama_available) {
            // Privacy-sensitive: force local regardless of tier
            (_, PrivacyLevel::LocalOnly, true) => ResolvedBackend::Ollama {
                model: self.config.mechanical.model.clone(),
                base_url: self.config.mechanical.base_url.clone(),
            },
            (_, PrivacyLevel::LocalOnly, false) => ResolvedBackend::Blocked {
                reason: "local model required but Ollama unavailable".into(),
            },

            // Normal routing by tier
            (ModelTier::Coordinator, _, _) => ResolvedBackend::Claude {
                model: "opus".into(),
            },
            (ModelTier::Reasoning, _, _) => ResolvedBackend::Claude {
                model: self.config.reasoning_model.clone(),
            },
            (ModelTier::Execution, _, _) => ResolvedBackend::Claude {
                model: self.config.execution_model.clone(),
            },

            // Mechanical: local if available and enabled, else fall back to execution model
            (ModelTier::Mechanical, _, true) if self.config.mechanical.enabled => {
                ResolvedBackend::Ollama {
                    model: self.config.mechanical.model.clone(),
                    base_url: self.config.mechanical.base_url.clone(),
                }
            }
            (ModelTier::Mechanical, _, _) => ResolvedBackend::Claude {
                model: self.config.execution_model.clone(),
            },
        }
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

    /// Resolve a specific tier + privacy combination.
    fn resolve_tier(&self, tier: ModelTier, privacy: PrivacyLevel) -> ResolvedBackend {
        match (tier, privacy, self.ollama_available) {
            (_, PrivacyLevel::LocalOnly, true) => ResolvedBackend::Ollama {
                model: self.config.mechanical.model.clone(),
                base_url: self.config.mechanical.base_url.clone(),
            },
            (_, PrivacyLevel::LocalOnly, false) => ResolvedBackend::Blocked {
                reason: "local model required but Ollama unavailable".into(),
            },
            (ModelTier::Coordinator, _, _) => ResolvedBackend::Claude { model: "opus".into() },
            (ModelTier::Reasoning, _, _) => ResolvedBackend::Claude {
                model: self.config.reasoning_model.clone(),
            },
            (ModelTier::Execution, _, _) => ResolvedBackend::Claude {
                model: self.config.execution_model.clone(),
            },
            (ModelTier::Mechanical, _, true) if self.config.mechanical.enabled => {
                ResolvedBackend::Ollama {
                    model: self.config.mechanical.model.clone(),
                    base_url: self.config.mechanical.base_url.clone(),
                }
            }
            (ModelTier::Mechanical, _, _) => ResolvedBackend::Claude {
                model: self.config.execution_model.clone(),
            },
        }
    }

    /// Resolve a model override, checking privacy constraints.
    fn resolve_model_override(&self, unit: &WorkUnit, model: &str) -> ResolvedBackend {
        // Privacy check: LocalOnly units cannot use API models
        if unit.privacy == PrivacyLevel::LocalOnly {
            // Allow only if the override points to the local backend
            if model == self.config.mechanical.model {
                return ResolvedBackend::Ollama {
                    model: model.to_string(),
                    base_url: self.config.mechanical.base_url.clone(),
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
        }
    }

    #[test]
    fn test_model_override_bypasses_tier() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Mechanical, PrivacyLevel::Public, Some("opus"));
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "opus".into() }
        );
    }

    #[test]
    fn test_local_only_blocks_without_ollama() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Execution, PrivacyLevel::LocalOnly, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Blocked { .. }));
    }

    #[test]
    fn test_local_only_routes_to_ollama() {
        let router = TierRouter::new(TierConfig::default(), true);
        let unit = test_unit(ModelTier::Execution, PrivacyLevel::LocalOnly, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Ollama { .. }));
    }

    #[test]
    fn test_reasoning_tier_routes_to_sonnet() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Reasoning, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "sonnet".into() }
        );
    }

    #[test]
    fn test_execution_tier_routes_to_haiku() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Execution, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "haiku".into() }
        );
    }

    #[test]
    fn test_mechanical_falls_back_to_haiku_when_disabled() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Mechanical, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "haiku".into() }
        );
    }

    #[test]
    fn test_mechanical_routes_to_ollama_when_enabled() {
        let mut config = TierConfig::default();
        config.mechanical.enabled = true;
        let router = TierRouter::new(config, true);
        let unit = test_unit(ModelTier::Mechanical, PrivacyLevel::Public, None);
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Ollama { .. }));
    }

    #[test]
    fn test_local_only_model_override_blocked() {
        let router = TierRouter::new(TierConfig::default(), true);
        let unit = test_unit(ModelTier::Execution, PrivacyLevel::LocalOnly, Some("sonnet"));
        assert!(matches!(router.resolve(&unit), ResolvedBackend::Blocked { .. }));
    }

    #[test]
    fn test_escalation_chain() {
        let router = TierRouter::new(TierConfig::default(), false);
        let mut unit = test_unit(ModelTier::Mechanical, PrivacyLevel::Public, None);

        // Mechanical → Execution
        unit.escalation_tier = Some(ModelTier::Execution);
        let esc = router.escalate(&unit).unwrap();
        assert_eq!(esc, ResolvedBackend::Claude { model: "haiku".into() });

        // Execution → Reasoning
        unit.tier = ModelTier::Execution;
        unit.escalation_tier = Some(ModelTier::Reasoning);
        let esc = router.escalate(&unit).unwrap();
        assert_eq!(esc, ResolvedBackend::Claude { model: "sonnet".into() });

        // Reasoning → None (no escalation)
        unit.tier = ModelTier::Reasoning;
        unit.escalation_tier = None;
        assert!(router.escalate(&unit).is_none());
    }

    #[test]
    fn test_coordinator_tier() {
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Coordinator, PrivacyLevel::Public, None);
        assert_eq!(
            router.resolve(&unit),
            ResolvedBackend::Claude { model: "opus".into() }
        );
    }

    // Test 21: resolve_backend returns correct trait objects
    #[test]
    fn test_resolve_backend_returns_trait_objects() {
        let router = TierRouter::new(TierConfig::default(), false);

        // Reasoning tier → claude backend
        let unit = test_unit(ModelTier::Reasoning, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("claude"));

        // Coordinator → claude:opus
        let unit = test_unit(ModelTier::Coordinator, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("claude"));
        assert!(backend.name().contains("opus"));

        // Mechanical with Ollama enabled → ollama backend
        let mut config = TierConfig::default();
        config.mechanical.enabled = true;
        let router = TierRouter::new(config, true);
        let unit = test_unit(ModelTier::Mechanical, PrivacyLevel::Public, None);
        let backend = router.resolve_backend(&unit).unwrap();
        assert!(backend.name().contains("ollama"));

        // Blocked → returns Err
        let router = TierRouter::new(TierConfig::default(), false);
        let unit = test_unit(ModelTier::Execution, PrivacyLevel::LocalOnly, None);
        assert!(router.resolve_backend(&unit).is_err());
    }
}
