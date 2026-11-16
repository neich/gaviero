//! T4 — DSL ↔ swarm contract integration test.
//!
//! `gaviero-dsl` and `gaviero-core::swarm::validation` independently
//! enforce scope-overlap rules using the same `path_pattern`
//! primitive. The integration contract is:
//!
//! - A `.gaviero` source the DSL accepts must produce a `CompiledPlan`
//!   whose work units pass `swarm::validation::validate_scopes`.
//! - A source the DSL rejects must surface a scope-related diagnostic
//!   so users see the failure at compile time, not at swarm bail.
//!
//! The negative case from the runtime side (validator bailing inside
//! `swarm::pipeline::execute`) is covered in
//! `crates/gaviero-core/tests/swarm_execute_validation.rs`. The two
//! files together pin the symmetry from both directions.

use gaviero_core::script_profiles::{ProfilePrecedence, SelectedProfile};
use gaviero_core::swarm::validation;

/// The profile shipped with the examples: generic tiers low / medium /
/// high / top, as a `--coordinated` plan finds in a `default` profile.
fn profile() -> SelectedProfile {
    SelectedProfile {
        name: "default".into(),
        path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/profiles/default.gaviero"),
        precedence: ProfilePrecedence::FillsGaps,
    }
}

fn compile(src: &str, filename: &str) -> Result<gaviero_dsl::CompiledPlan, miette::Report> {
    gaviero_dsl::compile(src, filename, Some(&profile()), None, None)
}

#[test]
fn dsl_compiled_plan_passes_swarm_validate_scopes() {
    let src = r#"
        agent reader {
            description "read"
            tier medium
            scope { owned ["docs/"] read_only ["src/"] }
        }
        agent writer {
            description "write"
            tier medium
            depends_on [reader]
            scope { owned ["src/feature/"] read_only ["docs/"] }
        }
        workflow w { steps [reader writer] }
    "#;

    let plan = compile(src, "t4.gaviero").expect("DSL should compile this two-agent workflow");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 2);

    let loop_groups = validation::expand_loop_groups_with_roster_init(
        plan.loop_configs
            .iter()
            .map(|lc| lc.agent_ids.clone())
            .collect(),
        &units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
    );
    let errors = validation::validate_scopes(&units, &loop_groups);
    assert!(
        errors.is_empty(),
        "DSL-accepted plan must also pass swarm scope validation; got: {errors:?}"
    );
}

#[test]
fn scientific_research_roster_passes_swarm_scope_validation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/scientific_research.gaviero");
    let plan = gaviero_dsl::compile_file_with_profile(
        &path,
        Some(&profile()),
        Some("scientific-research-consensus"),
        Some("test topic"),
        &[],
        &[],
        &[],
    )
    .expect("scientific_research should compile");
    let units = plan.work_units_ordered().expect("toposort");
    let loop_groups = validation::expand_loop_groups_with_roster_init(
        plan.loop_configs
            .iter()
            .map(|lc| lc.agent_ids.clone())
            .collect(),
        &units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
    );
    let errors = validation::validate_scopes(&units, &loop_groups);
    assert!(
        errors.is_empty(),
        "scientific_research init/refine scopes must validate; got: {errors:?}"
    );
}

#[test]
fn dsl_does_not_catch_scope_overlap_swarm_validator_does() {
    // Documents the actual division of responsibility: the DSL
    // accepts overlapping owned scopes silently (only emits a soft
    // workflow-shape warning to stderr); the swarm validator catches
    // it at runtime. This is what `gaviero-cli` users experience —
    // every overlap survives compile and bails on `swarm::execute`.
    //
    // If a future change moves the check into the DSL, flip this test
    // to expect a compile-time error and update CLAUDE.md to match.
    let src = r#"
        agent a {
            description "a"
            tier medium
            scope { owned ["src/"] }
        }
        agent b {
            description "b"
            tier medium
            scope { owned ["src/"] }
        }
        workflow w { steps [a b] }
    "#;

    // DSL accepts the source.
    let plan = compile(src, "t4-overlap.gaviero")
        .expect("DSL accepts overlap today (validation lives in swarm runtime)");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 2);

    // Swarm validator with no loop groups (none declared in workflow)
    // catches it.
    let errors = validation::validate_scopes(&units, &[]);
    assert!(
        !errors.is_empty(),
        "swarm runtime must reject what the DSL silently accepts; got no errors"
    );
    let msg: String = errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        msg.to_lowercase().contains("overlap") || msg.to_lowercase().contains("scope"),
        "swarm error must reference scope/overlap; got: {msg}"
    );
}

/// T5 — Coordinator-emitted DSL shape compiles cleanly through the DSL
/// pipeline. `swarm::coordinator::build_coordinator_dsl_prompt` instructs
/// the model to produce a `.gaviero` workflow with this exact structure
/// (`agent { description, tier <low|medium|high>, scope, depends_on,
/// produces, timeout, prompt #"..."#, max_retries }` blocks, no clients,
/// `workflow main { steps [...] }`). If the prompt template ever drifts past
/// what `gaviero_dsl::compile` accepts, this test catches it at the DSL
/// crate boundary instead of at coordinator runtime.
///
/// The verified contract: compile (against the run's profile) → topo-sort →
/// swarm scope/dependency validation must all succeed for a representative
/// coordinator output.
#[test]
fn coordinator_dsl_template_compiles_and_passes_swarm_validation() {
    let src = r##"
        agent designer {
            description "Design the new module structure"
            tier high
            scope {
                owned    ["docs/design.md"]
                read_only ["src/"]
            }
            produces ["docs/design.md"]
            timeout 1800
            prompt #"
                Read the existing code under src/ and produce a one-page
                design doc describing the new module layout.
            "#
            max_retries 2
        }

        agent implementer {
            description "Implement the module per the design"
            tier medium
            depends_on [designer]
            scope {
                owned    ["src/feature/"]
                read_only ["docs/design.md"]
            }
            prompt #"
                Implement the feature per docs/design.md. Keep changes
                inside src/feature/.
            "#
        }

        agent renamer {
            description "Update call sites for the rename"
            tier low
            depends_on [implementer]
            scope {
                owned    ["src/api/"]
                read_only ["src/feature/"]
            }
            prompt #"
                Propagate the rename produced by `implementer` across
                src/api/ call sites.
            "#
        }

        workflow main {
            steps [designer implementer renamer]
            max_parallel 2
        }
    "##;

    let plan = compile(src, "coordinator-template.gaviero")
        .expect("DSL must accept the coordinator's emit shape");

    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 3, "expected 3 work units");
    assert_eq!(plan.max_parallel, Some(2));
    assert_eq!(units[0].model.as_deref(), Some("claude:opus"), "tier high");
    assert_eq!(units[2].model.as_deref(), Some("claude:haiku"), "tier low");

    // Scope validation matches what `pipeline::execute` runs.
    let loop_groups = validation::expand_loop_groups_with_roster_init(
        plan.loop_configs
            .iter()
            .map(|lc| lc.agent_ids.clone())
            .collect(),
        &units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
    );
    let scope_errors = validation::validate_scopes(&units, &loop_groups);
    assert!(
        scope_errors.is_empty(),
        "coordinator template must scope-validate; got: {scope_errors:?}"
    );

    // Dependency tier computation matches `pipeline::execute` Phase 2.
    let tiers = validation::dependency_tiers(&units)
        .expect("coordinator template must produce a valid DAG");
    assert_eq!(tiers.len(), 3, "expected linear chain of 3 tiers");
    assert_eq!(tiers[0], vec!["designer"]);
    assert_eq!(tiers[1], vec!["implementer"]);
    assert_eq!(tiers[2], vec!["renamer"]);
}

/// Companion to T5: the coordinator's prompt also documents that
/// independent agents (no `depends_on`) should run in parallel. Verify
/// the DSL produces a single-tier topology for that shape so a future
/// prompt edit that introduces phantom dependencies is caught here.
#[test]
fn coordinator_dsl_independent_agents_collapse_to_one_tier() {
    let src = r##"
        agent a {
            description "a"
            tier medium
            scope { owned ["src/a/"] }
        }
        agent b {
            description "b"
            tier medium
            scope { owned ["src/b/"] }
        }
        agent c {
            description "c"
            tier medium
            scope { owned ["src/c/"] }
        }

        workflow main {
            steps [a b c]
            max_parallel 3
        }
    "##;

    let plan = compile(src, "coordinator-parallel.gaviero").expect("DSL compile");
    let units = plan.work_units_ordered().expect("toposort");
    let tiers = validation::dependency_tiers(&units).expect("tiers");
    assert_eq!(
        tiers.len(),
        1,
        "independent agents must collapse to a single parallel tier"
    );
    assert_eq!(tiers[0].len(), 3);
}

/// The plan the coordinator emits never declares clients: a plan that does
/// fails to compile, so the template must keep teaching `tier` only.
#[test]
fn a_coordinator_plan_with_clients_does_not_compile() {
    let src = r#"
        client deep { model "claude:opus" }
        agent a {
            description "a"
            tier high
            scope { owned ["out/"] }
            produces ["out/a.md"]
            timeout 600
        }
    "#;
    let err = compile(src, "coordinator-names.gaviero").unwrap_err();
    assert!(format!("{err:?}").contains("cannot declare `client deep`"), "{err:?}");

    let plan = compile(&src.replace(r#"client deep { model "claude:opus" }"#, ""), "c.gaviero")
        .expect("the same plan without the client compiles");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units[0].produces, vec!["out/a.md".to_string()]);
    assert_eq!(units[0].timeout_secs, 600);
}
