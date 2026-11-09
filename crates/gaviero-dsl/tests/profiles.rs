//! Model profiles: one provider-agnostic script, many sets of models.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use gaviero_core::script_profiles::{ProfilePrecedence, SelectedProfile};
use gaviero_dsl::{CompiledPlan, compile_file_with_profile};

const AGNOSTIC_SCRIPT: &str = r#"
agent worker { tier low prompt "w" }
agent planner { tier top prompt "p" }
"#;

const CLAUDE_PROFILE: &str = r#"
client small { model "claude:haiku" effort low }
client big { model "claude:fable" effort max }
tier low small
tier top big
"#;

const MIXED_PROFILE: &str = r#"
client small { model "dsh:deepseek-flash" effort high }
client big { model "codex:gpt-6-astra" effort xhigh }
tier low small
tier top big
"#;

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

fn chosen(path: PathBuf) -> SelectedProfile {
    SelectedProfile {
        name: "p".into(),
        path,
        precedence: ProfilePrecedence::OverridesScript,
    }
}

fn implicit(path: PathBuf) -> SelectedProfile {
    SelectedProfile {
        name: "default".into(),
        path,
        precedence: ProfilePrecedence::FillsGaps,
    }
}

fn compile(
    script: &Path,
    profile: Option<&SelectedProfile>,
    params: &[(String, String)],
) -> Result<CompiledPlan, String> {
    compile_file_with_profile(script, profile, None, None, &[], &[], params)
        .map_err(|report| format!("{report:?}"))
}

/// (model, effort) of every unit, by id.
fn models(plan: &CompiledPlan) -> HashMap<String, (Option<String>, Option<String>)> {
    plan.work_units_ordered()
        .unwrap()
        .into_iter()
        .chain(plan.loop_judge_units.iter().cloned())
        .map(|u| (u.id.clone(), (u.model.clone(), u.effort.clone())))
        .collect()
}

fn model_of(plan: &CompiledPlan, id: &str) -> String {
    models(plan)[id].0.clone().expect("unit has a model")
}

#[test]
fn the_same_script_runs_on_different_providers_per_profile() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let claude = chosen(write(dir.path(), "claude.gaviero", CLAUDE_PROFILE));
    let mixed = chosen(write(dir.path(), "mixed.gaviero", MIXED_PROFILE));

    let on_claude = models(&compile(&script, Some(&claude), &[]).unwrap());
    assert_eq!(
        on_claude["worker"],
        (Some("claude:haiku".into()), Some("low".into()))
    );
    assert_eq!(
        on_claude["planner"],
        (Some("claude:fable".into()), Some("max".into()))
    );

    let on_mixed = models(&compile(&script, Some(&mixed), &[]).unwrap());
    assert_eq!(
        on_mixed["worker"],
        (Some("dsh:deepseek-flash".into()), Some("high".into()))
    );
    assert_eq!(
        on_mixed["planner"],
        (Some("codex:gpt-6-astra".into()), Some("xhigh".into()))
    );
}

#[test]
fn a_tier_nothing_binds_says_how_to_fix_it() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let error = compile(&script, None, &[]).unwrap_err();
    assert!(error.contains("undefined tier alias"), "{error}");
    assert!(error.contains("profile"), "{error}");
}

#[test]
fn a_chosen_profile_replaces_the_scripts_own_binding() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        r#"
        client pinned { model "claude:sonnet" }
        tier low pinned
        agent worker { tier low prompt "w" }
        "#,
    );
    let profile = chosen(write(dir.path(), "p.gaviero", CLAUDE_PROFILE));
    let plan = compile(&script, Some(&profile), &[]).unwrap();
    assert_eq!(model_of(&plan, "worker"), "claude:haiku");
}

#[test]
fn the_implicit_default_only_fills_what_the_script_leaves_open() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        r#"
        client pinned { model "claude:sonnet" }
        tier low pinned
        agent worker { tier low prompt "w" }
        agent planner { tier top prompt "p" }
        "#,
    );
    let profile = implicit(write(dir.path(), "default.gaviero", CLAUDE_PROFILE));
    let plan = compile(&script, Some(&profile), &[]).unwrap();
    assert_eq!(model_of(&plan, "worker"), "claude:sonnet", "script keeps its tier");
    assert_eq!(model_of(&plan, "planner"), "claude:fable", "profile fills the gap");
}

#[test]
fn a_concrete_client_binding_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        r#"
        client pinned { model "claude:sonnet" }
        agent worker { client pinned prompt "w" }
        "#,
    );
    let profile = chosen(write(dir.path(), "p.gaviero", CLAUDE_PROFILE));
    let plan = compile(&script, Some(&profile), &[]).unwrap();
    assert_eq!(model_of(&plan, "worker"), "claude:sonnet");
}

#[test]
fn profile_and_script_may_use_the_same_client_names() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        r#"
        client small { model "codex:gpt-5.5" }
        agent pinned { client small prompt "p" }
        agent worker { tier low prompt "w" }
        "#,
    );
    let profile = chosen(write(dir.path(), "p.gaviero", CLAUDE_PROFILE));
    let plan = compile(&script, Some(&profile), &[]).unwrap();
    assert_eq!(model_of(&plan, "pinned"), "codex:gpt-5.5");
    assert_eq!(model_of(&plan, "worker"), "claude:haiku");
}

#[test]
fn profile_default_client_covers_unbound_agents() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"
        client main { model "claude:opus" default }
        tier high main
    "#;
    let script = write(
        dir.path(),
        "s.gaviero",
        r#"
        client own { model "claude:sonnet" default }
        agent worker { prompt "w" }
        "#,
    );
    let path = write(dir.path(), "p.gaviero", body);

    let plan = compile(&script, Some(&chosen(path.clone())), &[]).unwrap();
    assert_eq!(model_of(&plan, "worker"), "claude:opus", "chosen profile wins");

    let plan = compile(&script, Some(&implicit(path)), &[]).unwrap();
    assert_eq!(model_of(&plan, "worker"), "claude:sonnet", "script default stays");
}

#[test]
fn a_profile_may_include_shared_clients() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    write(dir.path(), "pool.gaviero", r#"client big { model "claude:opus" } client small { model "claude:haiku" }"#);
    let profile = chosen(write(
        dir.path(),
        "p.gaviero",
        "include \"pool.gaviero\"\ntier low small\ntier top big\n",
    ));
    let plan = compile(&script, Some(&profile), &[]).unwrap();
    assert_eq!(model_of(&plan, "planner"), "claude:opus");
}

#[test]
fn a_profile_is_only_a_model_table() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let profile = chosen(write(
        dir.path(),
        "p.gaviero",
        r#"client c { model "claude:opus" } tier low c agent stray { prompt "x" }"#,
    ));
    let error = compile(&script, Some(&profile), &[]).unwrap_err();
    assert!(error.contains("only declare"), "{error}");
}

#[test]
fn a_profile_tier_must_use_the_profiles_own_client() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        "client theirs { model \"claude:opus\" }\nagent w { client theirs prompt \"p\" }\n",
    );
    let profile = chosen(write(dir.path(), "p.gaviero", "tier low theirs\n"));
    let error = compile(&script, Some(&profile), &[]).unwrap_err();
    assert!(error.contains("does not declare"), "{error}");
}

#[test]
fn a_profile_without_tiers_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let profile = chosen(write(dir.path(), "p.gaviero", r#"client c { model "claude:opus" }"#));
    let error = compile(&script, Some(&profile), &[]).unwrap_err();
    assert!(error.contains("no `tier"), "{error}");
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

#[test]
fn shipped_profiles_drive_the_agnostic_example_through_params_and_roster() {
    let script = examples().join("tiered_review.gaviero");
    let expectations = [
        ("claude", "claude:haiku", "claude:sonnet", "claude:opus", "claude:fable"),
        ("codex", "codex:gpt-5.6-luna", "codex:gpt-5.6-terra", "codex:gpt-5.6-sol", "codex:gpt-6-astra"),
        ("mixed", "dsh:deepseek-flash", "codex:gpt-5.6-terra", "claude:opus", "claude:fable"),
    ];
    for (name, low, medium, high, top) in expectations {
        let profile = chosen(examples().join("profiles").join(format!("{name}.gaviero")));
        let plan = compile(&script, Some(&profile), &[]).unwrap();
        assert_eq!(model_of(&plan, "gather"), low, "{name}: agent tier");
        assert_eq!(model_of(&plan, "verdict"), medium, "{name}: client param");
        assert_eq!(model_of(&plan, "first-refine"), high, "{name}: roster");
        assert_eq!(model_of(&plan, "second-refine"), top, "{name}: roster");
    }
}

#[test]
fn param_overrides_can_move_an_agent_to_another_tier_or_a_concrete_model() {
    let script = examples().join("tiered_review.gaviero");
    let profile = chosen(examples().join("profiles/claude.gaviero"));

    let params = vec![
        ("judge".to_string(), "tier:top".to_string()),
        ("panel".to_string(), "only=tier:low,pinned=codex:gpt-5.5@high".to_string()),
    ];
    let plan = compile(&script, Some(&profile), &params).unwrap();
    assert_eq!(model_of(&plan, "verdict"), "claude:fable");
    assert_eq!(model_of(&plan, "only-refine"), "claude:haiku");
    assert_eq!(model_of(&plan, "pinned-refine"), "codex:gpt-5.5");
}

#[test]
fn a_tier_param_cannot_also_set_effort() {
    let script = examples().join("tiered_review.gaviero");
    let profile = chosen(examples().join("profiles/claude.gaviero"));

    let client = vec![("judge".to_string(), "tier:top@low".to_string())];
    let error = compile(&script, Some(&profile), &client).unwrap_err();
    assert!(error.contains("cannot be combined"), "{error}");

    let roster = vec![("panel".to_string(), "a=tier:top@low".to_string())];
    let error = compile(&script, Some(&profile), &roster).unwrap_err();
    assert!(error.contains("cannot be combined"), "{error}");
}

#[test]
fn a_tier_param_needs_a_tier_name() {
    let script = examples().join("tiered_review.gaviero");
    let profile = chosen(examples().join("profiles/claude.gaviero"));
    let params = vec![("judge".to_string(), "tier:".to_string())];
    let error = compile(&script, Some(&profile), &params).unwrap_err();
    assert!(error.contains("needs a tier name"), "{error}");
}

#[test]
fn the_form_shows_tier_params_in_override_syntax() {
    let inspection = gaviero_dsl::inspect_file(&examples().join("tiered_review.gaviero")).unwrap();
    let params = &inspection.workflows[0].params;
    let default_of = |name: &str| {
        params
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.default.clone())
    };
    assert_eq!(default_of("judge").as_deref(), Some("tier:medium"));
    assert_eq!(
        default_of("panel").as_deref(),
        Some("first=tier:high,second=tier:top")
    );
}

#[test]
fn compile_file_still_works_with_no_profile() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        "client c { model \"claude:opus\" default }\nagent w { prompt \"p\" }\n",
    );
    let plan = gaviero_dsl::compile_file(&script, None, None, &[], &[], &[]).unwrap();
    assert_eq!(model_of(&plan, "w"), "claude:opus");
}
