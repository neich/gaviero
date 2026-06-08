//! Bootstrap injection policy for chat turns.
//!
//! Chat callers resolve [`BootstrapArms`] from workspace defaults, per-
//! conversation mode overrides, and one-shot slash-command arms before
//! handing them to [`ContextPlanner`]. Swarm always passes
//! [`BootstrapArms::swarm_first_turn`].

use serde::{Deserialize, Serialize};

/// Workspace / per-conversation default for first-turn bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapMode {
    /// First turn injects topology + outline + memory + impact (today's default).
    #[default]
    Auto,
    /// First turn injects topology only (same layers `/lite` drops).
    Minimal,
    /// First turn injects nothing unless a one-shot `/inject` arms it.
    Manual,
    /// Never auto-inject; only explicit `/inject` arms apply.
    None,
}

impl BootstrapMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "minimal" => Some(Self::Minimal),
            "manual" => Some(Self::Manual),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Minimal => "minimal",
            Self::Manual => "manual",
            Self::None => "none",
        }
    }
}

/// One-shot slash-command override consumed on the next dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapOneShot {
    /// `/lite` — topology only.
    Lite,
    /// `/no-inject` — suppress all bootstrap layers.
    NoInject,
    /// `/inject all` — full bootstrap (works on follow-up turns too).
    All,
}

/// Per-layer bootstrap switches for a single planner pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootstrapArms {
    pub memory: bool,
    pub outline: bool,
    pub topology: bool,
    pub impact: bool,
    /// When true, arms apply even when `turn_count > 0` (explicit `/inject`).
    pub explicit: bool,
}

impl BootstrapArms {
    pub const fn all() -> Self {
        Self {
            memory: true,
            outline: true,
            topology: true,
            impact: true,
            explicit: false,
        }
    }

    pub const fn topology_only() -> Self {
        Self {
            memory: false,
            outline: false,
            topology: true,
            impact: false,
            explicit: false,
        }
    }

    pub const fn none() -> Self {
        Self {
            memory: false,
            outline: false,
            topology: false,
            impact: false,
            explicit: false,
        }
    }

    /// Swarm work units always bootstrap on their fresh first turn.
    pub const fn swarm_first_turn() -> Self {
        Self::all()
    }

    pub fn any_layer(self) -> bool {
        self.memory || self.outline || self.topology || self.impact
    }

    pub fn merge_explicit(mut self, other: Self) -> Self {
        self.memory |= other.memory;
        self.outline |= other.outline;
        self.topology |= other.topology;
        self.impact |= other.impact;
        if other.any_layer() {
            self.explicit = true;
        }
        self
    }

    /// Arms that apply on this turn given ledger state.
    pub fn for_turn(self, is_first_turn: bool) -> Self {
        if is_first_turn || self.explicit {
            self
        } else {
            Self::none()
        }
    }
}

/// Resolve bootstrap arms for a chat dispatch.
pub fn resolve_chat_bootstrap_arms(
    mode: BootstrapMode,
    is_first_turn: bool,
    one_shot: Option<BootstrapOneShot>,
    accumulated: BootstrapArms,
) -> BootstrapArms {
    if let Some(shot) = one_shot {
        let arms = match shot {
            BootstrapOneShot::Lite => BootstrapArms {
                topology: true,
                explicit: true,
                ..BootstrapArms::none()
            },
            BootstrapOneShot::NoInject => BootstrapArms::none(),
            BootstrapOneShot::All => BootstrapArms {
                explicit: true,
                ..BootstrapArms::all()
            },
        };
        return arms.for_turn(is_first_turn);
    }

    if accumulated.explicit && accumulated.any_layer() {
        return accumulated.for_turn(is_first_turn);
    }

    if !is_first_turn {
        return BootstrapArms::none();
    }

    let arms = match mode {
        BootstrapMode::Auto => BootstrapArms::all(),
        BootstrapMode::Minimal => BootstrapArms::topology_only(),
        BootstrapMode::Manual | BootstrapMode::None => BootstrapArms::none(),
    };
    arms.for_turn(is_first_turn)
}

/// Per-layer token ceilings used when projecting bootstrap size before dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootstrapBudgets {
    pub topology: usize,
    pub outline: usize,
    pub memory: usize,
    pub impact: usize,
}

impl BootstrapBudgets {
    pub fn from_workspace(
        topology_cfg: &crate::repo_map::TopologyConfig,
        graph_budget_tokens: usize,
        memory_cfg: &crate::memory::ChatInjectionConfig,
    ) -> Self {
        Self {
            topology: if topology_cfg.enabled {
                topology_cfg.max_token_budget
            } else {
                0
            },
            outline: graph_budget_tokens,
            memory: if memory_cfg.enabled {
                memory_cfg.token_budget
            } else {
                0
            },
            // Impact is buffer-seeded and variable; cap at a fraction of graph budget.
            impact: graph_budget_tokens.min(4_000),
        }
    }
}

/// Optional measured sizes from warmed caches or the last injection pass.
#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapEstimateHints {
    pub topology_chars: Option<usize>,
    pub outline_tokens: Option<usize>,
    pub memory_tokens: Option<usize>,
    pub impact_chars: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapEstimateContext {
    pub budgets: BootstrapBudgets,
    pub hints: BootstrapEstimateHints,
}

/// Project bootstrap injection size for the given arms.
///
/// Uses measured hints when present (cached topology body, last memory
/// injection) and falls back to workspace budgets otherwise.
pub fn estimate_bootstrap_tokens(
    arms: BootstrapArms,
    budgets: &BootstrapBudgets,
    hints: &BootstrapEstimateHints,
) -> usize {
    if !arms.any_layer() {
        return 0;
    }

    let mut total = 0usize;
    if arms.topology {
        total = total.saturating_add(
            hints
                .topology_chars
                .map(|chars| chars.div_ceil(4).min(budgets.topology))
                .unwrap_or(budgets.topology),
        );
    }
    if arms.outline {
        total = total.saturating_add(hints.outline_tokens.unwrap_or(budgets.outline));
    }
    if arms.memory {
        total = total.saturating_add(hints.memory_tokens.unwrap_or(budgets.memory));
    }
    if arms.impact {
        total = total.saturating_add(
            hints
                .impact_chars
                .map(|chars| chars.div_ceil(4).min(budgets.impact))
                .unwrap_or(budgets.impact),
        );
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_first_turn_injects_all() {
        let arms = resolve_chat_bootstrap_arms(BootstrapMode::Auto, true, None, BootstrapArms::none());
        assert!(arms.memory && arms.outline && arms.topology && arms.impact);
    }

    #[test]
    fn auto_follow_up_injects_nothing() {
        let arms = resolve_chat_bootstrap_arms(BootstrapMode::Auto, false, None, BootstrapArms::none());
        assert_eq!(arms, BootstrapArms::none());
    }

    #[test]
    fn lite_is_topology_only() {
        let arms = resolve_chat_bootstrap_arms(
            BootstrapMode::Auto,
            true,
            Some(BootstrapOneShot::Lite),
            BootstrapArms::none(),
        );
        assert!(arms.topology);
        assert!(!arms.memory && !arms.outline && !arms.impact);
    }

    #[test]
    fn explicit_memory_on_follow_up() {
        let accumulated = BootstrapArms {
            memory: true,
            explicit: true,
            ..BootstrapArms::none()
        };
        let arms = resolve_chat_bootstrap_arms(BootstrapMode::Auto, false, None, accumulated);
        assert!(arms.memory);
        assert!(!arms.outline);
    }

    #[test]
    fn manual_first_turn_default_empty() {
        let arms = resolve_chat_bootstrap_arms(BootstrapMode::Manual, true, None, BootstrapArms::none());
        assert_eq!(arms, BootstrapArms::none());
    }

    #[test]
    fn estimate_lite_is_topology_only() {
        let budgets = BootstrapBudgets {
            topology: 600,
            outline: 12_000,
            memory: 1_000,
            impact: 2_000,
        };
        let arms = resolve_chat_bootstrap_arms(
            BootstrapMode::Auto,
            true,
            Some(BootstrapOneShot::Lite),
            BootstrapArms::none(),
        );
        let tok = estimate_bootstrap_tokens(arms, &budgets, &BootstrapEstimateHints::default());
        assert_eq!(tok, 600);
    }

    #[test]
    fn estimate_inject_memory_on_follow_up() {
        let budgets = BootstrapBudgets {
            topology: 600,
            outline: 12_000,
            memory: 1_000,
            impact: 2_000,
        };
        let arms = resolve_chat_bootstrap_arms(
            BootstrapMode::Auto,
            false,
            None,
            BootstrapArms {
                memory: true,
                explicit: true,
                ..BootstrapArms::none()
            },
        );
        let tok = estimate_bootstrap_tokens(arms, &budgets, &BootstrapEstimateHints::default());
        assert_eq!(tok, 1_000);
    }

    #[test]
    fn estimate_prefers_topology_hint_over_budget() {
        let budgets = BootstrapBudgets {
            topology: 600,
            outline: 0,
            memory: 0,
            impact: 0,
        };
        let hints = BootstrapEstimateHints {
            topology_chars: Some(800),
            ..BootstrapEstimateHints::default()
        };
        let tok = estimate_bootstrap_tokens(BootstrapArms::topology_only(), &budgets, &hints);
        assert_eq!(tok, 200);
    }
}
