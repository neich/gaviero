//! `effort_support` must report what the installed Codex CLI really lists.
//!
//! Integration tests link the library without `cfg(test)`, so this reaches the
//! live catalog. Ignored because it needs the `codex` CLI; it costs no tokens.
//!
//! ```bash
//! cargo test -p gaviero-core --test codex_effort_catalog -- --ignored
//! ```

use gaviero_core::swarm::backend::effort::{EffortSupport, effort_support};

#[test]
#[ignore = "requires the codex CLI on PATH"]
fn levels_match_the_installed_codex_catalog() {
    let output = gaviero_core::util::spawn::agent_command_std("codex")
        .args(["debug", "models"])
        .output()
        .expect("codex runs");
    assert!(output.status.success());
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let models = catalog["models"].as_array().unwrap();
    assert!(!models.is_empty());
    for model in models {
        let slug = model["slug"].as_str().unwrap();
        let expected: Vec<String> = model["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level["effort"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            effort_support(&format!("codex:{slug}")),
            EffortSupport::Levels(expected),
            "{slug}"
        );
    }
}
