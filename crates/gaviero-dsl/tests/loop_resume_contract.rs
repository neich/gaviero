//! DSL ↔ `swarm::loop_resume` contract integration test.
//!
//! `loop_resume::detect` reads two things the DSL compiler produces:
//! `LoopConfig.verdict_output_dir` (from the script's `OUT_DIR` var) and each
//! roster reviewer's `scope.owned` globs with `{{OUT_DIR}}` / `{{REVIEWER_ID}}`
//! already substituted. If either stops being populated, resume silently
//! degrades to "start over and overwrite the panel" — a data-loss-shaped
//! regression that no unit test on either side would catch alone.
//!
//! These tests compile the shipped examples and drive detection against a
//! synthesised artefact tree, so the whole path from `.gaviero` source to
//! resume point is exercised.

use std::collections::HashMap;
use std::path::Path;

use gaviero_core::swarm::loop_resume;
use gaviero_core::swarm::models::WorkUnit;

fn example(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

fn unit_map<'a>(units: &[&'a WorkUnit]) -> HashMap<&'a str, &'a WorkUnit> {
    units.iter().map(|u| (u.id.as_str(), *u)).collect()
}

/// Compile `plan_refinement.gaviero` the way the operator's CLI invocation
/// did, with a three-provider roster and a relocated `OUT_DIR`.
fn compile_plan_refinement(out_dir: &str) -> gaviero_core::swarm::plan::CompiledPlan {
    gaviero_dsl::compile_file(
        &example("plan_refinement.gaviero"),
        Some("feature-plan-refinement"),
        Some("feature brief"),
        &[("OUT_DIR".to_string(), out_dir.to_string())],
        &[],
        &[(
            "roster".to_string(),
            "claude=claude:opus@max,codex=codex:gpt-5.5@high,cursor=cursor:composer-2.5@high"
                .to_string(),
        )],
    )
    .expect("plan_refinement.gaviero compiles")
}

#[test]
fn compiler_populates_the_fields_resume_depends_on() {
    let plan = compile_plan_refinement("plans/research-consensus-ux");
    let lc = plan
        .loop_configs
        .first()
        .expect("plan_refinement declares one loop block");

    assert_eq!(
        lc.verdict_output_dir.as_deref(),
        Some("plans/research-consensus-ux"),
        "OUT_DIR must reach LoopConfig or resume cannot find the artefacts"
    );
    assert_eq!(lc.agent_ids.len(), 3, "one loop body agent per roster entry");

    let units = plan.work_units_unordered();
    let map = unit_map(&units);
    for agent_id in &lc.agent_ids {
        let unit = map.get(agent_id.as_str()).expect("loop agent is a work unit");
        assert!(
            !unit.scope.owned_paths.is_empty(),
            "{agent_id} has no owned paths — resume cannot attribute artefacts"
        );
        assert!(
            unit.scope
                .owned_paths
                .iter()
                .all(|p| !p.contains("{{") && p.starts_with("plans/research-consensus-ux/")),
            "{agent_id} owned paths must be fully substituted, got {:?}",
            unit.scope.owned_paths
        );
    }
}

#[test]
fn detects_the_third_completed_round_of_a_three_provider_panel() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = "plans/research-consensus-ux";
    // Reproduce the shape a completed 3-round run leaves behind.
    for v in 1..=3 {
        for id in ["claude", "codex", "cursor"] {
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-refine-plan-v{v}.md"),
                "# plan",
            );
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-refine-summary-v{v}.md"),
                "# summary",
            );
        }
    }

    let plan = compile_plan_refinement(out_dir);
    let lc = plan.loop_configs.first().expect("loop block");
    let units = plan.work_units_unordered();

    let resume = loop_resume::detect(tmp.path(), lc, &unit_map(&units))
        .expect("three complete rounds are a resume point");

    assert_eq!(resume.last_complete_iter, 3);
    assert_eq!(resume.resume_iter_start, 4, "next round is v4, reading v3");
    assert_eq!(resume.original_iter_start, 1);
    assert_eq!(resume.reused.len(), 6);
    assert!(resume.discarded.is_empty());
    // plan_refinement has no init template, so nothing to suppress.
    assert!(resume.satisfied_init_units.is_empty());
}

#[test]
fn a_crashed_round_is_discarded_rather_than_resumed_into() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = "plans/research-consensus-ux";
    for v in 1..=3 {
        for id in ["claude", "codex", "cursor"] {
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-refine-plan-v{v}.md"),
                "# plan",
            );
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-refine-summary-v{v}.md"),
                "# summary",
            );
        }
    }
    // v4 died after one provider finished and a second wrote half its output.
    write(
        tmp.path(),
        &format!("{out_dir}/claude-refine-plan-v4.md"),
        "# plan",
    );
    write(
        tmp.path(),
        &format!("{out_dir}/claude-refine-summary-v4.md"),
        "# summary",
    );
    write(
        tmp.path(),
        &format!("{out_dir}/codex-refine-plan-v4.md"),
        "# plan",
    );

    let plan = compile_plan_refinement(out_dir);
    let lc = plan.loop_configs.first().expect("loop block");
    let units = plan.work_units_unordered();

    let resume = loop_resume::detect(tmp.path(), lc, &unit_map(&units)).expect("resume point");

    assert_eq!(
        resume.last_complete_iter, 3,
        "v4 was never finished by the whole panel"
    );
    assert_eq!(resume.discarded.len(), 3);
    assert!(resume.notes.iter().any(|n| n.contains("v4")));
}

/// `scientific_research.gaviero` adds an init template and `iter_start 2`,
/// so resume must also suppress the baseline agents.
#[test]
fn scientific_research_resume_suppresses_the_init_round() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = "research/run-a";
    for id in ["claude", "codex"] {
        write(
            tmp.path(),
            &format!("{out_dir}/{id}-conclusion-v1.md"),
            "# conclusion",
        );
        write(
            tmp.path(),
            &format!("{out_dir}/{id}-evidence-v1.md"),
            "# evidence",
        );
        for v in 2..=4 {
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-conclusion-v{v}.md"),
                "# conclusion",
            );
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-summary-v{v}.md"),
                "# summary",
            );
            write(
                tmp.path(),
                &format!("{out_dir}/{id}-evidence-v{v}.md"),
                "# evidence",
            );
        }
    }
    write(tmp.path(), &format!("{out_dir}/problem.md"), "# problem");

    let plan = gaviero_dsl::compile_file(
        &example("scientific_research.gaviero"),
        Some("scientific-research-consensus"),
        Some("research topic"),
        &[
            ("OUT_DIR".to_string(), out_dir.to_string()),
            (
                "PROBLEM_FILE".to_string(),
                format!("{out_dir}/problem.md"),
            ),
        ],
        &[],
        &[(
            "roster".to_string(),
            "claude=claude:opus@max,codex=codex:gpt-5.5@high".to_string(),
        )],
    )
    .expect("scientific_research.gaviero compiles");

    let lc = plan.loop_configs.first().expect("loop block");
    assert_eq!(lc.iter_start, 2, "script declares iter_start 2");
    let units = plan.work_units_unordered();

    let resume = loop_resume::detect(tmp.path(), lc, &unit_map(&units)).expect("resume point");

    assert_eq!(resume.last_complete_iter, 4);
    assert_eq!(resume.resume_iter_start, 5);
    assert_eq!(
        resume.satisfied_init_units.len(),
        2,
        "both `<id>-init` baselines are already on disk and must not re-run"
    );
    assert!(
        resume
            .satisfied_init_units
            .iter()
            .all(|id| id.ends_with("-init"))
    );
}

#[test]
fn a_fresh_out_dir_yields_no_resume_point() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = "plans/research-consensus-ux";
    std::fs::create_dir_all(tmp.path().join(out_dir)).expect("mkdir");

    let plan = compile_plan_refinement(out_dir);
    let lc = plan.loop_configs.first().expect("loop block");
    let units = plan.work_units_unordered();

    assert!(
        loop_resume::detect(tmp.path(), lc, &unit_map(&units)).is_none(),
        "an empty OUT_DIR must run the panel from iter_start"
    );
}
