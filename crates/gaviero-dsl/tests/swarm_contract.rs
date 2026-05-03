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

use gaviero_core::swarm::validation;

#[test]
fn dsl_compiled_plan_passes_swarm_validate_scopes() {
    let src = r#"
        client base { tier cheap model "claude:sonnet" }
        agent reader {
            description "read"
            client base
            scope { owned ["docs/"] read_only ["src/"] }
        }
        agent writer {
            description "write"
            client base
            depends_on [reader]
            scope { owned ["src/feature/"] read_only ["docs/"] }
        }
        workflow w { steps [reader writer] }
    "#;

    let plan = gaviero_dsl::compile(src, "t4.gaviero", None, None)
        .expect("DSL should compile this two-agent workflow");
    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 2);

    let loop_groups: Vec<Vec<String>> = plan
        .loop_configs
        .iter()
        .map(|lc| lc.agent_ids.clone())
        .collect();
    let errors = validation::validate_scopes(&units, &loop_groups);
    assert!(
        errors.is_empty(),
        "DSL-accepted plan must also pass swarm scope validation; got: {errors:?}"
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
        client base { tier cheap model "claude:sonnet" }
        agent a {
            description "a"
            client base
            scope { owned ["src/"] }
        }
        agent b {
            description "b"
            client base
            scope { owned ["src/"] }
        }
        workflow w { steps [a b] }
    "#;

    // DSL accepts the source.
    let plan = gaviero_dsl::compile(src, "t4-overlap.gaviero", None, None)
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
    let msg: String = errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
    assert!(
        msg.to_lowercase().contains("overlap") || msg.to_lowercase().contains("scope"),
        "swarm error must reference scope/overlap; got: {msg}"
    );
}
