//! Model profiles: one provider-agnostic script, many sets of models.
//!
//! A profile is the only place a tier gets a model. A script names tiers:
//! every agent declares one, a workflow's `tiers { }` block or `--tier`
//! moves an agent to another, and a list item's `tier` moves its clone.

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

fn compile_with(
    script: &Path,
    profile: Option<&SelectedProfile>,
    agent_tiers: &[(String, String)],
    params: &[(String, String)],
) -> Result<CompiledPlan, String> {
    compile_file_with_profile(script, profile, None, None, &[], agent_tiers, params)
        .map_err(|report| format!("{report:?}"))
}

fn compile(
    script: &Path,
    profile: Option<&SelectedProfile>,
    params: &[(String, String)],
) -> Result<CompiledPlan, String> {
    compile_with(script, profile, &[], params)
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

fn pair(model: &str, effort: &str) -> (Option<String>, Option<String>) {
    (Some(model.into()), Some(effort.into()))
}

#[test]
fn the_same_script_runs_on_different_providers_per_profile() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let claude = chosen(write(dir.path(), "claude.gaviero", CLAUDE_PROFILE));
    let mixed = chosen(write(dir.path(), "mixed.gaviero", MIXED_PROFILE));

    let on_claude = models(&compile(&script, Some(&claude), &[]).unwrap());
    assert_eq!(on_claude["worker"], pair("claude:haiku", "low"));
    assert_eq!(on_claude["planner"], pair("claude:fable", "max"));

    let on_mixed = models(&compile(&script, Some(&mixed), &[]).unwrap());
    assert_eq!(on_mixed["worker"], pair("dsh:deepseek-flash", "high"));
    assert_eq!(on_mixed["planner"], pair("codex:gpt-6-astra", "xhigh"));
}

#[test]
fn a_tier_nothing_binds_says_how_to_fix_it() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let error = compile(&script, None, &[]).unwrap_err();
    assert!(error.contains("no model profile is bound"), "{error}");
    assert!(error.contains("--profile"), "{error}");

    let partial = chosen(write(
        dir.path(),
        "p.gaviero",
        "client small { model \"claude:haiku\" }\ntier low small\n",
    ));
    let error = compile(&script, Some(&partial), &[]).unwrap_err();
    assert!(error.contains("uses tier `top`"), "{error}");
    assert!(error.contains("it defines: low"), "{error}");
}

/// A script cannot choose a model itself: no clients, no tier lines, no
/// agent bound to a client — so a profile can never be shadowed.
#[test]
fn a_script_cannot_declare_its_own_models() {
    let dir = tempfile::tempdir().unwrap();
    let profile = chosen(write(dir.path(), "p.gaviero", CLAUDE_PROFILE));
    for (body, needle) in [
        (
            "client pinned { model \"claude:sonnet\" }\nagent worker { tier low prompt \"w\" }\n",
            "cannot declare `client pinned`",
        ),
        (
            "tier low small\nagent worker { tier low prompt \"w\" }\n",
            "cannot declare `tier low small`",
        ),
        ("agent worker { client small prompt \"w\" }\n", "found 'client'"),
    ] {
        let script = write(dir.path(), "s.gaviero", body);
        let error = compile(&script, Some(&profile), &[]).unwrap_err();
        assert!(error.contains(needle), "{body}: {error}");
    }
}

/// With every agent declaring a tier, a `default` client means nothing; a
/// profile that marks one is told so.
#[test]
fn a_profile_default_client_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let profile = chosen(write(
        dir.path(),
        "p.gaviero",
        "client small { model \"claude:haiku\" default }\nclient big { model \"claude:opus\" }\ntier low small\ntier top big\n",
    ));
    let error = compile(&script, Some(&profile), &[]).unwrap_err();
    assert!(error.contains("marked `default`"), "{error}");
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
        r#"client c { model "claude:opus" } tier low c agent stray { tier t prompt "x" }"#,
    ));
    let error = compile(&script, Some(&profile), &[]).unwrap_err();
    assert!(error.contains("only declare"), "{error}");
}

#[test]
fn a_profile_tier_must_use_the_profiles_own_client() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
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

/// `tiered_review.gaviero` uses all three ways a tier is chosen: an agent's
/// own tier (`gather`, the reviewer template), the workflow's `tiers { }`
/// block (`verdict`) and a roster item's tier (`second`).
#[test]
fn shipped_profiles_drive_the_agnostic_example() {
    let script = examples().join("tiered_review.gaviero");
    let expectations = [
        ("claude", "claude:haiku", "claude:sonnet", "claude:opus", "claude:fable"),
        ("codex", "codex:gpt-6-luna", "codex:gpt-6.1-terra", "codex:gpt-6.1-sol", "codex:gpt-6-astra"),
        ("mixed", "dsh:deepseek-flash", "codex:gpt-6.1-terra", "claude:opus", "claude:fable"),
    ];
    for (name, low, medium, high, top) in expectations {
        let profile = chosen(examples().join("profiles").join(format!("{name}.gaviero")));
        let plan = compile(&script, Some(&profile), &[]).unwrap();
        assert_eq!(model_of(&plan, "gather"), low, "{name}: agent tier");
        assert_eq!(model_of(&plan, "verdict"), medium, "{name}: workflow tiers block");
        assert_eq!(model_of(&plan, "first-refine"), high, "{name}: template tier");
        assert_eq!(model_of(&plan, "second-refine"), top, "{name}: item tier");
    }
}

/// A run moves an agent with `--tier` and a roster item with `--param`;
/// both name tiers, never models.
#[test]
fn a_run_moves_agents_and_items_by_tier() {
    let script = examples().join("tiered_review.gaviero");
    let profile = chosen(examples().join("profiles/claude.gaviero"));

    let tiers = vec![("verdict".to_string(), "top".to_string())];
    let params = vec![("panel".to_string(), "only=low,plain".to_string())];
    let plan = compile_with(&script, Some(&profile), &tiers, &params).unwrap();
    assert_eq!(model_of(&plan, "verdict"), "claude:fable", "--tier beats the block");
    assert_eq!(model_of(&plan, "only-refine"), "claude:haiku");
    assert_eq!(model_of(&plan, "plain-refine"), "claude:opus", "template tier");

    for bad in ["only=tier:low", "pinned=codex:gpt-5.5@high"] {
        let params = vec![("panel".to_string(), bad.to_string())];
        let error = compile(&script, Some(&profile), &params).unwrap_err();
        assert!(error.contains("id=<tier>"), "{bad}: {error}");
    }
}

/// A profile client's model + effort pair is checked when a run starts.
fn preflight(profile: &str, script: &str) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let profile = chosen(write(dir.path(), "p.gaviero", profile));
    let script = write(dir.path(), "s.gaviero", script);
    let plan = compile(&script, Some(&profile), &[]).expect("compiles: the check is at run start");
    gaviero_core::swarm::pipeline::preflight_plan_models(&plan, "claude:sonnet", None)
        .map_err(|e| e.to_string())
}

#[test]
fn a_wrong_model_and_effort_pair_never_starts() {
    for (model, effort, expected) in [
        ("dsh:deepseek-flash", "medium", "no `medium` effort"),
        ("deepseek:deepseek-v4-pro", "xhigh", "no `xhigh` effort"),
        ("claude:opus", "ultra", "no `ultra` effort"),
        ("cursor:grok-4.7-high", "high", "model id"),
        ("ollama:llama3", "high", "does not use effort"),
    ] {
        let error = preflight(
            &format!("client c {{ model \"{model}\" effort {effort} }}\ntier x c\n"),
            "agent worker { tier x prompt \"p\" }\n",
        )
        .unwrap_err();
        assert!(error.contains(expected), "{model} {effort}: {error}");
        assert!(error.contains("agent 'worker'"), "names the agent: {error}");
    }
}

#[test]
fn a_right_pair_and_choosing_no_level_both_start() {
    for (model, effort) in [
        ("dsh:deepseek-flash", "high"),
        ("claude:opus", "xhigh"),
        ("cursor:grok-4.7-high", "auto"),
        ("ollama:llama3", "off"),
    ] {
        preflight(
            &format!("client c {{ model \"{model}\" effort {effort} }}\ntier x c\n"),
            "agent worker { tier x prompt \"p\" }\n",
        )
        .unwrap_or_else(|e| panic!("{model} {effort}: {e}"));
    }
}

#[test]
fn an_unused_wrong_tier_in_a_profile_does_not_block_a_run() {
    preflight(
        "client unused { model \"cursor:composer-2.5\" effort high }\n\
         client used { model \"claude:opus\" effort high }\n\
         tier spare unused\n\
         tier main used\n",
        "agent worker { tier main prompt \"p\" }\n",
    )
    .unwrap();
}

#[test]
fn a_roster_item_tier_is_checked_too() {
    let error = preflight(
        "client j { model \"claude:opus\" }\n\
         client bad { model \"dsh:deepseek-flash\" effort medium }\n\
         tier main j\n\
         tier flawed bad\n",
        "agent tpl { tier main template true prompt \"p\" }\n\
         agent judge { tier main prompt \"j\" }\n\
         workflow w { steps [ loop { reviewers [ { id \"a\" tier flawed } ] template_refine tpl until agent judge max_iterations 1 } ] }\n",
    )
    .unwrap_err();
    assert!(error.contains("agent 'a-refine'"), "{error}");
}

#[test]
fn the_form_shows_list_params_in_override_syntax() {
    let inspection = gaviero_dsl::inspect_file(&examples().join("tiered_review.gaviero")).unwrap();
    let params = &inspection.workflows[0].params;
    assert_eq!(params.len(), 1, "params are lists only");
    assert_eq!(params[0].name, "panel");
    assert_eq!(params[0].default.as_deref(), Some("first,second=top"));
    assert_eq!(inspection.tiers, ["low", "high", "medium", "top"]);
}

#[test]
fn compile_file_without_a_profile_names_the_fix() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", "agent w { tier low prompt \"p\" }\n");
    let error = gaviero_dsl::compile_file(&script, None, None, &[], &[], &[]).unwrap_err();
    assert!(format!("{error:?}").contains("no model profile is bound"), "{error:?}");
}

/// A run can point a tier at another client of the same profile, or add a
/// tier bound to one, without editing the profile file.
#[test]
fn a_run_rebinds_tiers_to_other_clients_of_the_profile() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "s.gaviero", AGNOSTIC_SCRIPT);
    let claude = chosen(write(dir.path(), "claude.gaviero", CLAUDE_PROFILE));
    let rebind = |pairs: &[(&str, &str)], agent_tiers: &[(String, String)]| {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(t, c)| (t.to_string(), c.to_string()))
            .collect();
        gaviero_dsl::compile_file_with_models(
            &script,
            Some(&claude),
            &gaviero_dsl::TierOverride::from_pairs(&pairs, &[]),
            None,
            None,
            &[],
            agent_tiers,
            &[],
        )
        .map_err(|report| format!("{report:?}"))
    };

    let plan = rebind(&[("low", "big")], &[]).unwrap();
    assert_eq!(models(&plan)["worker"], pair("claude:fable", "max"));
    assert_eq!(models(&plan)["planner"], pair("claude:fable", "max"));

    // A tier the profile lacks is added, and an agent can be moved onto it.
    let moved = [("planner".to_string(), "extra".to_string())];
    let plan = rebind(&[("extra", "small")], &moved).unwrap();
    assert_eq!(models(&plan)["planner"], pair("claude:haiku", "low"));

    let error = rebind(&[("low", "nope")], &[]).unwrap_err();
    assert!(error.contains("no client `nope`"), "{error}");
    assert!(error.contains("it has: small"), "{error}");
}

/// A run can change a tier's effort, alone or with its client, without
/// touching the client the other tiers share.
#[test]
fn a_run_changes_the_effort_of_one_tier() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "s.gaviero",
        "agent worker { tier low prompt \"w\" }\nagent other { tier same prompt \"o\" }\n",
    );
    let profile = chosen(write(
        dir.path(),
        "p.gaviero",
        concat!(
            "client small { model \"claude:haiku\" effort low }\n",
            "client big { model \"claude:fable\" effort max }\n",
            "tier low small\ntier same small\n",
        ),
    ));
    let run = |clients: &[(&str, &str)], efforts: &[(&str, &str)]| {
        let owned = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        };
        gaviero_dsl::compile_file_with_models(
            &script,
            Some(&profile),
            &gaviero_dsl::TierOverride::from_pairs(&owned(clients), &owned(efforts)),
            None,
            None,
            &[],
            &[],
            &[],
        )
        .map(|plan| models(&plan))
        .map_err(|report| format!("{report:?}"))
    };

    let effort_only = run(&[], &[("low", "high")]).unwrap();
    assert_eq!(effort_only["worker"], pair("claude:haiku", "high"));
    assert_eq!(effort_only["other"], pair("claude:haiku", "low"), "the shared client is untouched");

    let both = run(&[("low", "big")], &[("low", "medium")]).unwrap();
    assert_eq!(both["worker"], pair("claude:fable", "medium"));

    let error = run(&[], &[("low", "turbo")]).unwrap_err();
    assert!(error.contains("unknown effort"), "{error}");
    let error = run(&[], &[("nowhere", "high")]).unwrap_err();
    assert!(error.contains("no tier `nowhere`"), "{error}");
}
