use gaviero_core::iteration::Strategy;
use gaviero_core::script_profiles::{ProfilePrecedence, SelectedProfile};
use gaviero_core::types::ModelTier;

/// The profile every inline test compiles against: `tests/fixtures/test-profile.gaviero`.
fn test_profile() -> SelectedProfile {
    SelectedProfile {
        name: "test".into(),
        path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test-profile.gaviero"),
        precedence: ProfilePrecedence::OverridesScript,
    }
}

/// The model profile shipped with the examples (`examples/profiles/default.gaviero`).
fn example_profile() -> SelectedProfile {
    SelectedProfile {
        name: "default".into(),
        path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/profiles/default.gaviero"),
        precedence: ProfilePrecedence::FillsGaps,
    }
}

/// `gaviero_dsl::compile_file` with a profile: the examples' default profile
/// for a script under `examples/`, the test profile for any other file.
fn compile_file(
    path: &std::path::Path,
    workflow: Option<&str>,
    prompt: Option<&str>,
    vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    params: &[(String, String)],
) -> Result<gaviero_dsl::CompiledPlan, miette::Report> {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let profile = if path.starts_with(&examples) {
        example_profile()
    } else {
        test_profile()
    };
    gaviero_dsl::compile_file_with_profile(
        path,
        Some(&profile),
        workflow,
        prompt,
        vars,
        agent_tiers,
        params,
    )
}

/// `gaviero_dsl::compile` with the test profile.
fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    prompt: Option<&str>,
) -> Result<gaviero_dsl::CompiledPlan, miette::Report> {
    gaviero_dsl::compile(source, filename, Some(&test_profile()), workflow, prompt)
}

/// `gaviero_dsl::compile_with_vars` with the test profile.
fn compile_with_vars(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    prompt: Option<&str>,
    vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    params: &[(String, String)],
) -> Result<gaviero_dsl::CompiledPlan, miette::Report> {
    gaviero_dsl::compile_with_vars(
        source,
        filename,
        Some(&test_profile()),
        workflow,
        prompt,
        vars,
        agent_tiers,
        params,
    )
}

const FULL_EXAMPLE: &str = r##"
    agent researcher {
        description "Research and document the codebase architecture"
        tier opus
        scope {
            owned    ["docs/architecture.md"]
            read_only ["src/**"]
        }
        prompt #"
            Analyze the codebase and write a comprehensive architecture document.
            Focus on module structure, key abstractions, and data flows.
        "#
        max_retries 2
    }

    agent implementer {
        description "Implement the feature"
        tier sonnet
        depends_on [researcher]
        scope {
            owned    ["src/feature/"]
            read_only ["docs/architecture.md"]
        }
        prompt #"
            Based on the architecture document, implement the feature.
        "#
    }

    workflow feature_development {
        steps [researcher implementer]
        max_parallel 2
    }
"##;

#[test]
fn full_example_compiles_to_two_units() {
    let compiled =
        compile(FULL_EXAMPLE, "test.gaviero", None, None).expect("should compile without errors");
    let units = compiled.work_units_ordered().expect("toposort");

    assert_eq!(units.len(), 2, "expected 2 work units");

    // researcher
    assert_eq!(units[0].id, "researcher");
    assert_eq!(units[0].tier, ModelTier::Expensive);
    assert_eq!(units[0].model, Some("claude-opus-4-7".to_string()));
    assert!(
        units[0]
            .coordinator_instructions
            .contains("architecture document"),
        "prompt: {}",
        units[0].coordinator_instructions
    );
    assert_eq!(units[0].max_retries, 2);
    assert_eq!(units[0].scope.owned_paths, vec!["docs/architecture.md"]);
    assert_eq!(units[0].scope.read_only_paths, vec!["src/**"]);
    assert!(units[0].depends_on.is_empty());

    // implementer
    assert_eq!(units[1].id, "implementer");
    assert_eq!(units[1].tier, ModelTier::Cheap);
    assert_eq!(units[1].model, Some("claude-sonnet-4-6".to_string()));
    assert_eq!(units[1].depends_on, vec!["researcher"]);
    assert_eq!(units[1].scope.owned_paths, vec!["src/feature/"]);
}

#[test]
fn full_example_max_parallel_propagated() {
    let compiled =
        compile(FULL_EXAMPLE, "test.gaviero", None, None).expect("should compile without errors");
    assert_eq!(compiled.max_parallel, Some(2));
}

#[test]
fn lex_error_is_reported() {
    let src = "agent @ bad { }";
    let err = compile(src, "test.gaviero", None, None);
    assert!(err.is_err(), "expected error for invalid character");
    let report = format!("{:?}", err.unwrap_err());
    assert!(report.contains("unexpected"), "report: {}", report);
}

#[test]
fn undefined_tier_error_lists_the_profile_tiers() {
    let src = r#"agent x { tier ghost }"#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("ghost"), "report: {}", report);
    assert!(report.contains("it defines: local, low, opus, sonnet, t"), "report: {}", report);
}

// ── Tiers only ───────────────────────────────────────────────────

/// Every agent declares its default tier; there is no fallback client.
#[test]
fn an_agent_without_a_tier_is_an_error() {
    let err = compile(r#"agent x { description "d" }"#, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("agent `x` declares no tier"), "report: {report}");
}

/// What a tier means lives in the profile alone: a script may not declare
/// a client or a tier line, nor bind an agent to a client.
#[test]
fn a_script_cannot_choose_models_itself() {
    for (src, needle) in [
        (
            r#"client c { model "claude:opus" } agent x { tier t }"#,
            "cannot declare `client c`",
        ),
        (r#"tier fast t agent x { tier t }"#, "cannot declare `tier fast t`"),
    ] {
        let report = format!("{:?}", compile(src, "test.gaviero", None, None).unwrap_err());
        assert!(report.contains(needle), "{src}: {report}");
        assert!(report.contains("model profile"), "{src}: {report}");
    }
    // `client` is no longer an agent field at all.
    assert!(compile(r#"agent x { client c }"#, "test.gaviero", None, None).is_err());
    // Nor is a model-valued param.
    assert!(
        compile(
            r#"agent x { tier t } workflow w { param j { model "claude:opus" } steps [x] }"#,
            "test.gaviero",
            None,
            None
        )
        .is_err()
    );
}

/// Without a profile no tier is bound, and the diagnostic says how to get one.
#[test]
fn no_profile_means_no_tier() {
    let err = gaviero_dsl::compile(r#"agent x { tier t }"#, "test.gaviero", None, None, None)
        .unwrap_err();
    let report = format!("{err:?}");
    assert!(report.contains("no model profile is bound"), "report: {report}");
}

const TIERED: &str = r#"
    agent a { tier t }
    agent b { tier t }
    agent j { tier t prompt "judge" }
    workflow w {
        tiers {
            b opus
            j sonnet
        }
        steps [ a b loop { agents [a] until agent j } ]
    }
"#;

fn model_of(plan: &gaviero_dsl::CompiledPlan, id: &str) -> Option<String> {
    plan.work_units_unordered()
        .into_iter()
        .chain(plan.loop_judge_units.iter())
        .find(|u| u.id == id)
        .unwrap_or_else(|| panic!("no unit {id}"))
        .model
        .clone()
}

/// The workflow's `tiers { }` block moves an agent (and a loop judge) to
/// another tier; agents it does not name keep their own.
#[test]
fn the_workflow_tiers_block_overrides_an_agent_tier() {
    let plan = compile(TIERED, "test.gaviero", None, None).expect("compile");
    assert_eq!(model_of(&plan, "a"), None, "tier t keeps its own");
    assert_eq!(model_of(&plan, "b").as_deref(), Some("claude-opus-4-7"));
    assert_eq!(model_of(&plan, "j").as_deref(), Some("claude-sonnet-4-6"));
}

/// `--tier agent=tier` beats the block, and reaches agents it does not name.
#[test]
fn cli_tier_overrides_the_block() {
    let cli = vec![
        ("b".to_string(), "low".to_string()),
        ("a".to_string(), "sonnet".to_string()),
    ];
    let plan = compile_with_vars(TIERED, "test.gaviero", None, None, &[], &cli, &[]).unwrap();
    assert_eq!(model_of(&plan, "b").as_deref(), Some("claude:haiku"));
    assert_eq!(model_of(&plan, "a").as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(model_of(&plan, "j").as_deref(), Some("claude-sonnet-4-6"));
}

#[test]
fn tier_overrides_must_name_real_agents_and_tiers() {
    let unknown_agent = r#"agent a { tier t } workflow w { tiers { ghost opus } steps [a] }"#;
    let report = format!("{:?}", compile(unknown_agent, "t.gaviero", None, None).unwrap_err());
    assert!(report.contains("`tiers` names `ghost`"), "{report}");

    let twice = r#"agent a { tier t } workflow w { tiers { a opus a low } steps [a] }"#;
    let report = format!("{:?}", compile(twice, "t.gaviero", None, None).unwrap_err());
    assert!(report.contains("sets agent `a` twice"), "{report}");

    let cli = vec![("ghost".to_string(), "opus".to_string())];
    let err = compile_with_vars(TIERED, "t.gaviero", None, None, &[], &cli, &[]).unwrap_err();
    assert!(format!("{err:?}").contains("--tier ghost=opus"), "{err:?}");

    let cli = vec![("a".to_string(), "nope".to_string())];
    let err = compile_with_vars(TIERED, "t.gaviero", None, None, &[], &cli, &[]).unwrap_err();
    assert!(format!("{err:?}").contains("uses tier `nope`"), "{err:?}");
}

/// A list item's `tier` moves its own clone; items without one keep the
/// body agent's tier — which the `tiers { }` block may have moved.
#[test]
fn a_list_item_tier_wins_for_its_clone() {
    let src = r#"
        agent worker { tier t scope { owned ["out/{{x.id}}.md"] } }
        workflow w {
            tiers { worker sonnet }
            param items [ { id "a" } { id "b" tier opus } ]
            steps [ for x in items parallel { worker } ]
        }
    "#;
    let plan = compile(src, "test.gaviero", None, None).expect("compile");
    assert_eq!(model_of(&plan, "a-worker").as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(model_of(&plan, "b-worker").as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn no_workflow_runs_all_agents() {
    let src = r#"
        agent a { tier t description "first" }
        agent b { tier t description "second" }
    "#;
    let units = compile(src, "test.gaviero", None, None)
        .unwrap()
        .work_units_ordered()
        .expect("toposort");
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].id, "a");
    assert_eq!(units[1].id, "b");
}

#[test]
fn workflow_name_selector() {
    let src = r#"
        agent a { tier t description "a" }
        agent b { tier t description "b" }
        workflow just_b { steps [b] }
        workflow both   { steps [a b] }
    "#;
    let units = compile(src, "test.gaviero", Some("just_b"), None)
        .unwrap()
        .work_units_ordered()
        .expect("toposort");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "b");
}

#[test]
fn multiple_workflows_without_selector_is_an_error() {
    let src = r#"
        agent a { tier t description "a" }
        workflow first  { steps [a] }
        workflow second { steps [a] }
    "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("multiple workflows"), "report: {}", report);
}

#[test]
fn parse_error_missing_closing_brace() {
    let src = r#"agent x { tier t description "hello" "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(!report.is_empty());
}

#[test]
fn dependency_cycle_detected() {
    let src = r#"
        agent a { tier t depends_on [b] }
        agent b { tier t depends_on [a] }
    "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("cycle"), "expected cycle: {}", report);
}

// ── Iteration strategy tests ──────────────────────────────────────

#[test]
fn strategy_refine_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] strategy refine }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(
        matches!(compiled.iteration_config.strategy, Strategy::Refine),
        "expected Refine strategy"
    );
}

#[test]
fn strategy_single_pass_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] strategy single_pass }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(
        matches!(compiled.iteration_config.strategy, Strategy::SinglePass),
        "expected SinglePass strategy"
    );
}

#[test]
fn strategy_best_of_n_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] strategy best_of_3 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(
        matches!(
            compiled.iteration_config.strategy,
            Strategy::BestOfN { n: 3 }
        ),
        "expected BestOfN(3) strategy, got {:?}",
        compiled.iteration_config.strategy
    );
}

#[test]
fn test_first_true_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] test_first true }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(
        compiled.iteration_config.test_first,
        "expected test_first = true"
    );
}

#[test]
fn max_retries_workflow_level_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] max_retries 3 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert_eq!(compiled.iteration_config.max_retries, 3);
}

#[test]
fn verify_block_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w {
            steps [a]
            verify { compile true clippy true test false }
        }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(compiled.verification_config.compile);
    assert!(compiled.verification_config.clippy);
    assert!(!compiled.verification_config.test);
}

#[test]
fn verify_block_all_true() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w {
            steps [a]
            verify { compile true clippy true test true }
        }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert!(compiled.verification_config.compile);
    assert!(compiled.verification_config.clippy);
    assert!(compiled.verification_config.test);
}

#[test]
fn iteration_config_defaults_when_no_workflow() {
    let src = r#"agent a { tier t description "task" }"#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    // No workflow → defaults
    assert!(matches!(
        compiled.iteration_config.strategy,
        Strategy::Refine
    ));
    assert!(!compiled.iteration_config.test_first);
    assert_eq!(compiled.iteration_config.max_retries, 5);
    assert!(!compiled.verification_config.compile);
    assert!(!compiled.verification_config.clippy);
    assert!(!compiled.verification_config.test);
}

#[test]
fn escalate_after_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] escalate_after 5 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert_eq!(compiled.iteration_config.escalate_after, 5);
}

#[test]
fn attempts_propagated() {
    let src = r#"
        agent a { tier t description "task" }
        workflow w { steps [a] attempts 4 strategy best_of_4 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert_eq!(compiled.iteration_config.max_attempts, 4);
    assert!(matches!(
        compiled.iteration_config.strategy,
        Strategy::BestOfN { n: 4 }
    ));
}

// ── Example file compilation tests ──────────────────────────────

fn compile_example(filename: &str) -> Vec<gaviero_core::swarm::models::WorkUnit> {
    // Examples use `include "clients.gaviero"` so we must go through
    // compile_file (the inline compile() path rejects include).
    let path = std::path::PathBuf::from(format!(
        "{}/examples/{}",
        env!("CARGO_MANIFEST_DIR"),
        filename
    ));
    compile_file(&path, None, None, &[], &[], &[])
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
        .work_units_ordered()
        .expect("toposort")
}

#[test]
fn example_codebase_review() {
    let plan = compile_example_plan("codebase_review.gaviero");
    let units = plan.work_units_ordered().expect("toposort");

    // Review-only: inventory → loop { review_module } until halt_judge
    // → synthesize. Apply / verify_module / test_audit / stacked halt
    // probes are gone — findings are the deliverable.
    assert_eq!(units.len(), 3);
    let ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, vec!["inventory", "review_module", "synthesize"]);
    assert!(!ids.contains(&"execute_module"));
    assert!(!ids.contains(&"replan_module"));
    assert!(!ids.contains(&"verify_module"));
    assert!(!ids.contains(&"test_audit"));
    assert!(!ids.contains(&"final_verify"));

    assert_eq!(plan.loop_configs.len(), 1);
    let lc = &plan.loop_configs[0];
    assert_eq!(lc.agent_ids, vec!["review_module"]);
    assert_eq!(lc.max_iterations, 24);
    assert_eq!(lc.iter_start, 1);
    assert_eq!(lc.stability, 1);
    assert_eq!(lc.judge_timeout_secs, 60);
    assert!(lc.strict_judge);
    assert_eq!(
        lc.branch_chain,
        gaviero_core::swarm::plan::BranchChainMode::None
    );
    assert!(matches!(
        &lc.until,
        gaviero_core::swarm::plan::LoopUntilCondition::Agent(name) if name == "halt_judge"
    ));
    assert_eq!(plan.loop_judge_units.len(), 1);
    assert_eq!(plan.loop_judge_units[0].id, "halt_judge");
    assert_eq!(
        plan.loop_judge_units[0].model.as_deref(),
        Some("claude:sonnet")
    );
    assert!(plan.loop_judge_units[0].produces.is_empty());
    assert!(
        plan.loop_judge_units[0]
            .coordinator_instructions
            .contains("{{ITER_EVIDENCE}}"),
        "halt_judge should reference {{{{ITER_EVIDENCE}}}}"
    );

    // max_parallel 1 — shared checkout so later reviews can read earlier
    // review-N.md; stacked mode is not used (and would hard-fail here).
    assert_eq!(plan.max_parallel, Some(1));
    assert!(
        !plan.verification_config.compile
            && !plan.verification_config.clippy
            && !plan.verification_config.test,
        "review-only workflow must not run compile/clippy/test verify, got {:?}",
        plan.verification_config
    );

    let model_of = |id: &str| -> Option<String> {
        units
            .iter()
            .chain(plan.loop_judge_units.iter())
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("agent {id} not found"))
            .model
            .clone()
    };
    // Tier `medium` → sonnet, `high` → opus in the examples' default profile.
    assert_eq!(model_of("inventory").as_deref(), Some("claude:sonnet"));
    assert_eq!(model_of("review_module").as_deref(), Some("claude:opus"));
    assert_eq!(model_of("synthesize").as_deref(), Some("claude:sonnet"));

    let inventory = units.iter().find(|u| u.id == "inventory").unwrap();
    assert_eq!(
        inventory.produces,
        vec!["reviews/latest/inventory.md".to_string()]
    );
    assert_eq!(
        inventory.extra_allowed_tools,
        vec!["WebSearch".to_string(), "WebFetch".to_string()]
    );

    let review = units.iter().find(|u| u.id == "review_module").unwrap();
    assert_eq!(review.depends_on, vec!["inventory"]);
    assert_eq!(
        review.produces,
        vec!["reviews/latest/review-{{ITER}}.md".to_string()]
    );
    assert!(
        review.coordinator_instructions.contains("reviews/latest/"),
        "OUT_DIR should be compile-time substituted in prompts"
    );
    assert!(
        review.coordinator_instructions.contains("{{ITER}}"),
        "runtime ITER should survive compile"
    );
    assert!(
        !review.coordinator_instructions.contains("HALTED:"),
        "review body must not own loop halt via a HALTED token"
    );
    assert_eq!(
        review.extra_allowed_tools,
        vec!["WebSearch".to_string(), "WebFetch".to_string()]
    );
    assert!(
        review
            .scope
            .owned_paths
            .iter()
            .any(|p| p == "reviews/latest/review-*.md"),
        "OUT_DIR should be compile-time substituted in owned globs, got {:?}",
        review.scope.owned_paths
    );
    assert!(
        !review
            .scope
            .owned_paths
            .iter()
            .any(|p| p == "src/" || p == "crates/"),
        "review_module must not own source paths, got {:?}",
        review.scope.owned_paths
    );

    let synthesize = units.iter().find(|u| u.id == "synthesize").unwrap();
    assert!(
        synthesize.depends_on.iter().any(|d| d == "review_module"),
        "synthesize should depend on review_module, got {:?}",
        synthesize.depends_on
    );
    assert_eq!(
        synthesize.produces,
        vec!["reviews/latest/summary.md".to_string()]
    );
    assert_eq!(
        synthesize.extra_allowed_tools,
        vec!["WebSearch".to_string(), "WebFetch".to_string()]
    );
    assert_eq!(
        plan.loop_judge_units[0].extra_allowed_tools,
        vec!["WebSearch".to_string(), "WebFetch".to_string()]
    );
    assert_eq!(
        synthesize.write_namespace.as_deref(),
        Some("review-history")
    );
    assert_eq!(synthesize.memory_importance, Some(0.9));
    assert!(synthesize.memory_write_content.is_some());
}

#[test]
fn codebase_review_tier_overrides_reviewer() {
    let path = std::path::PathBuf::from(format!(
        "{}/examples/codebase_review.gaviero",
        env!("CARGO_MANIFEST_DIR")
    ));
    let plan = compile_file(
        &path,
        None,
        None,
        &[],
        &[("review_module".to_string(), "high-codex".to_string())],
        &[],
    )
    .expect("compile codebase_review with --tier review_module");
    let units = plan.work_units_ordered().expect("toposort");
    let model_of = |id: &str| -> Option<String> {
        units
            .iter()
            .chain(plan.loop_judge_units.iter())
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("agent {id} not found"))
            .model
            .clone()
    };
    assert_eq!(
        model_of("review_module").as_deref(),
        Some("codex:gpt-6.1-sol")
    );
    assert_eq!(model_of("inventory").as_deref(), Some("claude:sonnet"));
    assert_eq!(model_of("halt_judge").as_deref(), Some("claude:sonnet"));
    assert_eq!(model_of("synthesize").as_deref(), Some("claude:sonnet"));
}

#[test]
fn codebase_review_tier_overrides_several_agents() {
    let path = std::path::PathBuf::from(format!(
        "{}/examples/codebase_review.gaviero",
        env!("CARGO_MANIFEST_DIR")
    ));
    let plan = compile_file(
        &path,
        None,
        None,
        &[],
        &[
            ("review_module".to_string(), "high-codex".to_string()),
            ("inventory".to_string(), "low-codex".to_string()),
        ],
        &[],
    )
    .expect("compile codebase_review with two --tier overrides");
    let units = plan.work_units_ordered().expect("toposort");
    let review = units.iter().find(|u| u.id == "review_module").unwrap();
    let inventory = units.iter().find(|u| u.id == "inventory").unwrap();
    assert_eq!(review.model.as_deref(), Some("codex:gpt-6.1-sol"));
    assert_eq!(review.effort.as_deref(), Some("high"));
    assert_eq!(inventory.model.as_deref(), Some("codex:gpt-6-luna"));
    assert_eq!(inventory.effort.as_deref(), Some("low"));
}

#[test]
fn example_security_audit_memory() {
    let units = compile_example("security_audit_memory.gaviero");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "scan");

    // scan: read_ns merges workflow ["shared","security-policies"] + agent ["prior-audits"]
    let ns = units[0]
        .read_namespaces
        .as_ref()
        .expect("scan should have read_namespaces");
    assert!(
        ns.contains(&"shared".to_string()),
        "missing workflow ns 'shared': {:?}",
        ns
    );
    assert!(
        ns.contains(&"security-policies".to_string()),
        "missing workflow ns 'security-policies': {:?}",
        ns
    );
    assert!(
        ns.contains(&"prior-audits".to_string()),
        "missing agent ns 'prior-audits': {:?}",
        ns
    );
    // workflow ns must come before agent ns
    assert!(ns.iter().position(|s| s == "shared") < ns.iter().position(|s| s == "prior-audits"));

    // scan: agent write_ns overrides workflow write_ns
    assert_eq!(units[0].write_namespace.as_deref(), Some("scan-findings"));

    // scan: importance and staleness_sources
    assert!(matches!(units[0].memory_importance, Some(v) if (v - 0.9).abs() < 1e-4));
    assert_eq!(units[0].staleness_sources, vec!["src/"]);

    // verify: no agent-specific memory, but inherits workflow read_ns + write_ns
    assert_eq!(units[3].id, "verify");
    let verify_ns = units[3]
        .read_namespaces
        .as_ref()
        .expect("verify should inherit workflow ns");
    assert!(verify_ns.contains(&"shared".to_string()));
    assert!(verify_ns.contains(&"security-policies".to_string()));
    // verify has its own read_ns from memory block
    assert!(verify_ns.contains(&"scan-findings".to_string()));
    assert_eq!(
        units[3].write_namespace.as_deref(),
        Some("verification-results")
    );
}

fn compile_example_plan(filename: &str) -> gaviero_core::swarm::plan::CompiledPlan {
    let path = std::path::PathBuf::from(format!(
        "{}/examples/{}",
        env!("CARGO_MANIFEST_DIR"),
        filename
    ));
    compile_file(&path, None, None, &[], &[], &[])
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
}

#[test]
fn codebase_review_var_relocates_out_dir() {
    let path = std::path::PathBuf::from(format!(
        "{}/examples/codebase_review.gaviero",
        env!("CARGO_MANIFEST_DIR")
    ));
    let plan = compile_file(
        &path,
        None,
        None,
        &[("OUT_DIR".to_string(), "reviews/opus".to_string())],
        &[],
        &[],
    )
    .expect("compile codebase_review with --var OUT_DIR");
    let units = plan.work_units_ordered().expect("toposort");
    let inventory = units.iter().find(|u| u.id == "inventory").unwrap();
    assert_eq!(
        inventory.produces,
        vec!["reviews/opus/inventory.md".to_string()]
    );
    assert!(
        inventory
            .coordinator_instructions
            .contains("reviews/opus/inventory.md"),
        "prompt must use the overridden OUT_DIR, got {}",
        inventory.coordinator_instructions
    );
    assert!(
        !inventory
            .coordinator_instructions
            .contains("reviews/latest/inventory.md"),
        "default OUT_DIR must not remain in the inventory prompt"
    );
    let review = units.iter().find(|u| u.id == "review_module").unwrap();
    assert_eq!(
        review.produces,
        vec!["reviews/opus/review-{{ITER}}.md".to_string()]
    );
}

// ── Template compilation tests ─────────────────────────────────────

#[test]
fn template_update_docs() {
    let plan = compile_example_plan("update_docs.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 5);
    let inventory = units
        .iter()
        .find(|u| u.id == "inventory")
        .expect("inventory");
    // inventory runs on `top`, the README writer on `medium`.
    assert_eq!(inventory.model.as_deref(), Some("claude:fable"));
    let readme = units
        .iter()
        .find(|u| u.id == "write_readme_md")
        .expect("write_readme_md");
    assert_eq!(readme.model.as_deref(), Some("claude:sonnet"));
    assert_eq!(units[0].id, "inventory");
    // Three write agents depend on inventory
    assert!(units[1].depends_on.contains(&"inventory".to_string()));
    assert!(units[2].depends_on.contains(&"inventory".to_string()));
    assert!(units[3].depends_on.contains(&"inventory".to_string()));
    // record agent depends on all three writers
    assert_eq!(units[4].id, "record_docs_update");
    assert_eq!(units[4].depends_on.len(), 3);
    // No loops
    assert!(plan.loop_configs.is_empty());
    // Max parallel 1 (shared checkout; docs writers own disjoint paths)
    assert_eq!(plan.max_parallel, Some(1));
}

#[test]
fn example_pattern_map_reduce_emits_fanout() {
    let plan = compile_example_plan("pattern_map_reduce.gaviero");
    assert_eq!(plan.fanout_ops.len(), 1);
    assert_eq!(plan.fanout_ops[0].after_unit, "discover");
    assert_eq!(plan.fanout_ops[0].max_spawn, 8);
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].id, "discover");
    assert_eq!(units[1].id, "reduce");
    assert!(units[1].depends_on.contains(&"discover".to_string()));
}

/// Swapping every model at once is choosing another profile.
#[test]
fn update_docs_profile_swaps_every_model() {
    let examples = std::path::PathBuf::from(format!("{}/examples", env!("CARGO_MANIFEST_DIR")));
    let entry = examples.join("update_docs.gaviero");
    let codex = SelectedProfile {
        name: "codex".into(),
        path: examples.join("profiles/codex.gaviero"),
        precedence: ProfilePrecedence::OverridesScript,
    };
    let plan = gaviero_dsl::compile_file_with_profile(&entry, Some(&codex), None, None, &[], &[], &[])
        .expect("compile with the codex profile");
    let units = plan.work_units_ordered().expect("toposort");
    assert!(
        units.iter().all(|u| u.model.as_deref().is_some_and(|m| m.starts_with("codex:"))),
        "{:?}",
        units.iter().map(|u| u.model.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn template_plan_refinement() {
    let plan = compile_example_plan("plan_refinement.gaviero");
    let units = plan.work_units_ordered().expect("toposort");

    // Refine-only roster: Claude + Codex + Cursor (no separate init step)
    assert_eq!(units.len(), 3);

    let crefine = units
        .iter()
        .find(|u| u.id == "claude-refine")
        .expect("claude-refine");
    let xrefine = units
        .iter()
        .find(|u| u.id == "codex-refine")
        .expect("codex-refine");
    let urefine = units
        .iter()
        .find(|u| u.id == "cursor-refine")
        .expect("cursor-refine");

    // Each roster entry's `tier` sets its clone's model through the profile.
    assert_eq!(crefine.model.as_deref(), Some("claude:fable"));
    assert_eq!(xrefine.model.as_deref(), Some("codex:gpt-6-astra"));
    assert_eq!(urefine.model.as_deref(), Some("cursor:grok-4.7-high"));

    assert!(
        crefine
            .coordinator_instructions
            .contains("providers in this panel"),
        "claude-refine should mention peer panel"
    );
    assert!(
        crefine.coordinator_instructions.contains("codex"),
        "claude-refine should list codex as a peer"
    );
    assert!(
        crefine.coordinator_instructions.contains("cursor"),
        "claude-refine should list cursor as a peer"
    );
    assert!(
        crefine
            .coordinator_instructions
            .contains("PREV_ITER}}` = 0"),
        "claude-refine should document round-1 behaviour"
    );
    assert!(
        crefine.scope.read_only_paths.iter().any(|p| p == "."),
        "panel agents should have repo read-only scope"
    );

    // Refine agents: vars substituted; ITER/PREV_ITER survive for runtime
    assert!(
        crefine
            .coordinator_instructions
            .contains("claude-refine-plan-v{{ITER}}.md"),
        "claude-refine should reference claude-refine-plan-v{{ITER}}.md"
    );
    assert!(
        xrefine
            .coordinator_instructions
            .contains("codex-refine-plan-v{{ITER}}.md"),
        "codex-refine should reference codex-refine-plan-v{{ITER}}.md"
    );
    assert!(
        urefine
            .coordinator_instructions
            .contains("cursor-refine-plan-v{{ITER}}.md"),
        "cursor-refine should reference cursor-refine-plan-v{{ITER}}.md"
    );
    assert!(
        crefine
            .coordinator_instructions
            .contains("refine-plan-v{{PREV_ITER}}.md"),
        "claude-refine should reference refine-plan-v{{PREV_ITER}}.md"
    );

    // Summary file also uses ITER
    assert!(
        crefine
            .coordinator_instructions
            .contains("claude-refine-summary-v{{ITER}}.md"),
        "claude-refine should reference summary file"
    );

    // Refine agents write to memory
    assert_eq!(crefine.write_namespace.as_deref(), Some("plan-evolution"));
    assert_eq!(xrefine.write_namespace.as_deref(), Some("plan-evolution"));
    assert_eq!(urefine.write_namespace.as_deref(), Some("plan-evolution"));

    // Loop config: 3 refine agents, 5 iterations, iter_start=1 (round 1 PREV_ITER=0).
    assert_eq!(plan.loop_configs.len(), 1);
    let lc = &plan.loop_configs[0];
    assert_eq!(
        lc.agent_ids,
        vec!["claude-refine", "codex-refine", "cursor-refine"]
    );
    assert_eq!(lc.max_iterations, 5);
    assert_eq!(lc.iter_start, 1);
    assert_eq!(lc.stability, 2);
    assert_eq!(lc.judge_timeout_secs, 180);
    assert!(!lc.strict_judge);

    // Judge agent is compiled into the aux list, not the main DAG.
    assert_eq!(plan.loop_judge_units.len(), 1);
    assert_eq!(plan.loop_judge_units[0].id, "convergence-judge");
    // Judge prompt uses iteration evidence and panel-wide globs (N providers).
    assert!(
        plan.loop_judge_units[0]
            .coordinator_instructions
            .contains("{{ITER_EVIDENCE}}"),
        "judge prompt should reference {{{{ITER_EVIDENCE}}}}"
    );
    assert!(
        plan.loop_judge_units[0]
            .coordinator_instructions
            .contains("*-refine-summary-v{{ITER}}.md"),
        "judge should glob all provider summaries"
    );
    assert_eq!(
        plan.loop_judge_units[0].model.as_deref(),
        Some("claude:sonnet")
    );

    // max_parallel sized for 3+ provider panels
    assert_eq!(plan.max_parallel, Some(4));
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}

#[test]
fn plan_refinement_three_provider_roster() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plan_refinement.gaviero");
    let plan = compile_file(
        &path,
        Some("feature-plan-refinement"),
        Some("multi-provider plan test"),
        &[],
        &[],
        &[(
            "roster".into(),
            "claude=top-claude,codex=high-codex,cursor=medium-cursor"
                .into(),
        )],
    )
    .expect("three-provider roster should compile");
    let ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.clone())
        .collect();
    for prefix in &["claude", "codex", "cursor"] {
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-refine")),
            "missing {prefix}-refine in {ids:?}"
        );
    }
    assert_eq!(ids.len(), 3);
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(plan.loop_configs[0].agent_ids.len(), 3);
    assert_eq!(plan.loop_configs[0].iter_start, 1);

    let claude_refine = plan
        .work_units_unordered()
        .into_iter()
        .find(|u| u.id == "claude-refine")
        .expect("claude-refine");
    assert!(
        claude_refine.coordinator_instructions.contains("codex"),
        "claude-refine should list codex peer"
    );
    assert!(
        claude_refine.coordinator_instructions.contains("cursor"),
        "claude-refine should list cursor peer"
    );
}

#[test]
fn template_phased_plan() {
    let plan = compile_example_plan("phased_plan.gaviero");
    let units = plan.work_units_ordered().expect("toposort");

    // Main DAG: analyse_plan + phase_executor + phase_gate + final_audit.
    // The judge is compiled into loop_judge_units, not the main DAG.
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "analyse_plan");
    assert_eq!(units.last().unwrap().id, "final_audit");

    // Loop wraps phase_executor + phase_gate, with phase_judge as the
    // until-agent.
    assert_eq!(plan.loop_configs.len(), 1);
    let lc = &plan.loop_configs[0];
    assert_eq!(lc.agent_ids, vec!["phase_executor", "phase_gate"]);
    assert_eq!(lc.max_iterations, 12);
    assert_eq!(lc.stability, 1);
    assert_eq!(lc.judge_timeout_secs, 60);
    assert!(lc.strict_judge);

    assert_eq!(plan.loop_judge_units.len(), 1);
    assert_eq!(plan.loop_judge_units[0].id, "phase_judge");

    // Model routing: reasoning agents use opus, gate/judge use sonnet via
    // concrete `client` bindings in phased_plan.gaviero (included clients.gaviero
    // omits `tier` on client blocks). WorkUnit `tier` is the default; the
    // resolved `model` string is what backends use.
    let model_of = |id: &str| -> Option<String> {
        units
            .iter()
            .chain(plan.loop_judge_units.iter())
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("agent {} not found", id))
            .model
            .clone()
    };
    assert_eq!(model_of("analyse_plan").as_deref(), Some("claude:opus"));
    assert_eq!(model_of("phase_executor").as_deref(), Some("claude:opus"));
    assert_eq!(model_of("final_audit").as_deref(), Some("claude:opus"));
    assert_eq!(model_of("phase_gate").as_deref(), Some("claude:sonnet"));
    assert_eq!(model_of("phase_judge").as_deref(), Some("claude:sonnet"));

    // Dependency chain: phase_executor → analyse_plan; phase_gate →
    // phase_executor; final_audit → phase_gate (loop exit).
    let dep = |id: &str| -> Vec<String> {
        units
            .iter()
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("agent {} not found", id))
            .depends_on
            .clone()
    };
    assert_eq!(dep("phase_executor"), vec!["analyse_plan"]);
    assert_eq!(dep("phase_gate"), vec!["phase_executor", "analyse_plan"]);
    assert_eq!(dep("final_audit"), vec!["phase_gate"]);

    // OUT_DIR is compile-time substituted in prompts AND scope paths;
    // {{PROMPT}}, {{ITER}}, {{ITER_EVIDENCE}} survive to runtime.
    let executor = units.iter().find(|u| u.id == "phase_executor").unwrap();
    assert!(
        executor.coordinator_instructions.contains("plans/log/"),
        "OUT_DIR should be compile-time substituted in prompts"
    );
    assert!(
        executor.coordinator_instructions.contains("{{ITER}}"),
        "runtime ITER should survive compile"
    );
    assert!(
        executor
            .scope
            .owned_paths
            .iter()
            .any(|p| p == "plans/log/phase-*.md"),
        "OUT_DIR should be compile-time substituted in scope.owned, got {:?}",
        executor.scope.owned_paths
    );
    assert!(
        executor
            .scope
            .read_only_paths
            .iter()
            .any(|p| p == "plans/log/"),
        "OUT_DIR should be compile-time substituted in scope.read_only, got {:?}",
        executor.scope.read_only_paths
    );

    let analyse = &units[0];
    assert!(
        analyse.coordinator_instructions.contains("{{PROMPT}}"),
        "runtime PROMPT should survive compile"
    );

    let judge = &plan.loop_judge_units[0];
    assert!(
        judge.coordinator_instructions.contains("{{ITER_EVIDENCE}}"),
        "judge should reference {{{{ITER_EVIDENCE}}}}"
    );

    // Final audit writes to memory.
    let audit = units.iter().find(|u| u.id == "final_audit").unwrap();
    assert_eq!(audit.write_namespace.as_deref(), Some("phase-history"));
    assert_eq!(audit.memory_importance, Some(0.9));
    assert!(audit.memory_write_content.is_some());
}

// ── compile_file include integration tests ─────────────────────────────────

#[test]
fn compile_file_generic_consensus_with_reviewers() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/generic_consensus.gaviero");
    let plan = compile_file(
        &path,
        Some("generic-consensus"),
        Some("test topic"),
        &[],
        &[],
        &[],
    )
    .expect("generic_consensus should compile");
    let ids: Vec<_> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.as_str())
        .collect();
    assert!(ids.contains(&"claude-init"));
    assert!(ids.contains(&"codex-refine"));
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(
        plan.loop_configs[0].consensus_mode,
        gaviero_core::swarm::plan::ConsensusMode::PartialOk
    );
}

#[test]
fn generic_consensus_param_roster_override_swaps_to_three_reviewers() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/generic_consensus.gaviero");
    let overrides = vec![(
        "roster".to_string(),
        "claude=top-claude,codex=high-codex,cursor=medium-cursor".to_string(),
    )];
    let plan = compile_file(
        &path,
        Some("generic-consensus"),
        Some("test topic"),
        &[],
        &[],
        &overrides,
    )
    .expect("--param override should compile");
    let ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.clone())
        .collect();
    // All three reviewer ids expanded into init + refine.
    for prefix in &["claude", "codex", "cursor"] {
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-init")),
            "missing {prefix}-init in {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-refine")),
            "missing {prefix}-refine in {ids:?}"
        );
    }
    // Loop's refine agent list contains all three.
    assert_eq!(plan.loop_configs[0].agent_ids.len(), 3);
}

#[test]
fn compile_file_scientific_research_default_roster() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let plan = compile_file(
        &path,
        Some("scientific-research-consensus"),
        Some("test topic"),
        &[],
        &[],
        &[],
    )
    .expect("scientific_research default roster should compile");
    let ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.clone())
        .collect();
    for prefix in &["claude", "codex", "cursor"] {
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-init")),
            "missing {prefix}-init in {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-refine")),
            "missing {prefix}-refine in {ids:?}"
        );
    }
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(plan.loop_configs[0].agent_ids.len(), 3);
    assert_eq!(plan.loop_configs[0].max_iterations, 10);
    assert_eq!(plan.loop_configs[0].iter_start, 2);
    assert_eq!(plan.max_parallel, Some(5));
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}

/// Each reviewer clone must carry its own output contract — an un-prefixed
/// `produces` would have every clone claim the same artefacts, so a panel
/// where one member wrote nothing would still look complete.
#[test]
fn compile_file_scientific_research_produces_is_per_reviewer() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let plan = compile_file(
        &path,
        Some("scientific-research-consensus"),
        Some("test topic"),
        &[],
        &[],
        &[("roster".into(), "a=high-claude,b=high-codex".into())],
    )
    .expect("two-reviewer roster should compile");

    let units = plan.work_units_unordered();
    let by_id = |id: &str| {
        units
            .iter()
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    };

    // Init is versioned by INIT_VER, substituted at compile time.
    assert_eq!(
        by_id("a-init").produces,
        vec!["research/a-conclusion-v1.md", "research/a-summary-v1.md"]
    );
    // Refine keeps {{ITER}} for the runtime to substitute per pass.
    assert_eq!(
        by_id("b-refine").produces,
        vec![
            "research/b-conclusion-v{{ITER}}.md",
            "research/b-summary-v{{ITER}}.md"
        ]
    );
    // The judge declares no artefacts — it only reads.
    assert!(
        plan.loop_judge_units.iter().all(|u| u.produces.is_empty()),
        "judges must not declare produced artefacts"
    );
}

#[test]
fn compile_file_scientific_research_param_roster_override_to_two() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let overrides = vec![(
        "roster".to_string(),
        "claude=top-claude,codex=high-codex".to_string(),
    )];
    let plan = compile_file(
        &path,
        Some("scientific-research-consensus"),
        Some("test topic"),
        &[],
        &[],
        &overrides,
    )
    .expect("scientific_research roster override should compile");
    let ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.clone())
        .collect();
    for prefix in &["claude", "codex"] {
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-init")),
            "missing {prefix}-init in {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id == &format!("{prefix}-refine")),
            "missing {prefix}-refine in {ids:?}"
        );
    }
    assert!(
        !ids.iter().any(|id| id == "cursor-init"),
        "cursor should be absent after override, got {ids:?}"
    );
    assert_eq!(plan.loop_configs[0].agent_ids.len(), 2);
}

/// The roster's peer block carries `{{OUT_DIR}}` placeholders. They must be
/// substituted like any other var: a reviewer told to read a file under a
/// literal `{{OUT_DIR}}/` directory never finds its peers.
#[test]
fn scientific_research_peer_block_substitutes_out_dir() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let vars = vec![("OUT_DIR".to_string(), "research/run-a".to_string())];
    let overrides = vec![(
        "roster".to_string(),
        "claude=top-claude,codex=high-codex".to_string(),
    )];
    let plan = compile_file(
        &path,
        Some("scientific-research-consensus"),
        Some("test topic"),
        &vars,
        &[],
        &overrides,
    )
    .expect("scientific_research should compile");
    let units = plan.work_units_unordered();
    let refine = units
        .iter()
        .find(|u| u.id == "claude-refine")
        .expect("claude-refine unit");
    let prompt = &refine.coordinator_instructions;
    assert!(
        !prompt.contains("{{OUT_DIR}}"),
        "literal {{{{OUT_DIR}}}} reached the refine prompt"
    );
    assert!(
        prompt.contains("research/run-a/codex-conclusion-v"),
        "peer conclusion path missing from the refine prompt"
    );
    assert!(
        !prompt.contains("-init- on first refine pass"),
        "stale -init- note still in the peer block"
    );
}

#[test]
fn workflow_execution_document_parsed() {
    let src = r#"
agent a { tier t scope { owned ["out/"] } prompt "x" }
workflow w {
    execution_mode document
    steps [a]
}
"#;
    let plan = compile(src, "test.gaviero", Some("w"), None).expect("compile");
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}

#[test]
fn scientific_research_judge_runs_on_its_own_tier() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let plan = compile_file(
        &path,
        Some("scientific-research-consensus"),
        Some("topic"),
        &[],
        &[],
        &[],
    )
    .expect("scientific_research should compile");
    assert_eq!(plan.loop_judge_units.len(), 1);
    assert_eq!(
        plan.loop_judge_units[0].model.as_deref(),
        Some("claude:sonnet")
    );
    assert_eq!(plan.loop_judge_units[0].effort.as_deref(), Some("medium"));
}

#[test]
fn cli_tier_swaps_the_loop_judge_model() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/generic_consensus.gaviero");
    let tiers = vec![("consensus-judge".to_string(), "low".to_string())];
    let plan = compile_file(
        &path,
        Some("generic-consensus"),
        Some("topic"),
        &[],
        &tiers,
        &[],
    )
    .expect("--tier on the judge should compile");
    assert_eq!(
        plan.loop_judge_units[0].model.as_deref(),
        Some("claude:haiku")
    );
    assert_eq!(plan.loop_judge_units[0].effort.as_deref(), Some("low"));
}

/// A roster entry names a tier, never a model: `id=provider:model` is
/// rejected with a pointer to the profile.
#[test]
fn generic_consensus_roster_rejects_model_specs() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/generic_consensus.gaviero");
    let overrides = vec![(
        "roster".to_string(),
        "claude=claude:opus,codex=codex:gpt-5.5".to_string(),
    )];
    let err = compile_file(
        &path,
        Some("generic-consensus"),
        Some("t"),
        &[],
        &[],
        &overrides,
    )
    .expect_err("a model spec in a roster must fail");
    let msg = format!("{err:?}");
    assert!(msg.contains("id=<tier>"), "expected id=<tier> diagnostic, got: {msg}");

    // A tier the profile does not define names the agent and the tier.
    let overrides = vec![("roster".to_string(), "claude=nope".to_string())];
    let err = compile_file(&path, Some("generic-consensus"), Some("t"), &[], &[], &overrides)
        .expect_err("an unknown tier must fail");
    assert!(format!("{err:?}").contains("uses tier `nope`"), "{err:?}");
}

#[test]
fn param_without_default_or_cli_value_is_required_error() {
    let src = concat!(
        "prompt p #\"x\"#\n",
        "prompt r #\"y\"#\n",
        "agent tinit { tier t template true prompt p scope { owned [\"out/x-*\"] } }\n",
        "agent tref  { tier t template true prompt r scope { owned [\"out/y-*\"] } }\n",
        "agent judge { tier t prompt \"j\" }\n",
        "workflow w {\n",
        "  param roster\n",
        "  steps [ loop {\n",
        "    reviewers       roster\n",
        "    template_init   tinit\n",
        "    template_refine tref\n",
        "    until agent     judge\n",
        "    max_iterations  3\n",
        "  } ]\n",
        "}\n",
    );
    let err = compile(src, "inline.gaviero", None, None)
        .expect_err("required param without default must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("`roster` has no default") || msg.contains("not supplied"),
        "expected required-param diagnostic, got: {msg}"
    );
}

#[test]
fn compile_file_resolves_include_and_compiles_workflow() {
    use std::fs;
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Library: a shared agent and prompt
    let mut lib = fs::File::create(dir.join("lib.gaviero")).unwrap();
    lib.write_all(
        br##"
            prompt work #"do work"#
            agent worker {
                description "test"
                tier sonnet
                prompt work
            }
        "##,
    )
    .unwrap();

    // Entry: a workflow that runs the included agent
    let entry = dir.join("main.gaviero");
    let mut f = fs::File::create(&entry).unwrap();
    f.write_all(
        br#"
            include "lib.gaviero"
            workflow w { steps [worker] }
        "#,
    )
    .unwrap();

    let plan = compile_file(&entry, None, None, &[], &[], &[])
        .expect("compile_file should succeed across include boundary");
    let units = plan.work_units_ordered().unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "worker");
    assert_eq!(units[0].model, Some("claude-sonnet-4-6".to_string()));
    assert_eq!(units[0].tier, ModelTier::Cheap);
}

/// An included file is part of the script: it may not declare clients
/// either, and the diagnostic points at the included file.
#[test]
fn compile_file_rejects_a_client_in_an_included_file() {
    use std::fs;
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::File::create(dir.join("clients.gaviero"))
        .unwrap()
        .write_all(br#"client base { model "claude:sonnet" }"#)
        .unwrap();
    let entry = dir.join("main.gaviero");
    fs::File::create(&entry)
        .unwrap()
        .write_all(
            br#"
                include "clients.gaviero"
                agent w { tier t prompt "x" }
                workflow wf { steps [w] }
            "#,
        )
        .unwrap();

    let err = compile_file(&entry, None, None, &[], &[], &[]).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(msg.contains("cannot declare `client base`"), "got: {msg}");
    assert!(msg.contains("clients.gaviero"), "got: {msg}");
}

#[test]
fn compile_file_reports_duplicate_decl_across_files() {
    use std::fs;
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Both files declare `prompt base` — duplicate should be flagged.
    fs::File::create(dir.join("a.gaviero"))
        .unwrap()
        .write_all(br#"prompt base "one""#)
        .unwrap();
    fs::File::create(dir.join("b.gaviero"))
        .unwrap()
        .write_all(br#"prompt base "two""#)
        .unwrap();
    let entry = dir.join("main.gaviero");
    fs::File::create(&entry)
        .unwrap()
        .write_all(
            br#"
                include "a.gaviero"
                include "b.gaviero"
                agent w { tier t prompt base }
                workflow wf { steps [w] }
            "#,
        )
        .unwrap();

    let err = compile_file(&entry, None, None, &[], &[], &[]).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("duplicate prompt name `base`"),
        "expected duplicate-prompt diagnostic, got: {}",
        msg
    );
}

#[test]
fn compile_rejects_inline_include_with_helpful_diagnostic() {
    let src = r#"
        include "lib.gaviero"
        agent a { tier t }
    "#;
    let err = compile(src, "inline.gaviero", None, None).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("compile_file") || msg.contains("--script"),
        "expected diagnostic to point at compile_file/--script, got: {}",
        msg
    );
}

// ── `for` loops (examples/for_loops.gaviero) ─────────────────────────────

fn for_loops_plan(params: &[(String, String)]) -> gaviero_core::swarm::plan::CompiledPlan {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/for_loops.gaviero");
    compile_file(
        &path,
        Some("for-loops"),
        Some("the release"),
        &[("OUT_DIR".to_string(), "out/x".to_string())],
        &[],
        params,
    )
    .expect("for_loops.gaviero should compile")
}

fn unit<'a>(
    plan: &'a gaviero_core::swarm::plan::CompiledPlan,
    id: &str,
) -> &'a gaviero_core::swarm::models::WorkUnit {
    plan.work_units_unordered()
        .into_iter()
        .find(|u| u.id == id)
        .unwrap_or_else(|| panic!("missing unit {id}"))
}

#[test]
fn for_loops_clones_are_named_item_dash_agent() {
    let plan = for_loops_plan(&[]);
    let ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .map(|u| u.id.clone())
        .collect();
    for item in ["a", "b", "c"] {
        for agent in ["analyst", "editor", "reviewer"] {
            let id = format!("{item}-{agent}");
            assert!(ids.contains(&id), "missing {id} in {ids:?}");
        }
    }
    for body in ["analyst", "editor", "reviewer"] {
        assert!(!ids.iter().any(|i| i == body), "body agent {body} must not run itself");
    }
}

#[test]
fn for_loops_item_vars_reach_prompt_and_scope() {
    let plan = for_loops_plan(&[]);
    let b = unit(&plan, "b-analyst");
    let prompt = &b.coordinator_instructions;
    assert!(prompt.contains("network"), "focus field missing: {prompt}");
    assert!(prompt.contains("a,c"), "peers missing: {prompt}");
    assert!(prompt.contains("analyst 2 of 3"), "index/count missing: {prompt}");
    assert!(prompt.contains("out/x/a-notes.md"), "peer_block not rendered: {prompt}");
    assert!(prompt.contains("out/x/c-notes.md"), "peer_block not rendered: {prompt}");
    assert!(!prompt.contains("- out/x/b-notes.md"), "self listed as a peer: {prompt}");
    assert!(!prompt.contains("{{t."), "unsubstituted item var: {prompt}");
    assert!(!prompt.contains("{{OUT_DIR}}"), "unsubstituted OUT_DIR: {prompt}");
    assert_eq!(b.scope.owned_paths, vec!["out/x/b-notes.md".to_string()]);
    assert_eq!(b.produces, vec!["out/x/b-notes.md".to_string()]);
}

#[test]
fn for_loops_sequential_chain_and_empty_prev() {
    let plan = for_loops_plan(&[]);
    let a = unit(&plan, "a-editor");
    let b = unit(&plan, "b-editor");
    let c = unit(&plan, "c-editor");
    assert!(b.depends_on.contains(&"a-editor".to_string()), "{:?}", b.depends_on);
    assert!(c.depends_on.contains(&"b-editor".to_string()), "{:?}", c.depends_on);
    assert!(
        !a.depends_on.iter().any(|d| d.ends_with("-editor")),
        "first item must not depend on another editor: {:?}",
        a.depends_on
    );
    assert!(a.coordinator_instructions.contains("out/x/-draft.md"));
    assert!(b.coordinator_instructions.contains("out/x/a-draft.md"));
}

#[test]
fn for_loops_fan_in_depends_on_every_clone() {
    let plan = for_loops_plan(&[]);
    let s = unit(&plan, "summarizer");
    for id in ["a-analyst", "b-analyst", "c-analyst"] {
        assert!(s.depends_on.contains(&id.to_string()), "{:?}", s.depends_on);
    }
    assert!(!s.depends_on.contains(&"analyst".to_string()));
}

#[test]
fn for_loops_parallel_inside_loop_becomes_loop_agents() {
    let plan = for_loops_plan(&[]);
    let lc = plan.loop_configs.first().expect("loop config");
    assert_eq!(lc.agent_ids, vec!["a-reviewer", "b-reviewer", "c-reviewer"]);
    assert!(lc.baseline_units.is_empty(), "{:?}", lc.baseline_units);
}

const FOR_BASE_SRC: &str = r#"
agent base { tier t
    scope { owned ["out/{{t.id}}-v*.md"] }
    prompt "baseline {{t.id}}"
}
agent rev { tier t
    scope { owned ["out/{{t.id}}-v*.md"] }
    prompt "review {{t.id}} v{{ITER}}"
}
agent judge { tier t prompt "VERDICT" }
workflow w {
    execution_mode document
    param topics [ { id "a" } { id "b" } { id "c" } ]
    steps [
        for t in topics parallel { base }
        loop {
            for t in topics parallel { rev }
            max_iterations 2
            until agent judge
        }
    ]
}
"#;

#[test]
fn for_loops_baseline_units_pair_a_for_step_with_its_loop() {
    let plan =
        compile_with_vars(FOR_BASE_SRC, "t.gaviero", Some("w"), None, &[], &[], &[])
            .expect("compiles");
    let lc = plan.loop_configs.first().expect("loop config");
    assert_eq!(lc.agent_ids, vec!["a-rev", "b-rev", "c-rev"]);
    assert_eq!(lc.baseline_units, vec!["a-base", "b-base", "c-base"]);
}

#[test]
fn for_loops_param_list_literal_override() {
    let plan = for_loops_plan(&[(
        "topics".to_string(),
        r#"[{id "x" focus "one"} {id "y" focus "two"}]"#.to_string(),
    )]);
    let mut ids: Vec<String> = plan
        .work_units_unordered()
        .into_iter()
        .filter(|u| u.id.ends_with("-analyst"))
        .map(|u| u.id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["x-analyst", "y-analyst"]);
}

/// A minimal script around one `for` block, for the error cases.
fn for_error(steps: &str, agents: &str, params: &str, document: bool) -> String {
    let src = format!(
        r#"
agent judge {{ tier t prompt "VERDICT" }}
{agents}
workflow w {{
    {mode}
    param topics [ {{ id "a" }} {{ id "b" }} ]
    {params}
    steps [ {steps} ]
}}
"#,
        mode = if document { "execution_mode document" } else { "" },
    );
    match compile_with_vars(&src, "t.gaviero", Some("w"), None, &[], &[], &[]) {
        Ok(_) => panic!("expected a compile error for:\n{src}"),
        Err(e) => format!("{e:?}"),
    }
}

const OK_AGENT: &str = r#"agent a { tier t scope { owned ["out/{{t.id}}.md"] } prompt "x {{t.id}}" }
agent uses_judgeparam { tier t prompt "j" }"#;

#[test]
fn for_error_parallel_body_must_write_per_item_files() {
    let msg = for_error(
        "for t in topics parallel { a } uses_judgeparam",
        r#"agent a { tier t scope { owned ["out/shared.md"] } prompt "x" }
agent uses_judgeparam { tier t prompt "j" }"#,
        "",
        true,
    );
    assert!(msg.contains("{{t.id}}"), "{msg}");
}

#[test]
fn for_error_sequential_needs_document_mode() {
    let msg = for_error("for t in topics sequential { a } uses_judgeparam", OK_AGENT, "", false);
    // miette wraps the message, so match the words separately.
    assert!(msg.contains("execution_mode"), "{msg}");
    assert!(msg.contains("document"), "{msg}");
}

#[test]
fn for_error_unknown_item_field() {
    let msg = for_error(
        "for t in topics parallel { a } uses_judgeparam",
        r#"agent a { tier t scope { owned ["out/{{t.id}}.md"] } prompt "x {{t.nope}}" }
agent uses_judgeparam { tier t prompt "j" }"#,
        "",
        true,
    );
    assert!(msg.contains("t.nope"), "{msg}");
}

#[test]
fn for_error_empty_list_from_cli() {
    let src = r#"
agent a { tier t scope { owned ["out/{{t.id}}.md"] } prompt "x" }
workflow w { param topics [ { id "a" } ] steps [ for t in topics parallel { a } ] }
"#;
    let err = compile_with_vars(
        src,
        "t.gaviero",
        Some("w"),
        None,
        &[],
        &[],
        &[("topics".to_string(), " , ".to_string())],
    )
    .unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("empty"), "{msg}");
}

#[test]
fn for_error_duplicate_ids() {
    let msg = for_error(
        "for t in topics2 parallel { a } uses_judgeparam",
        OK_AGENT,
        r#"param topics2 [ { id "a" } { id "a" } ]"#,
        true,
    );
    assert!(msg.contains("duplicate"), "{msg}");
}

#[test]
fn for_error_invalid_id() {
    let msg = for_error(
        "for t in topics2 parallel { a } uses_judgeparam",
        OK_AGENT,
        r#"param topics2 [ { id "a b" } ]"#,
        true,
    );
    assert!(msg.contains("id"), "{msg}");
    assert!(msg.contains("invalid"), "{msg}");
}

#[test]
fn for_error_reserved_field() {
    let msg = for_error(
        "for t in topics2 parallel { a } uses_judgeparam",
        OK_AGENT,
        r#"param topics2 [ { id "a" prev "x" } ]"#,
        true,
    );
    assert!(msg.contains("reserved"), "{msg}");
}

#[test]
fn for_error_body_agent_listed_as_plain_step() {
    let msg = for_error("for t in topics parallel { a } a uses_judgeparam", OK_AGENT, "", true);
    assert!(msg.contains("plain step"), "{msg}");
}

#[test]
fn for_error_sequential_inside_loop() {
    let msg = for_error(
        "loop { for t in topics sequential { a } until agent judge } uses_judgeparam",
        OK_AGENT,
        "",
        true,
    );
    assert!(msg.contains("sequential"), "{msg}");
    assert!(msg.contains("loop"), "{msg}");
}

// ── loop ordering and the one-loop rule ──────────────────────────────────

#[test]
fn loop_agent_ids_follow_intra_loop_depends_on() {
    let src = r#"
agent inv { tier t scope { owned ["out/inv-v*.md"] } prompt "investigate v{{ITER}}" }
agent cur { tier t scope { owned ["out/cur-v*.md"] } depends_on [inv] prompt "curate v{{ITER}}" }
agent judge { tier t prompt "VERDICT" }
workflow w {
    execution_mode document
    steps [
        loop {
            agents [cur inv]
            max_iterations 2
            until agent judge
        }
    ]
}
"#;
    let plan = compile_with_vars(src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .expect("compiles");
    let lc = plan.loop_configs.first().expect("loop config");
    assert_eq!(lc.agent_ids, vec!["inv", "cur"]);
}

#[test]
fn error_two_loops_in_one_workflow() {
    let src = r#"
agent a { tier t scope { owned ["out/a-v*.md"] } prompt "a v{{ITER}}" }
agent b { tier t scope { owned ["out/b-v*.md"] } prompt "b v{{ITER}}" }
agent judge { tier t prompt "VERDICT" }
workflow w {
    execution_mode document
    steps [
        loop { agents [a] max_iterations 2 until agent judge }
        loop { agents [b] max_iterations 2 until agent judge }
    ]
}
"#;
    let err = compile_with_vars(src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("a workflow may contain at most one `loop` block"),
        "{msg}"
    );
}

// ── `for … in json` (runtime lists) ──────────────────────────────────────

const RUNTIME_FOR_AGENTS: &str = r#"
agent worker { tier t
    scope { owned ["out/{{q.id}}.md"] }
    produces ["out/{{q.id}}.md"]
    prompt "MARK-{{q.id}} focus={{q.focus}}"
}
agent summary { tier t depends_on [worker] prompt "summarise" }
agent judge { tier t prompt "VERDICT" }
"#;

fn runtime_for_compile(
    workflow_body: &str,
) -> Result<gaviero_core::swarm::plan::CompiledPlan, String> {
    let src = format!("{RUNTIME_FOR_AGENTS}\nworkflow w {{ {workflow_body} }}\n");
    compile_with_vars(&src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .map_err(|e| format!("{e:?}"))
}

fn runtime_for_error(workflow_body: &str) -> String {
    runtime_for_compile(workflow_body).expect_err("expected a compile error")
}

#[test]
fn runtime_for_top_level_emits_template_and_op() {
    let plan = runtime_for_compile(
        r#"execution_mode document
           steps [ for q in json "out/items.json" parallel { worker } summary ]"#,
    )
    .expect("compiles");
    let worker = unit(&plan, "worker");
    assert_eq!(worker.produces, vec!["out/{{q.id}}.md"]);
    assert!(worker.coordinator_instructions.contains("MARK-{{q.id}}"));
    assert_eq!(plan.runtime_fors.len(), 1);
    let op = &plan.runtime_fors[0];
    assert_eq!(op.unit_id, "worker");
    assert_eq!(op.var, "q");
    assert_eq!(op.source, "out/items.json");
    assert_eq!(op.max_items, gaviero_core::swarm::plan::DEFAULT_MAX_SPAWN);
    assert!(unit(&plan, "summary").depends_on.contains(&"worker".to_string()));
}

#[test]
fn runtime_for_inside_loop_joins_loop_agents() {
    let plan = runtime_for_compile(
        r#"execution_mode document
           steps [ loop {
               for q in json "out/items-v{{PREV_ITER}}.json" parallel max 6 { worker }
               max_iterations 3
               until agent judge
           } ]"#,
    )
    .expect("compiles");
    let lc = plan.loop_configs.first().expect("loop config");
    assert_eq!(lc.agent_ids, vec!["worker"]);
    assert_eq!(plan.runtime_fors[0].source, "out/items-v{{PREV_ITER}}.json");
    assert_eq!(plan.runtime_fors[0].max_items, 6);
}

#[test]
fn runtime_for_source_gets_compile_time_vars() {
    let src = format!(
        "{RUNTIME_FOR_AGENTS}\nvars {{ OUT_DIR \"research/x\" }}\nworkflow w {{ execution_mode document steps [ for q in json \"{{{{OUT_DIR}}}}/items.json\" parallel {{ worker }} ] }}\n"
    );
    let plan = compile_with_vars(&src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .expect("compiles");
    assert_eq!(plan.runtime_fors[0].source, "research/x/items.json");
}

#[test]
fn tier_bindings_are_emitted() {
    let plan = runtime_for_compile(
        r#"execution_mode document
           steps [ for q in json "out/items.json" parallel { worker } ]"#,
    )
    .expect("compiles");
    // Every tier of the profile is emitted, so runtime items can name one.
    let b = plan.tier_bindings.get("low").expect("tier low bound");
    assert_eq!(b.model.as_deref(), Some("claude:haiku"));
    assert_eq!(b.effort.as_deref(), Some("low"));
}

#[test]
fn runtime_for_error_sequential() {
    let msg = runtime_for_error(
        r#"execution_mode document steps [ for q in json "out/i.json" sequential { worker } ]"#,
    );
    assert!(msg.contains("`for … in json` supports only `parallel`"), "{msg}");
}

#[test]
fn runtime_for_error_max_without_json() {
    let msg = runtime_for_error(
        r#"execution_mode document param topics [ { id "a" } ]
           steps [ for q in topics parallel max 2 { worker } ]"#,
    );
    assert!(msg.contains("`max` applies only to `for … in json`"), "{msg}");
}

#[test]
fn runtime_for_error_max_zero() {
    let msg = runtime_for_error(
        r#"execution_mode document steps [ for q in json "out/i.json" parallel max 0 { worker } ]"#,
    );
    assert!(msg.contains("`max` must be at least 1"), "{msg}");
}

#[test]
fn runtime_for_error_peer_template() {
    let msg = runtime_for_error(
        r#"execution_mode document steps [ for q in json "out/i.json" parallel { worker peer_template "x" } ]"#,
    );
    assert!(
        msg.contains("`peer_template` is not available with `for … in json`"),
        "{msg}"
    );
}

#[test]
fn runtime_for_error_two_body_agents() {
    let msg = runtime_for_error(
        r#"execution_mode document steps [ for q in json "out/i.json" parallel { worker summary } ]"#,
    );
    assert!(msg.contains("`for … in json` takes exactly one body agent"), "{msg}");
}

#[test]
fn runtime_for_error_needs_document_mode() {
    let msg = runtime_for_error(r#"steps [ for q in json "out/i.json" parallel { worker } ]"#);
    assert!(
        msg.contains("`for … in json` requires `execution_mode document`"),
        "{msg}"
    );
}

#[test]
fn runtime_for_error_body_without_produces() {
    let src = r#"
agent bare { tier t scope { owned ["out/{{q.id}}.md"] } prompt "x" }
workflow w { execution_mode document steps [ for q in json "out/i.json" parallel { bare } ] }
"#;
    let msg = format!(
        "{:?}",
        compile_with_vars(src, "t.gaviero", Some("w"), None, &[], &[], &[])
            .unwrap_err()
    );
    assert!(
        msg.contains("the body of `for … in json` must declare `produces`"),
        "{msg}"
    );
}

#[test]
fn runtime_for_error_inside_stacked_loop() {
    let msg = runtime_for_error(
        r#"execution_mode document steps [ loop {
               for q in json "out/i.json" parallel { worker }
               branch_chain stacked
               until agent judge
           } ]"#,
    );
    assert!(
        msg.contains("`for … in json` cannot run inside a `branch_chain stacked` loop"),
        "{msg}"
    );
}

#[test]
fn map_reduce_requires_document_mode() {
    let src = r#"
agent d { tier t prompt "discover" }
agent r { tier t depends_on [d] prompt "reduce" }
workflow w {
    steps [d r]
    pattern map_reduce { discover d reduce r max_spawn 4 }
}
"#;
    let msg = format!(
        "{:?}",
        compile_with_vars(src, "t.gaviero", Some("w"), None, &[], &[], &[])
            .unwrap_err()
    );
    assert!(
        msg.contains("`pattern map_reduce` requires `execution_mode document`"),
        "{msg}"
    );
}

// ── budget { wall usd } ──────────────────────────────────────────────────

fn budget_compile(block: &str) -> Result<gaviero_core::swarm::plan::CompiledPlan, String> {
    let src = format!(
        "agent a {{ tier t prompt \"x\" }}\nworkflow w {{ steps [a] {block} }}\n"
    );
    compile_with_vars(&src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .map_err(|e| format!("{e:?}"))
}

#[test]
fn budget_block_compiles() {
    let plan = budget_compile(r#"budget { wall "8h" usd 40 }"#).expect("compiles");
    let b = plan.budget.expect("budget");
    assert_eq!(b.wall_secs, Some(28800));
    assert_eq!(b.usd, Some(40.0));
}

#[test]
fn budget_usd_float() {
    let plan = budget_compile(r#"budget { usd 12.5 }"#).expect("compiles");
    let b = plan.budget.expect("budget");
    assert_eq!(b.usd, Some(12.5));
    assert_eq!(b.wall_secs, None);
}

#[test]
fn budget_wall_units() {
    let secs = |w: &str| {
        budget_compile(&format!("budget {{ wall \"{w}\" }}"))
            .expect("compiles")
            .budget
            .unwrap()
            .wall_secs
    };
    assert_eq!(secs("90s"), Some(90));
    assert_eq!(secs("30m"), Some(1800));
    assert_eq!(secs("45"), Some(45));
    assert!(budget_compile("").unwrap().budget.is_none());
}

#[test]
fn budget_error_bad_duration() {
    let msg = budget_compile(r#"budget { wall "eight hours" }"#).unwrap_err();
    assert!(
        msg.contains(r#"`wall` takes a duration like \"90s\", \"30m\" or \"8h\""#)
            || msg.contains("`wall` takes a duration like"),
        "{msg}"
    );
}

#[test]
fn budget_error_usd_not_positive() {
    let msg = budget_compile(r#"budget { usd 0 }"#).unwrap_err();
    assert!(msg.contains("`usd` must be positive"), "{msg}");
}

#[test]
fn budget_error_empty() {
    let msg = budget_compile(r#"budget { }"#).unwrap_err();
    assert!(msg.contains("`budget` needs `wall` or `usd`"), "{msg}");
}

// ── produces json ────────────────────────────────────────────────────────

#[test]
fn produces_json_compiles_into_both_lists() {
    let src = r#"
vars { OUT_DIR "research/x" }
agent a { tier t
    scope { owned ["{{OUT_DIR}}/*"] }
    produces ["{{OUT_DIR}}/report.md"]
    produces json ["{{OUT_DIR}}/frontier-v{{ITER}}.json"]
    prompt "x"
}
workflow w { steps [a] }
"#;
    let plan = compile_with_vars(src, "t.gaviero", Some("w"), None, &[], &[], &[])
        .expect("compiles");
    let a = unit(&plan, "a");
    assert_eq!(
        a.produces,
        vec!["research/x/report.md", "research/x/frontier-v{{ITER}}.json"]
    );
    assert_eq!(a.produces_json, vec!["research/x/frontier-v{{ITER}}.json"]);
}

// ── examples/deep-explore.gaviero ────────────────────────────────────────

/// The `{{name}}` placeholders of a text, without the braces.
fn regex_lite_placeholders() -> impl Fn(&str) -> Vec<String> {
    |text: &str| {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    out.push(after[..end].to_string());
                    rest = &after[end + 2..];
                }
                None => break,
            }
        }
        out
    }
}

#[test]
fn example_deep_explore_compiles() {
    // The script names no provider: models come from the profile, here the
    // shipped `mixed` one (high = claude:opus, top = claude:fable, medium =
    // codex terra), the way a run's implicit `default` profile fills its tiers.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let script = dir.join("deep-explore.gaviero");
    let profile = gaviero_core::script_profiles::SelectedProfile {
        name: "default".into(),
        path: dir.join("profiles/mixed.gaviero"),
        precedence: gaviero_core::script_profiles::ProfilePrecedence::FillsGaps,
    };
    let compile = |workflow: &str, prompt: Option<&str>, tiers: &[(String, String)]| {
        gaviero_dsl::compile_file_with_profile(&script, Some(&profile), Some(workflow), prompt, &[], tiers, &[])
            .unwrap_or_else(|e| panic!("compiling deep-explore.gaviero --workflow {workflow}:\n{e:?}"))
    };
    let plan = compile("deep-explore", None, &[]);
    // Without a profile no tier is bound: the script declares none of its own.
    assert!(gaviero_dsl::compile_file(&script, Some("deep-explore"), None, &[], &[], &[]).is_err());
    // Two workflows: a run must name one.
    assert!(
        gaviero_dsl::compile_file_with_profile(&script, Some(&profile), None, None, &[], &[], &[]).is_err()
    );

    // The only input is the run prompt (SWARM form Prompt box / --prompt):
    // no file var for the form to ask for.
    let inspection = gaviero_dsl::inspect_file(&script).expect("inspect");
    assert!(inspection.requires_prompt);
    let vars: Vec<&str> = inspection.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        vars,
        ["OUT_DIR", "COLLECTION", "ACADEMIC_SERVER", "FALSIFY_TIER", "TOOLS_DIR", "PYTHON"]
    );
    // Every agent declares its tier; the form learns which ones the profile
    // must define.
    assert_eq!(inspection.tiers, ["high", "top", "medium"]);
    let workflows: Vec<&str> = inspection.workflows.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(workflows, ["deep-explore", "deep-explore-report"]);

    let with_prompt = compile("deep-explore", Some("Map the failure modes of X."), &[]);
    let curator_prompt = &unit(&with_prompt, "curator").coordinator_instructions;
    assert!(curator_prompt.contains("Map the failure modes of X."), "{curator_prompt}");
    assert!(curator_prompt.contains("`academic-mcp` MCP server"), "{curator_prompt}");
    // Every placeholder left after compiling is one the runtime fills: the
    // loop pass, the gate's feedback or a runtime `for` item. A typo would
    // reach the agent.
    let runtime_placeholder = regex_lite_placeholders();
    let runtime_filled = |found: &str| {
        found == "ITER" || found == "PREV_ITER" || found == "GATE_FEEDBACK" || found.starts_with("q.")
    };
    for u in with_prompt
        .work_units_unordered()
        .into_iter()
        .chain(with_prompt.loop_judge_units.iter())
    {
        for found in runtime_placeholder(&u.coordinator_instructions) {
            assert!(runtime_filled(&found), "unresolved {{{{{found}}}}} in {}", u.id);
        }
    }

    // No model judges the loop: the pass gate script does.
    assert!(plan.loop_judge_units.is_empty());
    let ids: Vec<String> = plan.work_units_unordered().into_iter().map(|u| u.id.clone()).collect();
    for gone in ["stop_judge", "citation_check", "synthesize"] {
        assert!(!ids.iter().any(|id| id == gone), "{gone} in {ids:?}");
    }
    let model = |id: &str| -> String {
        unit(&plan, id)
            .model
            .clone()
            .unwrap_or_else(|| panic!("no model for {id}"))
    };
    assert_eq!(model("scout"), "claude:opus", "tier high");
    assert_eq!(model("curator"), "claude:fable", "tier top");
    assert_eq!(model("investigator"), "claude:opus", "tier high");
    // The auditor declares `tier medium`: another family than the
    // investigators under this profile.
    assert_eq!(model("verifier"), "codex:gpt-6.1-terra", "tier medium");
    assert!(plan.tier_bindings.contains_key("high"), "runtime items can name tier high");

    // `--tier` moves the auditor for a run.
    let moved = compile("deep-explore", None, &[("verifier".to_string(), "top".to_string())]);
    assert_eq!(unit(&moved, "verifier").model.as_deref(), Some("claude:fable"));

    assert_eq!(plan.runtime_fors.len(), 1);
    let op = &plan.runtime_fors[0];
    assert_eq!(op.unit_id, "investigator");
    assert_eq!(op.source, "research/explore/assignments-v{{ITER}}.json");
    assert_eq!(op.max_items, 6);

    let lc = plan.loop_configs.first().expect("loop config");
    // Dependency order inside every pass: curate → investigate → audit.
    assert_eq!(lc.agent_ids, vec!["curator", "investigator", "verifier"]);
    assert_eq!(lc.max_iterations, 6);
    assert!(
        matches!(
            &lc.until,
            gaviero_core::swarm::plan::LoopUntilCondition::Command(cmd)
                if cmd == "python -B crates/gaviero-dsl/examples/deep-explore/pass_gate.py research/explore {{ITER}}"
        ),
        "{:?}",
        lc.until
    );

    let budget = plan.budget.as_ref().expect("budget");
    assert_eq!(budget.usd, Some(60.0));
    assert_eq!(budget.wall_secs, Some(10 * 3600));
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );

    let curator = unit(&plan, "curator");
    assert_eq!(curator.produces_json.len(), 2);
    assert_eq!(
        unit(&plan, "verifier").produces_json,
        vec!["research/explore/audit-v{{ITER}}.json"]
    );
    assert_eq!(
        unit(&plan, "scout").produces_json,
        vec!["research/explore/landscape.json"]
    );

    // The shape the runtime launches: scope validation as `execute` runs it.
    let units = plan.work_units_ordered().expect("toposort");
    let groups = gaviero_core::swarm::validation::add_baseline_units_to_loop_groups(
        gaviero_core::swarm::validation::expand_loop_groups_with_roster_init(
            plan.loop_configs.iter().map(|lc| lc.agent_ids.clone()).collect(),
            &units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
        ),
        &plan.loop_configs,
    );
    let errors = gaviero_core::swarm::validation::validate_scopes(&units, &groups);
    assert!(errors.is_empty(), "{errors:?}");

    // The helper scripts TOOLS_DIR names (relative to the repository root).
    let tools = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/deep-explore");
    for file in ["gate_common.py", "pass_gate.py", "report_gate.py"] {
        assert!(tools.join(file).is_file(), "missing {file} in {}", tools.display());
    }

    // `deep-explore-report`: one loop, synthesize until the report gate passes.
    let report = compile("deep-explore-report", Some("Map the failure modes of X."), &[]);
    let report_ids: Vec<String> =
        report.work_units_unordered().into_iter().map(|u| u.id.clone()).collect();
    assert_eq!(report_ids, vec!["synthesize".to_string()]);
    let synth = unit(&report, "synthesize");
    assert!(synth.depends_on.is_empty(), "{:?}", synth.depends_on);
    assert_eq!(synth.model.as_deref(), Some("claude:fable"), "tier top");
    for found in runtime_placeholder(&synth.coordinator_instructions) {
        assert!(runtime_filled(&found), "unresolved {{{{{found}}}}} in synthesize");
    }
    assert!(report.loop_judge_units.is_empty());
    let rlc = report.loop_configs.first().expect("report loop");
    assert_eq!(rlc.agent_ids, vec!["synthesize"]);
    assert_eq!(rlc.max_iterations, 3);
    assert!(
        matches!(
            &rlc.until,
            gaviero_core::swarm::plan::LoopUntilCondition::Command(cmd)
                if cmd == "python -B crates/gaviero-dsl/examples/deep-explore/report_gate.py research/explore"
        ),
        "{:?}",
        rlc.until
    );
    let report_budget = report.budget.as_ref().expect("report budget");
    assert_eq!(report_budget.usd, Some(10.0));
    assert_eq!(report_budget.wall_secs, Some(2 * 3600));
    assert_eq!(
        report.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}
