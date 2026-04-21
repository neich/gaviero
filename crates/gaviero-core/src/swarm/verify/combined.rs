//! Combined verification: runs structural + diff review + tests in sequence
//! with early termination on failure.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::diff_review::{DiffReviewConfig, DiffReviewer, ReviewableDiff};
use super::structural;
use super::test_runner::{self, TestRunnerConfig};
use super::{
    CombinedReport, CostEstimate, DiffReviewReport, EscalationReason, EscalationRecord,
    FailureSeverity, TestReport, VerificationStrategy,
};
use crate::observer::SwarmObserver;
use crate::swarm::models::{AgentManifest, WorkUnit};
use crate::types::ModelTier;

/// Run the verification pipeline based on the selected strategy.
pub async fn run_verification(
    strategy: &VerificationStrategy,
    manifests: &[AgentManifest],
    units: &[WorkUnit],
    workspace_root: &Path,
    observer: &dyn SwarmObserver,
) -> Result<CombinedReport> {
    // Collect all modified files across all manifests
    let all_modified: Vec<PathBuf> = manifests
        .iter()
        .filter(|m| matches!(m.status, crate::swarm::models::AgentStatus::Completed))
        .flat_map(|m| m.modified_files.iter().cloned())
        .collect();

    match strategy {
        VerificationStrategy::StructuralOnly => {
            run_structural_only(&all_modified, workspace_root, observer).await
        }
        VerificationStrategy::DiffReview {
            review_tiers,
            batch_strategy,
        } => {
            run_diff_review_only(
                review_tiers,
                batch_strategy,
                units,
                workspace_root,
                &all_modified,
                observer,
            )
            .await
        }
        VerificationStrategy::TestSuite { command, targeted } => {
            run_test_suite_only(command, *targeted, &all_modified, workspace_root, observer).await
        }
        VerificationStrategy::Combined {
            review_tiers,
            test_command,
        } => {
            run_combined(
                review_tiers,
                test_command.as_deref(),
                manifests,
                units,
                workspace_root,
                &all_modified,
                observer,
            )
            .await
        }
    }
}

async fn run_structural_only(
    modified_files: &[PathBuf],
    workspace_root: &Path,
    observer: &dyn SwarmObserver,
) -> Result<CombinedReport> {
    observer.on_verification_started("structural_only");
    let step = super::VerificationStep::Structural;
    observer.on_verification_step_started(&step);

    let report = structural::verify(modified_files, workspace_root);
    let passed = report.failures.is_empty();

    observer.on_verification_step_complete(&step, passed);
    observer.on_verification_complete(passed);

    Ok(CombinedReport {
        structural: report,
        diff_review: None,
        test_suite: None,
        overall_passed: passed,
        escalations_performed: Vec::new(),
        cost_estimate: CostEstimate::default(),
    })
}

async fn run_diff_review_only(
    review_tiers: &[ModelTier],
    batch_strategy: &super::BatchStrategy,
    units: &[WorkUnit],
    workspace_root: &Path,
    modified_files: &[PathBuf],
    observer: &dyn SwarmObserver,
) -> Result<CombinedReport> {
    observer.on_verification_started("diff_review");

    // Structural first (always)
    let structural_step = super::VerificationStep::Structural;
    observer.on_verification_step_started(&structural_step);
    let structural_report = structural::verify(modified_files, workspace_root);
    let structural_passed = structural_report.failures.is_empty();
    observer.on_verification_step_complete(&structural_step, structural_passed);

    if !structural_passed {
        observer.on_verification_complete(false);
        return Ok(CombinedReport {
            structural: structural_report,
            diff_review: None,
            test_suite: None,
            overall_passed: false,
            escalations_performed: Vec::new(),
            cost_estimate: CostEstimate::default(),
        });
    }

    // Diff review — build ReviewableDiffs from units
    // For now, we create empty diffs as a placeholder since actual diff computation
    // requires git2 merge-base which will be integrated in the pipeline wiring.
    let diffs: Vec<ReviewableDiff> = units
        .iter()
        .filter(|u| review_tiers.contains(&u.tier))
        .map(|u| ReviewableDiff {
            unit_id: u.id.clone(),
            tier: u.tier,
            coordinator_instructions: u.coordinator_instructions.clone(),
            file_diffs: Vec::new(), // Populated by pipeline when wired
        })
        .collect();

    let review_step = super::VerificationStep::DiffReview {
        units_to_review: diffs.len(),
    };
    observer.on_verification_step_started(&review_step);

    let mut reviewer = DiffReviewer::new(DiffReviewConfig {
        review_tiers: review_tiers.to_vec(),
        batch_strategy: batch_strategy.clone(),
        ..Default::default()
    });
    let review_report = reviewer.review(&diffs, "", workspace_root).await?;
    let review_passed = review_report.aggregate_approved;

    observer.on_verification_step_complete(&review_step, review_passed);
    observer.on_verification_complete(review_passed);

    Ok(CombinedReport {
        structural: structural_report,
        diff_review: Some(review_report),
        test_suite: None,
        overall_passed: review_passed,
        escalations_performed: Vec::new(),
        cost_estimate: CostEstimate::default(),
    })
}

async fn run_test_suite_only(
    command: &str,
    targeted: bool,
    modified_files: &[PathBuf],
    workspace_root: &Path,
    observer: &dyn SwarmObserver,
) -> Result<CombinedReport> {
    observer.on_verification_started("test_suite");

    // Structural first
    let structural_step = super::VerificationStep::Structural;
    observer.on_verification_step_started(&structural_step);
    let structural_report = structural::verify(modified_files, workspace_root);
    let structural_passed = structural_report.failures.is_empty();
    observer.on_verification_step_complete(&structural_step, structural_passed);

    if !structural_passed {
        observer.on_verification_complete(false);
        return Ok(CombinedReport {
            structural: structural_report,
            diff_review: None,
            test_suite: None,
            overall_passed: false,
            escalations_performed: Vec::new(),
            cost_estimate: CostEstimate::default(),
        });
    }

    // Test suite
    let test_step = super::VerificationStep::TestSuite {
        command: command.into(),
        targeted,
    };
    observer.on_verification_step_started(&test_step);

    let config = TestRunnerConfig {
        command: Some(command.into()),
        targeted,
        ..Default::default()
    };
    let test_report = test_runner::run(&config, modified_files, workspace_root).await?;
    let test_passed = test_report.passed;

    observer.on_verification_step_complete(&test_step, test_passed);
    observer.on_verification_complete(test_passed);

    Ok(CombinedReport {
        structural: structural_report,
        diff_review: None,
        test_suite: Some(test_report),
        overall_passed: test_passed,
        escalations_performed: Vec::new(),
        cost_estimate: CostEstimate::default(),
    })
}

/// Run all three verification strategies in sequence with early termination.
async fn run_combined(
    review_tiers: &[ModelTier],
    test_command: Option<&str>,
    manifests: &[AgentManifest],
    units: &[WorkUnit],
    workspace_root: &Path,
    modified_files: &[PathBuf],
    observer: &dyn SwarmObserver,
) -> Result<CombinedReport> {
    observer.on_verification_started("combined");
    let mut escalations = Vec::new();

    // Step 1: STRUCTURAL (always runs, cheapest)
    let structural_step = super::VerificationStep::Structural;
    observer.on_verification_step_started(&structural_step);
    let structural_report = structural::verify(modified_files, workspace_root);
    let structural_passed = structural_report
        .failures
        .iter()
        .all(|f| matches!(f.severity, FailureSeverity::MissingNode));
    observer.on_verification_step_complete(&structural_step, structural_passed);

    if !structural_passed {
        // Identify failing units for potential escalation
        for failure in &structural_report.failures {
            if matches!(
                failure.severity,
                FailureSeverity::ParseError | FailureSeverity::MissingSymbol { .. }
            ) {
                if let Some(unit) = find_unit_for_file(units, manifests, &failure.path) {
                    escalations.push(EscalationRecord {
                        unit_id: unit.id.clone(),
                        reason: EscalationReason::StructuralParseError,
                        from_tier: unit.tier,
                        to_tier: unit.escalation_tier.unwrap_or(ModelTier::Expensive),
                        succeeded: false, // Not attempted in this pass
                    });
                }
            }
        }

        observer.on_verification_complete(false);
        return Ok(CombinedReport {
            structural: structural_report,
            diff_review: None,
            test_suite: None,
            overall_passed: false,
            escalations_performed: escalations,
            cost_estimate: CostEstimate::default(),
        });
    }

    // Step 2: DIFF REVIEW (if review_tiers is non-empty)
    let mut diff_review_report: Option<DiffReviewReport> = None;
    if !review_tiers.is_empty() {
        let diffs: Vec<ReviewableDiff> = units
            .iter()
            .filter(|u| review_tiers.contains(&u.tier))
            .map(|u| ReviewableDiff {
                unit_id: u.id.clone(),
                tier: u.tier,
                coordinator_instructions: u.coordinator_instructions.clone(),
                file_diffs: Vec::new(), // Populated by pipeline when wired
            })
            .collect();

        if !diffs.is_empty() {
            let review_step = super::VerificationStep::DiffReview {
                units_to_review: diffs.len(),
            };
            observer.on_verification_step_started(&review_step);

            let mut reviewer = DiffReviewer::new(DiffReviewConfig {
                review_tiers: review_tiers.to_vec(),
                ..Default::default()
            });
            let review = reviewer.review(&diffs, "", workspace_root).await?;
            let review_passed = review.aggregate_approved;
            observer.on_verification_step_complete(&review_step, review_passed);

            if !review_passed {
                // Record escalation opportunities for rejected units
                for unit_review in &review.reviews {
                    if !unit_review.approved {
                        let error_issues: Vec<_> = unit_review
                            .issues
                            .iter()
                            .filter(|i| i.severity == super::IssueSeverity::Error)
                            .cloned()
                            .collect();
                        if !error_issues.is_empty() {
                            escalations.push(EscalationRecord {
                                unit_id: unit_review.unit_id.clone(),
                                reason: EscalationReason::DiffReviewRejection {
                                    issues: error_issues,
                                },
                                from_tier: ModelTier::Cheap, // Approximate
                                to_tier: ModelTier::Expensive,
                                succeeded: false,
                            });
                        }
                    }
                }

                diff_review_report = Some(review);
                observer.on_verification_complete(false);
                return Ok(CombinedReport {
                    structural: structural_report,
                    diff_review: diff_review_report,
                    test_suite: None,
                    overall_passed: false,
                    escalations_performed: escalations,
                    cost_estimate: CostEstimate::default(),
                });
            }

            diff_review_report = Some(review);
        }
    }

    // Step 3: TEST SUITE (if test_command is Some)
    let mut test_report: Option<TestReport> = None;
    if let Some(cmd) = test_command {
        let test_step = super::VerificationStep::TestSuite {
            command: cmd.into(),
            targeted: true,
        };
        observer.on_verification_step_started(&test_step);

        let config = TestRunnerConfig {
            command: Some(cmd.into()),
            targeted: true,
            ..Default::default()
        };
        let report = test_runner::run(&config, modified_files, workspace_root).await?;
        let test_passed = report.passed;
        observer.on_verification_step_complete(&test_step, test_passed);

        if !test_passed {
            // Attribute test failures to units
            if let Some(ref parsed) = report.parsed_results {
                for failure in &parsed.failures {
                    let test_names = vec![failure.test_name.clone()];
                    escalations.push(EscalationRecord {
                        unit_id: "unknown".into(), // Attribution requires file matching
                        reason: EscalationReason::TestFailure { test_names },
                        from_tier: ModelTier::Cheap,
                        to_tier: ModelTier::Expensive,
                        succeeded: false,
                    });
                }
            }

            test_report = Some(report);
            observer.on_verification_complete(false);
            return Ok(CombinedReport {
                structural: structural_report,
                diff_review: diff_review_report,
                test_suite: test_report,
                overall_passed: false,
                escalations_performed: escalations,
                cost_estimate: CostEstimate::default(),
            });
        }

        test_report = Some(report);
    }

    // All steps passed
    observer.on_verification_complete(true);

    Ok(CombinedReport {
        structural: structural_report,
        diff_review: diff_review_report,
        test_suite: test_report,
        overall_passed: true,
        escalations_performed: escalations,
        cost_estimate: CostEstimate::default(),
    })
}

/// Find which WorkUnit is responsible for a given file path.
fn find_unit_for_file<'a>(
    units: &'a [WorkUnit],
    manifests: &[AgentManifest],
    file_path: &Path,
) -> Option<&'a WorkUnit> {
    let file_str = file_path.to_string_lossy();
    for manifest in manifests {
        if manifest.modified_files.iter().any(|f| f == file_path) {
            return units.iter().find(|u| u.id == manifest.work_unit_id);
        }
    }
    // Fallback: check owned_paths
    for unit in units {
        if unit.scope.is_owned(&file_str) {
            return Some(unit);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::models::{AgentManifest, AgentStatus};
    use crate::types::{FileScope, PrivacyLevel};
    use std::collections::HashMap;

    fn mock_observer() -> MockObserver {
        MockObserver::default()
    }

    #[derive(Default)]
    struct MockObserver {
        steps_started: std::sync::Mutex<Vec<String>>,
        steps_completed: std::sync::Mutex<Vec<(String, bool)>>,
    }

    impl crate::observer::SwarmObserver for MockObserver {
        fn on_phase_changed(&self, _: &str) {}
        fn on_agent_state_changed(&self, _: &str, _: &AgentStatus, _: &str) {}
        fn on_tier_started(&self, _: usize, _: usize) {}
        fn on_merge_conflict(&self, _: &str, _: &[String]) {}
        fn on_completed(&self, _: &crate::swarm::models::SwarmResult) {}
        fn on_verification_started(&self, strategy: &str) {
            self.steps_started.lock().unwrap().push(strategy.into());
        }
        fn on_verification_step_started(&self, step: &super::super::VerificationStep) {
            self.steps_started
                .lock()
                .unwrap()
                .push(format!("{:?}", step));
        }
        fn on_verification_step_complete(
            &self,
            step: &super::super::VerificationStep,
            passed: bool,
        ) {
            self.steps_completed
                .lock()
                .unwrap()
                .push((format!("{:?}", step), passed));
        }
        fn on_verification_complete(&self, _passed: bool) {}
    }

    fn make_unit(id: &str, tier: ModelTier) -> WorkUnit {
        WorkUnit {
            id: id.into(),
            description: format!("Task {}", id),
            scope: FileScope {
                owned_paths: vec![format!("src/{}.rs", id)],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            backend: Default::default(),
            model: None,
            effort: None,
            extra: Vec::new(),
            tier,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: Some(ModelTier::Expensive),
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
            extra_allowed_tools: vec![],
        }
    }

    #[tokio::test]
    async fn test_structural_only_passes_valid_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("good.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let obs = mock_observer();
        let report = run_structural_only(&[PathBuf::from("good.rs")], dir.path(), &obs)
            .await
            .unwrap();

        assert!(report.overall_passed);
        assert_eq!(report.structural.files_checked, 1);
        assert_eq!(report.structural.files_passed, 1);
    }

    #[tokio::test]
    async fn test_structural_only_fails_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.rs");
        std::fs::write(&file, "fn main( {\n").unwrap();

        let obs = mock_observer();
        let report = run_structural_only(&[PathBuf::from("bad.rs")], dir.path(), &obs)
            .await
            .unwrap();

        assert!(!report.overall_passed);
        assert!(!report.structural.failures.is_empty());
    }

    #[tokio::test]
    async fn test_combined_structural_fail_stops_early() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.rs");
        std::fs::write(&file, "fn broken( {\n").unwrap();

        let units = vec![make_unit("a", ModelTier::Cheap)];
        let manifests = vec![AgentManifest {
            work_unit_id: "a".into(),
            status: AgentStatus::Completed,
            modified_files: vec![PathBuf::from("bad.rs")],
            branch: None,
            summary: None,
            output: None,
            cost_usd: 0.0,
        }];

        let obs = mock_observer();
        let report = run_combined(
            &[ModelTier::Cheap],
            Some("cargo test"),
            &manifests,
            &units,
            dir.path(),
            &[PathBuf::from("bad.rs")],
            &obs,
        )
        .await
        .unwrap();

        assert!(!report.overall_passed);
        // Diff review and tests should not have run
        assert!(report.diff_review.is_none());
        assert!(report.test_suite.is_none());
    }

    #[tokio::test]
    async fn test_combined_all_pass() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("good.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let units = vec![make_unit("a", ModelTier::Cheap)];
        let manifests = vec![AgentManifest {
            work_unit_id: "a".into(),
            status: AgentStatus::Completed,
            modified_files: vec![PathBuf::from("good.rs")],
            branch: None,
            summary: None,
            output: None,
            cost_usd: 0.0,
        }];

        let obs = mock_observer();
        // No test command → only structural and diff review run
        let report = run_combined(
            &[],  // No review tiers
            None, // No test command
            &manifests,
            &units,
            dir.path(),
            &[PathBuf::from("good.rs")],
            &obs,
        )
        .await
        .unwrap();

        assert!(report.overall_passed);
    }

    #[test]
    fn test_find_unit_for_file_by_manifest() {
        let units = vec![make_unit("a", ModelTier::Cheap)];
        let manifests = vec![AgentManifest {
            work_unit_id: "a".into(),
            status: AgentStatus::Completed,
            modified_files: vec![PathBuf::from("src/a.rs")],
            branch: None,
            summary: None,
            output: None,
            cost_usd: 0.0,
        }];

        let found = find_unit_for_file(&units, &manifests, Path::new("src/a.rs"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "a");
    }

    #[test]
    fn test_find_unit_for_file_by_scope() {
        let units = vec![make_unit("a", ModelTier::Cheap)];
        let manifests = vec![];

        let found = find_unit_for_file(&units, &manifests, Path::new("src/a.rs"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "a");
    }
}
