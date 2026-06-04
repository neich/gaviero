use gaviero_core::iteration::Strategy;
use gaviero_core::types::ModelTier;
use gaviero_dsl::compile;

const FULL_EXAMPLE: &str = r##"
    client opus   { tier coordinator model "claude-opus-4-7"    privacy public }
    client sonnet { tier execution   model "claude-sonnet-4-6"  }

    agent researcher {
        description "Research and document the codebase architecture"
        client opus
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
        client sonnet
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
fn undefined_client_error() {
    let src = r#"agent x { client ghost }"#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("ghost"), "report: {}", report);
}

#[test]
fn no_workflow_runs_all_agents() {
    let src = r#"
        agent a { description "first" }
        agent b { description "second" }
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
        agent a { description "a" }
        agent b { description "b" }
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
        agent a { description "a" }
        workflow first  { steps [a] }
        workflow second { steps [a] }
    "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("multiple workflows"), "report: {}", report);
}

#[test]
fn parse_error_missing_closing_brace() {
    let src = r#"agent x { description "hello" "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(!report.is_empty());
}

#[test]
fn dependency_cycle_detected() {
    let src = r#"
        agent a { depends_on [b] }
        agent b { depends_on [a] }
    "#;
    let err = compile(src, "test.gaviero", None, None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("cycle"), "expected cycle: {}", report);
}

// ── Iteration strategy tests ──────────────────────────────────────

#[test]
fn strategy_refine_propagated() {
    let src = r#"
        agent a { description "task" }
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
        agent a { description "task" }
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
        agent a { description "task" }
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
        agent a { description "task" }
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
        agent a { description "task" }
        workflow w { steps [a] max_retries 3 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert_eq!(compiled.iteration_config.max_retries, 3);
}

#[test]
fn verify_block_propagated() {
    let src = r#"
        agent a { description "task" }
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
        agent a { description "task" }
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
    let src = r#"agent a { description "task" }"#;
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
        agent a { description "task" }
        workflow w { steps [a] escalate_after 5 }
    "#;
    let compiled = compile(src, "test.gaviero", None, None).expect("should compile");
    assert_eq!(compiled.iteration_config.escalate_after, 5);
}

#[test]
fn attempts_propagated() {
    let src = r#"
        agent a { description "task" }
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
    gaviero_dsl::compile_file(&path, None, None, &[], &[], &[])
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
        .work_units_ordered()
        .expect("toposort")
}

#[test]
fn example_codebase_review() {
    let plan = compile_example_plan("codebase_review.gaviero");
    let units = plan.work_units_ordered().expect("toposort");

    // Sequential per-module loop with replan + execute, plus inventory
    // upstream and test_audit + final_verify downstream. The previous
    // shape included a per-iteration verify_module agent; that proved
    // fragile (sonnet sometimes never wrote the expected verify-N.md)
    // and duplicated the workflow-level `verify {compile true ...}`
    // safety net. The current shape relies on halt-propagation through
    // apply-{{ITER}}.md instead.
    // Ordering: inventory → loop body (2 agents) → test_audit → final_verify
    assert_eq!(units.len(), 5);
    let ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
    assert!(
        ids.contains(&"inventory"),
        "expected inventory in {:?}",
        ids
    );
    assert!(ids.contains(&"replan_module"));
    assert!(ids.contains(&"execute_module"));
    assert!(!ids.contains(&"verify_module"), "verify_module was removed");
    assert!(ids.contains(&"test_audit"));
    assert!(ids.contains(&"final_verify"));

    // The loop now uses an `until command "..."` shell probe instead of
    // an LLM judge — no entries in loop_judge_units.
    assert_eq!(plan.loop_judge_units.len(), 0);
    assert!(matches!(
        &plan.loop_configs[0].until,
        gaviero_core::swarm::plan::LoopUntilCondition::Command(cmd)
            if cmd.contains("apply-{{ITER}}.md") && cmd.contains("HALTED:")
    ));
    // {{OUT_DIR}} should have been substituted at compile time.
    if let gaviero_core::swarm::plan::LoopUntilCondition::Command(cmd) =
        &plan.loop_configs[0].until
    {
        assert!(
            cmd.contains("reviews/latest"),
            "expected OUT_DIR=reviews/latest substituted in command, got: {cmd}"
        );
    }

    // Single sequential loop with the two body agents.
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(
        plan.loop_configs[0].agent_ids,
        vec!["replan_module", "execute_module"]
    );
    assert_eq!(plan.loop_configs[0].max_iterations, 24);
    assert_eq!(plan.loop_configs[0].iter_start, 1);
    // max_parallel 1 — sequential is the whole point of this example.
    assert_eq!(plan.max_parallel, Some(1));
    // The loop must use stacked mode — without it, iter N's replan_module
    // doesn't see iter N-1's source edits (chain anchor isn't established).
    assert_eq!(
        plan.loop_configs[0].branch_chain,
        gaviero_core::swarm::plan::BranchChainMode::Stacked
    );

    // test_audit depends on the loop body so it runs AFTER iterations settle
    // (gated by the post-loop tier dispatch in `swarm::pipeline`).
    let test_audit = units.iter().find(|u| u.id == "test_audit").unwrap();
    assert!(
        test_audit.depends_on.iter().any(|d| d == "execute_module"),
        "test_audit should depend on execute_module, got {:?}",
        test_audit.depends_on
    );

    // final_verify depends on test_audit.
    let final_verify = units.iter().find(|u| u.id == "final_verify").unwrap();
    assert!(final_verify.depends_on.contains(&"test_audit".to_string()));
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
    gaviero_dsl::compile_file(&path, None, None, &[], &[], &[])
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
}

// ── Template compilation tests ─────────────────────────────────────

#[test]
fn template_update_docs() {
    let plan = compile_example_plan("update_docs.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 5);
    let inventory = units.iter().find(|u| u.id == "inventory").expect("inventory");
    assert_eq!(inventory.model.as_deref(), Some("claude:opus"));
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
    // Max parallel 3
    assert_eq!(plan.max_parallel, Some(3));
}

#[test]
fn update_docs_tiers_file_overrides_profile() {
    let examples = std::path::PathBuf::from(format!("{}/examples", env!("CARGO_MANIFEST_DIR")));
    let entry = examples.join("update_docs.gaviero");
    let codex_profile = examples.join("profiles/doc-codex.gaviero");
    let overrides = gaviero_dsl::load_tier_overrides(&codex_profile)
        .expect("load codex tiers profile");
    let plan = gaviero_dsl::compile_file(&entry, None, None, &[], &overrides, &[])
        .expect("compile with --tiers-file overrides");
    let inventory = plan
        .work_units_ordered()
        .expect("toposort")
        .into_iter()
        .find(|u| u.id == "inventory")
        .expect("inventory");
    assert_eq!(inventory.model.as_deref(), Some("codex:gpt-5.5"));
}

#[test]
fn template_plan_refinement() {
    let plan = compile_example_plan("plan_refinement.gaviero");
    let units = plan.work_units_ordered().expect("toposort");

    // 4 agents: 2 init + 2 refine (separate focused prompts per phase)
    assert_eq!(units.len(), 4);

    let cinit = units
        .iter()
        .find(|u| u.id == "claude-init")
        .expect("claude-init");
    let xinit = units
        .iter()
        .find(|u| u.id == "codex-init")
        .expect("codex-init");
    let crefine = units
        .iter()
        .find(|u| u.id == "claude-refine")
        .expect("claude-refine");
    let xrefine = units
        .iter()
        .find(|u| u.id == "codex-refine")
        .expect("codex-refine");

    // All four agents bind to a concrete client (not via a tier alias), so
    // the resolved `tier` is whatever the client carries. The clients in
    // plan_refinement.gaviero intentionally don't set `tier` — see the
    // comment block at the top of that file — so every unit lands on the
    // default tier. Models are what actually distinguish the agents.
    let expected_default = gaviero_core::types::ModelTier::default();
    for u in &[cinit, xinit, crefine, xrefine] {
        assert_eq!(
            u.tier, expected_default,
            "agent {} should resolve to the default tier (no tier on its client)",
            u.id
        );
    }
    assert_eq!(cinit.model.as_deref(), Some("claude:opus"));
    assert_eq!(xinit.model.as_deref(), Some("codex:gpt-5.5"));
    assert_eq!(crefine.model.as_deref(), Some("claude:opus"));
    assert_eq!(xrefine.model.as_deref(), Some("codex:gpt-5.5"));

    // Init agents: vars (MODEL_NAME, PLANS) substituted at compile time; no ITER
    assert!(
        cinit.coordinator_instructions.contains("claude-init-v1.md"),
        "claude-init prompt should reference claude-init-v1.md"
    );
    assert!(
        xinit.coordinator_instructions.contains("codex-init-v1.md"),
        "codex-init prompt should reference codex-init-v1.md"
    );
    assert!(
        !cinit.coordinator_instructions.contains("{{ITER}}"),
        "init prompt should not contain {{ITER}}"
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
        crefine
            .coordinator_instructions
            .contains("claude-refine-plan-v{{PREV_ITER}}.md"),
        "claude-refine should reference claude-refine-plan-v{{PREV_ITER}}.md"
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

    // Loop config: 2 refine agents, 5 iterations, iter_start=2,
    // plus the new judge-control knobs (stability, judge_timeout, strict_judge).
    assert_eq!(plan.loop_configs.len(), 1);
    let lc = &plan.loop_configs[0];
    assert_eq!(lc.agent_ids, vec!["claude-refine", "codex-refine"]);
    assert_eq!(lc.max_iterations, 5);
    assert_eq!(lc.iter_start, 2);
    assert_eq!(lc.stability, 2);
    assert_eq!(lc.judge_timeout_secs, 180);
    assert!(!lc.strict_judge);

    // Judge agent is compiled into the aux list, not the main DAG.
    assert_eq!(plan.loop_judge_units.len(), 1);
    assert_eq!(plan.loop_judge_units[0].id, "convergence-judge");
    // Judge prompt uses the new iteration-evidence placeholder.
    assert!(
        plan.loop_judge_units[0]
            .coordinator_instructions
            .contains("{{ITER_EVIDENCE}}"),
        "judge prompt should reference {{{{ITER_EVIDENCE}}}}"
    );

    // max_parallel 2
    assert_eq!(plan.max_parallel, Some(2));
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
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/generic_consensus.gaviero");
    let plan = gaviero_dsl::compile_file(
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
    assert!(ids.iter().any(|id| *id == "claude-init"));
    assert!(ids.iter().any(|id| *id == "codex-refine"));
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(
        plan.loop_configs[0].consensus_mode,
        gaviero_core::swarm::plan::ConsensusMode::PartialOk
    );
}

#[test]
fn generic_consensus_param_roster_override_swaps_to_three_reviewers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/generic_consensus.gaviero");
    let overrides = vec![(
        "roster".to_string(),
        "claude=claude:opus@max,codex=codex:gpt-5.5@high,cursor=cursor:composer-2.5"
            .to_string(),
    )];
    let plan = gaviero_dsl::compile_file(
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
    let plan = gaviero_dsl::compile_file(
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
}

#[test]
fn compile_file_scientific_research_param_roster_override_to_two() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let overrides = vec![(
        "roster".to_string(),
        "claude=claude:opus@max,codex=codex:gpt-5.5@high".to_string(),
    )];
    let plan = gaviero_dsl::compile_file(
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

#[test]
fn compile_file_scientific_plan_refinement_default_roster() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_plan_refinement.gaviero");
    let plan = gaviero_dsl::compile_file(
        &path,
        Some("scientific-plan-refinement"),
        Some("sparse attention study"),
        &[],
        &[],
        &[],
    )
    .expect("scientific_plan_refinement default roster should compile");
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
    assert_eq!(plan.loop_configs[0].max_iterations, 8);
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}

#[test]
fn workflow_execution_document_parsed() {
    let src = r#"
client c { tier cheap model "claude:sonnet" effort low default }
agent a { client c scope { owned ["out/"] } prompt "x" }
workflow w {
    execution_mode document
    steps [a]
}
"#;
    let plan = gaviero_dsl::compile(src, "test.gaviero", Some("w"), None).expect("compile");
    assert_eq!(
        plan.execution_mode,
        gaviero_core::swarm::plan::ExecutionMode::Document
    );
}

#[test]
fn scientific_research_judge_uses_client_param_default() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let plan = gaviero_dsl::compile_file(
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
fn client_param_override_swaps_judge_model() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/generic_consensus.gaviero");
    let overrides = vec![("judge".to_string(), "claude:haiku@low".to_string())];
    let plan = gaviero_dsl::compile_file(
        &path,
        Some("generic-consensus"),
        Some("topic"),
        &[],
        &[],
        &overrides,
    )
    .expect("judge param override should compile");
    assert_eq!(plan.loop_judge_units[0].model.as_deref(), Some("claude:haiku"));
    assert_eq!(plan.loop_judge_units[0].effort.as_deref(), Some("low"));
}

#[test]
fn client_param_without_default_requires_cli() {
    let src = concat!(
        "prompt p #\"x\"#\n",
        "agent judge { client gate prompt \"j\" }\n",
        "workflow w {\n",
        "  param gate { }\n",
        "  steps [ judge ]\n",
        "}\n",
    );
    let err =
        gaviero_dsl::compile_with_vars(src, "inline.gaviero", Some("w"), None, &[], &[], &[])
            .expect_err("required client param must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("gate") && (msg.contains("model") || msg.contains("CLI")),
        "expected client-param required diagnostic, got: {msg}"
    );
}

#[test]
fn generic_consensus_param_roster_missing_provider_is_rejected() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/generic_consensus.gaviero");
    let overrides = vec![("roster".to_string(), "claude=opus,codex=gpt-5.5".to_string())];
    let err = gaviero_dsl::compile_file(
        &path,
        Some("generic-consensus"),
        Some("t"),
        &[],
        &[],
        &overrides,
    )
    .expect_err("model without provider prefix must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("provider:model"),
        "expected provider:model diagnostic, got: {msg}"
    );
}

#[test]
fn param_without_default_or_cli_value_is_required_error() {
    let src = concat!(
        "prompt p #\"x\"#\n",
        "prompt r #\"y\"#\n",
        "client c { model \"claude:sonnet\" }\n",
        "agent tinit { template true prompt p scope { owned [\"out/x-*\"] } }\n",
        "agent tref  { template true prompt r scope { owned [\"out/y-*\"] } }\n",
        "agent judge { client c prompt \"j\" }\n",
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
    let err = gaviero_dsl::compile(src, "inline.gaviero", None, None)
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

    // Library: shared client + tier alias
    let mut lib = fs::File::create(dir.join("lib.gaviero")).unwrap();
    lib.write_all(
        br#"
            client sonnet { tier cheap model "claude:sonnet" }
            tier cheap sonnet
        "#,
    )
    .unwrap();

    // Entry: agent + workflow that reference the included client
    let entry = dir.join("main.gaviero");
    let mut f = fs::File::create(&entry).unwrap();
    f.write_all(
        br#"
            include "lib.gaviero"
            agent worker {
                description "test"
                tier cheap
                prompt "do work"
            }
            workflow w { steps [worker] }
        "#,
    )
    .unwrap();

    let plan = gaviero_dsl::compile_file(&entry, None, None, &[], &[], &[])
        .expect("compile_file should succeed across include boundary");
    let units = plan.work_units_ordered().unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "worker");
    assert_eq!(units[0].model, Some("claude:sonnet".to_string()));
    assert_eq!(units[0].tier, ModelTier::Cheap);
}

#[test]
fn compile_file_reports_duplicate_decl_across_files() {
    use std::fs;
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Both files declare `client base` — duplicate should be flagged.
    fs::File::create(dir.join("a.gaviero"))
        .unwrap()
        .write_all(br#"client base { tier cheap model "claude:sonnet" }"#)
        .unwrap();
    fs::File::create(dir.join("b.gaviero"))
        .unwrap()
        .write_all(br#"client base { tier cheap model "claude:haiku" }"#)
        .unwrap();
    let entry = dir.join("main.gaviero");
    fs::File::create(&entry)
        .unwrap()
        .write_all(
            br#"
                include "a.gaviero"
                include "b.gaviero"
                agent w { client base prompt "x" }
                workflow wf { steps [w] }
            "#,
        )
        .unwrap();

    let err = gaviero_dsl::compile_file(&entry, None, None, &[], &[], &[]).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("duplicate client name `base`"),
        "expected duplicate-client diagnostic, got: {}",
        msg
    );
}

#[test]
fn compile_rejects_inline_include_with_helpful_diagnostic() {
    let src = r#"
        include "lib.gaviero"
        client c { tier cheap model "claude:sonnet" }
    "#;
    let err = gaviero_dsl::compile(src, "inline.gaviero", None, None).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("compile_file") || msg.contains("--script"),
        "expected diagnostic to point at compile_file/--script, got: {}",
        msg
    );
}
