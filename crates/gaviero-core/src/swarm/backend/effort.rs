//! Which effort levels a model offers, and whether an effort name exists.
//!
//! The vocabulary is provider-neutral; what each model accepts is not:
//!
//! | Provider | Source |
//! |---|---|
//! | Codex | the installed CLI's catalog, else [`super::codex`]'s static table |
//! | Claude | Claude Code's model-config docs — the CLI lists no per-model data |
//! | DeepSeek / dsh | `GET /models` (`supported_levels`) |
//! | Cursor | none: effort is part of the model id |
//! | Ollama / local | none: effort is not used |
//!
//! A wrong pair never reaches a provider. A script is refused before any agent
//! starts ([`effort_problem`], from `pipeline::preflight_plan_models` and
//! fan-out), and chat refuses or moves the level ([`fit_effort`]).

use super::{codex, deepseek};

/// Reasoning levels, weakest first.
const LEVEL_LADDER: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// Names that choose no level: gaviero sends the provider nothing.
const EFFORT_CONTROLS: &[&str] = &["auto", "off"];

/// True when `name` is an effort gaviero understands, on any provider.
pub fn is_known_effort(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    LEVEL_LADDER.contains(&name.as_str()) || EFFORT_CONTROLS.contains(&name.as_str())
}

/// Every effort name, for diagnostics.
pub fn known_effort_names() -> String {
    EFFORT_CONTROLS
        .iter()
        .chain(LEVEL_LADDER)
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// How a model takes effort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffortSupport {
    /// These levels, weakest first.
    Levels(Vec<String>),
    /// Effort is chosen by picking a model id (`cursor:grok-4.7-high`).
    InModelId,
    /// The model does not use effort.
    Ignored,
}

/// What `model_spec` (`provider:model`) accepts.
pub fn effort_support(model_spec: &str) -> EffortSupport {
    let (provider, model) = model_spec.trim().split_once(':').unwrap_or(("", ""));
    match provider {
        "codex" | "codex-app-server" => EffortSupport::Levels(codex::codex_effort_levels(model)),
        "claude" => claude_support(model),
        "deepseek" | "dsh" => EffortSupport::Levels(owned(&["low", "high", "max"])),
        "cursor" => EffortSupport::InModelId,
        _ => EffortSupport::Ignored,
    }
}

fn owned(levels: &[&str]) -> Vec<String> {
    levels.iter().map(|level| level.to_string()).collect()
}

/// Per Claude Code's docs: current models take the full list, the 4.6
/// generation has no `xhigh`, and older models take no effort at all.
fn claude_support(model: &str) -> EffortSupport {
    let model = model.to_ascii_lowercase();
    if ["-4-5", "-4-2", "-4-1", "-4-0", "-3-"]
        .iter()
        .any(|old| model.contains(old))
    {
        return EffortSupport::Ignored;
    }
    if model.contains("-4-6") {
        return EffortSupport::Levels(owned(&["low", "medium", "high", "max"]));
    }
    EffortSupport::Levels(owned(&["low", "medium", "high", "xhigh", "max"]))
}

/// The strongest offered level at or below `requested`, else the weakest
/// level above it. `None` when `requested` is not a level or none is offered.
pub fn nearest_level(requested: &str, offered: &[String]) -> Option<&'static str> {
    let is_offered = |level: &&&str| offered.iter().any(|o| o == **level);
    let rank = LEVEL_LADDER.iter().position(|level| *level == requested)?;
    LEVEL_LADDER[..=rank]
        .iter()
        .rev()
        .find(is_offered)
        .or_else(|| LEVEL_LADDER[rank + 1..].iter().find(is_offered))
        .copied()
}

/// The `--effort` value for the Claude CLI, which lists
/// `low|medium|high|xhigh|max`. The CLI itself steps a real level down for
/// the active model; names outside its list are translated here. `None` sends
/// no flag.
pub fn claude_cli_effort(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" | "ultra" => Some("max"),
        _ => None,
    }
}

/// The level `model_spec` will really run `effort` at.
fn resolved_level(model_spec: &str, effort: &str, offered: &[String]) -> Option<&'static str> {
    if matches!(
        model_spec.trim().split_once(':').map(|(provider, _)| provider),
        Some("deepseek" | "dsh")
    ) {
        return deepseek::reasoning_effort_for_deepseek(effort);
    }
    nearest_level(effort, offered)
}

/// Why `model_spec` cannot run at `effort`, or `None` when it can. `auto` and
/// `off` choose no level, so they suit every model.
pub fn effort_problem(model_spec: &str, effort: &str) -> Option<String> {
    let effort = effort.trim().to_ascii_lowercase();
    if EFFORT_CONTROLS.contains(&effort.as_str()) {
        return None;
    }
    match effort_support(model_spec) {
        EffortSupport::Levels(offered) if offered.contains(&effort) => None,
        EffortSupport::Levels(offered) => Some(format!(
            "`{model_spec}` has no `{effort}` effort (offers: {})",
            offered.join(", ")
        )),
        EffortSupport::InModelId => Some(format!(
            "`{model_spec}` takes no `effort {effort}`: Cursor sets effort in the model id \
             (see `agent --list-models`)"
        )),
        EffortSupport::Ignored => Some(format!(
            "`{model_spec}` takes no `effort {effort}`: this model does not use effort"
        )),
    }
}

/// The closest valid effort for `model_spec`: `effort` itself when the model
/// has it, its nearest level otherwise, and `auto` when the model takes none
/// or the name is not an effort.
pub fn fit_effort(model_spec: &str, effort: &str) -> &'static str {
    let effort = effort.trim().to_ascii_lowercase();
    if let Some(control) = EFFORT_CONTROLS.iter().find(|c| **c == effort) {
        return control;
    }
    match effort_support(model_spec) {
        EffortSupport::Levels(offered) => {
            resolved_level(model_spec, &effort, &offered).unwrap_or("auto")
        }
        EffortSupport::InModelId | EffortSupport::Ignored => "auto",
    }
}

/// One line describing what `model_spec` accepts.
pub fn effort_summary(model_spec: &str) -> String {
    match effort_support(model_spec) {
        EffortSupport::Levels(offered) => offered.join(", "),
        EffortSupport::InModelId => {
            "effort is part of the model id; choose a variant with /model".to_string()
        }
        EffortSupport::Ignored => "this model does not use effort".to_string(),
    }
}

/// Completions for an effort argument: the model's levels, then `auto` and
/// `off`. A model with no levels to choose offers nothing.
pub fn effort_completions(model_spec: &str, partial: &str) -> Vec<String> {
    let EffortSupport::Levels(offered) = effort_support(model_spec) else {
        return Vec::new();
    };
    let partial = partial.trim().to_ascii_lowercase();
    offered
        .into_iter()
        .chain(EFFORT_CONTROLS.iter().map(|c| c.to_string()))
        .filter(|name| name.starts_with(&partial))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(spec: &str) -> Vec<String> {
        match effort_support(spec) {
            EffortSupport::Levels(levels) => levels,
            other => panic!("{spec}: {other:?}"),
        }
    }

    #[test]
    fn known_names_cover_controls_and_levels_in_any_case() {
        for name in ["off", "auto", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"] {
            assert!(is_known_effort(name), "{name}");
        }
        assert!(is_known_effort(" HIGH "));
        assert!(!is_known_effort("hgih"));
        assert!(!is_known_effort(""));
    }

    #[test]
    fn each_provider_reports_its_own_levels() {
        assert_eq!(levels("deepseek:deepseek-flash"), ["low", "high", "max"]);
        assert_eq!(levels("dsh:deepseek-v4-pro"), ["low", "high", "max"]);
        assert_eq!(levels("claude:opus"), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(levels("claude:opus[1m]"), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(levels("claude:claude-opus-5-5"), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(levels("claude:claude-opus-4-8"), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(levels("claude:claude-sonnet-4-6"), ["low", "medium", "high", "max"]);
        assert_eq!(effort_support("claude:claude-haiku-4-5"), EffortSupport::Ignored);
        assert_eq!(effort_support("claude:claude-sonnet-4-20250514"), EffortSupport::Ignored);
        assert_eq!(effort_support("cursor:composer-2.5"), EffortSupport::InModelId);
        assert_eq!(effort_support("ollama:qwen2.5-coder:7b"), EffortSupport::Ignored);
    }

    #[test]
    fn codex_levels_follow_the_family() {
        assert_eq!(levels("codex:gpt-6.1-sol").last().unwrap(), "ultra");
        assert_eq!(levels("codex:gpt-6-luna").last().unwrap(), "max");
        assert_eq!(levels("codex:gpt-5.5").last().unwrap(), "xhigh");
        assert_eq!(levels("codex-app-server:gpt-6-astra").last().unwrap(), "ultra");
    }

    #[test]
    fn nearest_level_steps_down_then_up() {
        let offered = owned(&["low", "medium", "high", "max"]);
        assert_eq!(nearest_level("xhigh", &offered), Some("high"));
        assert_eq!(nearest_level("ultra", &offered), Some("max"));
        assert_eq!(nearest_level("minimal", &offered), Some("low"));
        assert_eq!(nearest_level("high", &offered), Some("high"));
        assert_eq!(nearest_level("hgih", &offered), None);
        assert_eq!(nearest_level("high", &[]), None);
    }

    #[test]
    fn claude_cli_only_receives_levels_it_lists() {
        assert_eq!(claude_cli_effort("ultra"), Some("max"));
        assert_eq!(claude_cli_effort("minimal"), Some("low"));
        assert_eq!(claude_cli_effort("XHIGH"), Some("xhigh"));
        for none in ["off", "auto", "", "hgih"] {
            assert_eq!(claude_cli_effort(none), None, "{none}");
        }
    }

    #[test]
    fn an_offered_level_or_a_control_is_no_problem() {
        assert_eq!(effort_problem("claude:opus", "max"), None);
        assert_eq!(effort_problem("cursor:composer-2.5", "auto"), None);
        assert_eq!(effort_problem("ollama:llama3", "off"), None);
    }

    #[test]
    fn wrong_pairs_are_problems() {
        for (spec, effort) in [
            ("codex:gpt-6-luna", "ultra"),
            ("dsh:deepseek-flash", "medium"),
            ("deepseek:deepseek-v4-pro", "xhigh"),
            ("claude:opus", "ultra"),
            ("claude:claude-sonnet-4-6", "xhigh"),
            ("cursor:composer-2.5", "high"),
            ("ollama:llama3", "high"),
            ("claude:opus", "hgih"),
        ] {
            assert!(effort_problem(spec, effort).is_some(), "{spec} {effort}");
        }
        let cursor = effort_problem("cursor:grok-4.7-high", "xhigh").unwrap();
        assert!(cursor.contains("model id"), "{cursor}");
    }

    #[test]
    fn fit_keeps_valid_values_and_moves_the_rest() {
        assert_eq!(fit_effort("claude:opus", "high"), "high");
        assert_eq!(fit_effort("claude:opus", "ultra"), "max");
        assert_eq!(fit_effort("claude:opus", "off"), "off");
        assert_eq!(fit_effort("codex:gpt-6-luna", "ultra"), "max");
        // DeepSeek rounds medium up, unlike the step-down providers.
        assert_eq!(fit_effort("dsh:deepseek-flash", "medium"), "high");
        assert_eq!(fit_effort("deepseek:deepseek-flash", "minimal"), "low");
        assert_eq!(fit_effort("cursor:composer-2.5", "high"), "auto");
        assert_eq!(fit_effort("ollama:llama3", "max"), "auto");
        assert_eq!(fit_effort("claude:opus", "hgih"), "auto");
        assert_eq!(fit_effort("claude:opus", ""), "auto");
    }

    #[test]
    fn a_fitted_effort_is_never_a_problem() {
        let specs = [
            "claude:opus",
            "claude:claude-sonnet-4-6",
            "claude:claude-haiku-4-5",
            "codex:gpt-6.1-sol",
            "codex:gpt-6-luna",
            "codex:gpt-5.5",
            "dsh:deepseek-flash",
            "deepseek:deepseek-v4-pro",
            "cursor:composer-2.5",
            "ollama:llama3",
        ];
        let efforts = [
            "off", "auto", "minimal", "low", "medium", "high", "xhigh", "max", "ultra", "hgih", "",
        ];
        for spec in specs {
            for effort in efforts {
                let fitted = fit_effort(spec, effort);
                assert_eq!(effort_problem(spec, fitted), None, "{spec} {effort} -> {fitted}");
            }
        }
    }

    #[test]
    fn completions_filter_by_prefix_and_add_controls() {
        assert_eq!(
            effort_completions("dsh:deepseek-flash", ""),
            ["low", "high", "max", "auto", "off"]
        );
        assert_eq!(effort_completions("claude:opus", "h"), ["high"]);
        assert_eq!(effort_completions("claude:opus", "X"), ["xhigh"]);
        assert!(effort_completions("cursor:composer-2.5", "").is_empty());
        assert!(effort_completions("ollama:llama3", "").is_empty());
    }

    #[test]
    fn summary_describes_each_kind_of_model() {
        assert_eq!(effort_summary("dsh:deepseek-flash"), "low, high, max");
        assert!(effort_summary("cursor:composer-2.5").contains("model id"));
        assert!(effort_summary("ollama:llama3").contains("does not use"));
    }
}
