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
    let path = format!("{}/examples/{}", env!("CARGO_MANIFEST_DIR"), filename);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {}", path, e));
    compile(&source, filename, None, None)
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
        .work_units_ordered()
        .expect("toposort")
}

#[test]
fn example_bugfix_with_tests() {
    let units = compile_example("bugfix_with_tests.gaviero");
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].id, "diagnose");
    assert_eq!(units[1].id, "fix");
    assert_eq!(units[2].id, "verify");
    // verify depends on fix, fix depends on diagnose
    assert_eq!(units[1].depends_on, vec!["diagnose"]);
    assert_eq!(units[2].depends_on, vec!["fix"]);
    // diagnose uses coordinator tier (opus) → maps to Expensive
    assert_eq!(units[0].tier, ModelTier::Expensive);
    // verify has max_retries 3
    assert_eq!(units[2].max_retries, 3);
}

#[test]
fn example_feature_tdd() {
    let units = compile_example("feature_tdd.gaviero");
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].id, "write_tests");
    assert_eq!(units[2].id, "verify_no_regressions");
    assert!(units[0].coordinator_instructions.contains("TDD"));
    assert!(units[2].coordinator_instructions.contains("cargo test"));
}

#[test]
fn example_refactor_safe() {
    let units = compile_example("refactor_safe.gaviero");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "analyze_coverage");
    assert_eq!(units[3].id, "verify");
    // analyze_structure depends on analyze_coverage
    assert_eq!(units[1].depends_on, vec!["analyze_coverage"]);
    // haiku tier for coverage agent (mechanical → Cheap)
    assert_eq!(units[0].tier, ModelTier::Cheap);
}

#[test]
fn example_multi_crate_test() {
    let units = compile_example("multi_crate_test.gaviero");
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].id, "change_core");
    assert_eq!(units[2].id, "workspace_test");
    assert!(
        units[2]
            .coordinator_instructions
            .contains("cargo test --workspace")
    );
}

#[test]
fn example_security_audit() {
    let units = compile_example("security_audit.gaviero");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "scan");
    assert_eq!(units[3].id, "final_verification");
    // scan uses reasoning tier → maps to Expensive
    assert_eq!(units[0].tier, ModelTier::Expensive);
    // write_security_tests owns the tests/security/ directory
    assert!(
        units[2]
            .scope
            .owned_paths
            .contains(&"tests/security/".to_string())
    );
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
    let path = format!("{}/examples/{}", env!("CARGO_MANIFEST_DIR"), filename);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {}", path, e));
    compile(&source, filename, None, None)
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
}

// ── Template compilation tests ─────────────────────────────────────

#[test]
fn template_feature_iterative() {
    let plan = compile_example_plan("feature_iterative.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "orchestrator");
    assert_eq!(units[3].id, "summarize");
    // Has a loop config
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(
        plan.loop_configs[0].agent_ids,
        vec!["implement", "write_tests"]
    );
    assert_eq!(plan.loop_configs[0].max_iterations, 5);
    // Orchestrator reads from memory
    assert!(
        units[0]
            .read_namespaces
            .as_ref()
            .unwrap()
            .contains(&"architecture".to_string())
    );
    // Summarize writes to memory with custom content
    assert_eq!(units[3].write_namespace.as_deref(), Some("feature-history"));
    assert!(units[3].memory_write_content.is_some());
}

#[test]
fn template_refactor_codebase() {
    let plan = compile_example_plan("refactor_codebase.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "analyse");
    assert_eq!(units[3].id, "record_changes");
    // Has a loop config
    assert_eq!(plan.loop_configs.len(), 1);
    assert_eq!(
        plan.loop_configs[0].agent_ids,
        vec!["refactor", "fix_tests"]
    );
    assert_eq!(plan.loop_configs[0].max_iterations, 8);
    // Analyse has custom read_query
    assert!(units[0].memory_read_query.is_some());
    assert_eq!(units[0].memory_read_limit, Some(15));
    // record_changes writes with custom template
    assert!(units[3].memory_write_content.is_some());
    assert_eq!(units[3].memory_importance, Some(0.9));
}

#[test]
fn template_update_docs() {
    let plan = compile_example_plan("update_docs.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 5);
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
fn template_sync_memory() {
    let plan = compile_example_plan("sync_memory.gaviero");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "audit_codebase");
    // Three reconcile agents depend on audit
    assert!(units[1].depends_on.contains(&"audit_codebase".to_string()));
    assert!(units[2].depends_on.contains(&"audit_codebase".to_string()));
    assert!(units[3].depends_on.contains(&"audit_codebase".to_string()));
    // audit reads with custom query and high limit
    assert!(units[0].memory_read_query.is_some());
    assert_eq!(units[0].memory_read_limit, Some(20));
    // reconcile_architecture has staleness_sources
    let arch_agent = units
        .iter()
        .find(|u| u.id == "reconcile_architecture")
        .unwrap();
    assert!(!arch_agent.staleness_sources.is_empty());
    assert!(arch_agent.memory_write_content.is_some());
    // No loops
    assert!(plan.loop_configs.is_empty());
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

    // All use expensive tier
    for u in &[cinit, xinit, crefine, xrefine] {
        assert_eq!(
            u.tier,
            gaviero_core::types::ModelTier::Expensive,
            "agent {} should be expensive",
            u.id
        );
    }

    // Init agents: vars (MODEL_NAME, PLANS) substituted at compile time; no ITER
    assert!(
        cinit.coordinator_instructions.contains("claude-plan-v1.md"),
        "claude-init prompt should reference claude-plan-v1.md"
    );
    assert!(
        xinit.coordinator_instructions.contains("codex-plan-v1.md"),
        "codex-init prompt should reference codex-plan-v1.md"
    );
    assert!(
        !cinit.coordinator_instructions.contains("{{ITER}}"),
        "init prompt should not contain {{ITER}}"
    );

    // Refine agents: vars substituted; ITER/PREV_ITER survive for runtime
    assert!(
        crefine
            .coordinator_instructions
            .contains("claude-plan-v{{ITER}}.md"),
        "claude-refine should reference claude-plan-v{{ITER}}.md"
    );
    assert!(
        xrefine
            .coordinator_instructions
            .contains("codex-plan-v{{ITER}}.md"),
        "codex-refine should reference codex-plan-v{{ITER}}.md"
    );
    assert!(
        crefine
            .coordinator_instructions
            .contains("claude-plan-v{{PREV_ITER}}.md"),
        "claude-refine should reference claude-plan-v{{PREV_ITER}}.md"
    );

    // Summary file also uses ITER
    assert!(
        crefine
            .coordinator_instructions
            .contains("claude-summary-v{{ITER}}.md"),
        "claude-refine should reference summary file"
    );

    // Refine agents write to memory
    assert_eq!(crefine.write_namespace.as_deref(), Some("plan-evolution"));
    assert_eq!(xrefine.write_namespace.as_deref(), Some("plan-evolution"));

    // Loop config: 2 refine agents, 10 iterations, iter_start=2,
    // plus the new judge-control knobs (stability, judge_timeout, strict_judge).
    assert_eq!(plan.loop_configs.len(), 1);
    let lc = &plan.loop_configs[0];
    assert_eq!(lc.agent_ids, vec!["claude-refine", "codex-refine"]);
    assert_eq!(lc.max_iterations, 10);
    assert_eq!(lc.iter_start, 2);
    assert_eq!(lc.stability, 2);
    assert_eq!(lc.judge_timeout_secs, 90);
    assert!(lc.strict_judge);

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

    // Tier routing: expensive on reasoning agents, cheap on gate and judge.
    let tier_of = |id: &str| {
        units
            .iter()
            .chain(plan.loop_judge_units.iter())
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("agent {} not found", id))
            .tier
    };
    assert_eq!(
        tier_of("analyse_plan"),
        gaviero_core::types::ModelTier::Expensive
    );
    assert_eq!(
        tier_of("phase_executor"),
        gaviero_core::types::ModelTier::Expensive
    );
    assert_eq!(
        tier_of("final_audit"),
        gaviero_core::types::ModelTier::Expensive
    );
    assert_eq!(
        tier_of("phase_gate"),
        gaviero_core::types::ModelTier::Cheap
    );
    assert_eq!(
        tier_of("phase_judge"),
        gaviero_core::types::ModelTier::Cheap
    );

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
    assert_eq!(dep("phase_gate"), vec!["phase_executor"]);
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
