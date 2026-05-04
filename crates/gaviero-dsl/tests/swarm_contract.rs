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

/// T5 — Coordinator-emitted DSL shape compiles cleanly through the DSL
/// pipeline. `swarm::coordinator::build_coordinator_dsl_prompt` instructs
/// the model to produce a `.gaviero` workflow with this exact structure
/// (clients with `tier <kw>` + `model`, `agent { description, client, scope,
/// depends_on, prompt #"..."# }` blocks, `workflow main { steps [...] }`).
/// If the prompt template ever drifts past what `gaviero_dsl::compile`
/// accepts, this test catches it at the DSL crate boundary instead of at
/// coordinator runtime.
///
/// The verified contract: compile → topo-sort → swarm scope/dependency
/// validation must all succeed for a representative coordinator output.
///
/// Client names are free identifiers; the coordinator's "use `reasoning`,
/// `execution`, `mechanical` as client names" instruction is broken
/// against the lexer (those are reserved tier-value keywords). See the
/// `coordinator_prompt_uses_reserved_tier_words_as_client_names` test
/// below for the regression that pins that contract bug.
#[test]
fn coordinator_dsl_template_compiles_and_passes_swarm_validation() {
    // Mirror the structural shape `build_coordinator_dsl_prompt` instructs
    // the model to emit:
    //   - one client per tier
    //   - agents with description, client, scope { owned, read_only },
    //     depends_on, prompt #" "#, max_retries
    //   - workflow main { steps [...] max_parallel <n> }
    let src = r##"
        client opus    { tier expensive model "claude:opus" }
        client sonnet  { tier cheap     model "claude:sonnet" }
        client haiku   { tier cheap     model "claude:haiku" }

        agent designer {
            description "Design the new module structure"
            client opus
            scope {
                owned    ["docs/design.md"]
                read_only ["src/"]
            }
            prompt #"
                Read the existing code under src/ and produce a one-page
                design doc describing the new module layout.
            "#
            max_retries 2
        }

        agent implementer {
            description "Implement the module per the design"
            client sonnet
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
            client haiku
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

    let plan = gaviero_dsl::compile(src, "coordinator-template.gaviero", None, None)
        .expect("DSL must accept the coordinator's emit shape");

    let units = plan.work_units_ordered().expect("toposort");
    assert_eq!(units.len(), 3, "expected 3 work units");
    assert_eq!(plan.max_parallel, Some(2));

    // Scope validation matches what `pipeline::execute` runs.
    let loop_groups: Vec<Vec<String>> = plan
        .loop_configs
        .iter()
        .map(|lc| lc.agent_ids.clone())
        .collect();
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
        client sonnet { tier cheap model "claude:sonnet" }

        agent a {
            description "a"
            client sonnet
            scope { owned ["src/a/"] }
        }
        agent b {
            description "b"
            client sonnet
            scope { owned ["src/b/"] }
        }
        agent c {
            description "c"
            client sonnet
            scope { owned ["src/c/"] }
        }

        workflow main {
            steps [a b c]
            max_parallel 3
        }
    "##;

    let plan = gaviero_dsl::compile(src, "coordinator-parallel.gaviero", None, None)
        .expect("DSL compile");
    let units = plan.work_units_ordered().expect("toposort");
    let tiers = validation::dependency_tiers(&units).expect("tiers");
    assert_eq!(
        tiers.len(),
        1,
        "independent agents must collapse to a single parallel tier"
    );
    assert_eq!(tiers[0].len(), 3);
}

/// Regression: pins the contract bug between
/// `swarm::coordinator::build_coordinator_dsl_prompt` and the DSL lexer.
///
/// The coordinator's prompt template tells the model:
///   ```
///   CLIENT NAMES to use:
///   - `reasoning` for reasoning-tier agents
///   - `execution` for execution-tier agents
///   - `mechanical` for mechanical-tier agents
///   ```
/// but the DSL lexer reserves `reasoning`, `execution`, and `mechanical`
/// as tier-value keywords, so a model that follows the instructions
/// produces DSL the parser rejects. Until the prompt is fixed, swarm
/// `--coordinated` runs that obey the prompt are doomed to a parse error.
///
/// This test asserts the failure today (the DSL rejects `client reasoning`)
/// so a future fix to the prompt → grammar contract surfaces here as a
/// failure-to-failure (the test would pass — flipping the assertion is
/// the signal to also delete this regression).
#[test]
fn coordinator_prompt_uses_reserved_tier_words_as_client_names() {
    let src = r#"
        client reasoning { tier expensive model "claude:opus" }
        agent a {
            description "a"
            client reasoning
            scope { owned ["src/a/"] }
        }
    "#;
    let result = gaviero_dsl::compile(src, "regression.gaviero", None, None);
    assert!(
        result.is_err(),
        "DSL still rejects `client reasoning` — if this test starts to pass, \
         the lexer or prompt has been fixed; remove this regression."
    );
}
