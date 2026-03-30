use gaviero_core::types::ModelTier;
use gaviero_dsl::compile;

const FULL_EXAMPLE: &str = r##"
    client opus   { tier coordinator model "claude-opus-4-6"    privacy public }
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
    let units = compile(FULL_EXAMPLE, "test.gaviero", None)
        .expect("should compile without errors");

    assert_eq!(units.len(), 2, "expected 2 work units");

    // researcher
    assert_eq!(units[0].id, "researcher");
    assert_eq!(units[0].tier, ModelTier::Coordinator);
    assert_eq!(units[0].model, Some("claude-opus-4-6".to_string()));
    assert!(
        units[0].coordinator_instructions.contains("architecture document"),
        "prompt: {}",
        units[0].coordinator_instructions
    );
    assert_eq!(units[0].max_retries, 2);
    assert_eq!(units[0].scope.owned_paths, vec!["docs/architecture.md"]);
    assert_eq!(units[0].scope.read_only_paths, vec!["src/**"]);
    assert!(units[0].depends_on.is_empty());

    // implementer
    assert_eq!(units[1].id, "implementer");
    assert_eq!(units[1].tier, ModelTier::Execution);
    assert_eq!(units[1].model, Some("claude-sonnet-4-6".to_string()));
    assert_eq!(units[1].depends_on, vec!["researcher"]);
    assert_eq!(units[1].scope.owned_paths, vec!["src/feature/"]);
}

#[test]
fn lex_error_is_reported() {
    let src = "agent @ bad { }";
    let err = compile(src, "test.gaviero", None);
    assert!(err.is_err(), "expected error for invalid character");
    let report = format!("{:?}", err.unwrap_err());
    assert!(report.contains("unexpected"), "report: {}", report);
}

#[test]
fn undefined_client_error() {
    let src = r#"agent x { client ghost }"#;
    let err = compile(src, "test.gaviero", None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("ghost"), "report: {}", report);
}

#[test]
fn no_workflow_runs_all_agents() {
    let src = r#"
        agent a { description "first" }
        agent b { description "second" }
    "#;
    let units = compile(src, "test.gaviero", None).unwrap();
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
    let units = compile(src, "test.gaviero", Some("just_b")).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "b");
}

#[test]
fn parse_error_missing_closing_brace() {
    let src = r#"agent x { description "hello" "#;
    let err = compile(src, "test.gaviero", None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(!report.is_empty());
}

#[test]
fn dependency_cycle_detected() {
    let src = r#"
        agent a { depends_on [b] }
        agent b { depends_on [a] }
    "#;
    let err = compile(src, "test.gaviero", None).unwrap_err();
    let report = format!("{:?}", err);
    assert!(report.contains("cycle"), "expected cycle: {}", report);
}

// ── Example file compilation tests ──────────────────────────────

fn compile_example(filename: &str) -> Vec<gaviero_core::swarm::models::WorkUnit> {
    let path = format!(
        "{}/examples/{}",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {}", path, e));
    compile(&source, filename, None)
        .unwrap_or_else(|e| panic!("compiling {}:\n{:?}", filename, e))
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
    // diagnose uses coordinator tier (opus)
    assert_eq!(units[0].tier, ModelTier::Coordinator);
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
    // haiku tier for coverage agent
    assert_eq!(units[0].tier, ModelTier::Mechanical);
}

#[test]
fn example_multi_crate_test() {
    let units = compile_example("multi_crate_test.gaviero");
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].id, "change_core");
    assert_eq!(units[2].id, "workspace_test");
    assert!(units[2].coordinator_instructions.contains("cargo test --workspace"));
}

#[test]
fn example_security_audit() {
    let units = compile_example("security_audit.gaviero");
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].id, "scan");
    assert_eq!(units[3].id, "final_verification");
    // scan uses reasoning tier
    assert_eq!(units[0].tier, ModelTier::Reasoning);
    // write_security_tests owns the tests/security/ directory
    assert!(units[2].scope.owned_paths.contains(&"tests/security/".to_string()));
}
