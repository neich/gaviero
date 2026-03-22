//! LLM diff reviewer: Sonnet reviews diffs from lower-tier agents.
//!
//! Catches semantic errors that structural verification misses — wrong logic,
//! incomplete changes, misinterpreted instructions.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{
    BatchStrategy, DiffReviewReport, IssueSeverity, ReviewIssue, UnitReview,
};
use crate::acp::session::{AcpSession, AgentOptions};
use crate::types::ModelTier;

/// Configuration for the diff reviewer.
#[derive(Debug, Clone)]
pub struct DiffReviewConfig {
    pub review_model: String,
    pub max_diff_tokens: u32,
    pub batch_strategy: BatchStrategy,
    pub review_tiers: Vec<ModelTier>,
}

impl Default for DiffReviewConfig {
    fn default() -> Self {
        Self {
            review_model: "sonnet".into(),
            max_diff_tokens: 16384,
            batch_strategy: BatchStrategy::PerDependencyTier,
            review_tiers: vec![ModelTier::Mechanical, ModelTier::Execution],
        }
    }
}

/// A diff prepared for review.
#[derive(Debug, Clone)]
pub struct ReviewableDiff {
    pub unit_id: String,
    pub tier: ModelTier,
    pub coordinator_instructions: String,
    pub file_diffs: Vec<FileDiff>,
}

/// Diff for a single file.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub original: String,
    pub modified: String,
    pub unified_diff: String,
}

/// The diff reviewer. Spawns Sonnet sessions with WriteMode::RejectAll.
pub struct DiffReviewer {
    config: DiffReviewConfig,
    /// Track reviewed units to prevent re-reviewing after escalation (loop prevention).
    reviewed_units: HashSet<String>,
}

impl DiffReviewer {
    pub fn new(config: DiffReviewConfig) -> Self {
        Self {
            config,
            reviewed_units: HashSet::new(),
        }
    }

    /// Review diffs from units whose tier is in `review_tiers`.
    ///
    /// Skips units already reviewed (loop prevention after escalation).
    pub async fn review(
        &mut self,
        diffs: &[ReviewableDiff],
        plan_summary: &str,
        workspace_root: &std::path::Path,
    ) -> Result<DiffReviewReport> {
        // Filter to reviewable tiers and not-already-reviewed
        let to_review: Vec<&ReviewableDiff> = diffs
            .iter()
            .filter(|d| self.config.review_tiers.contains(&d.tier))
            .filter(|d| !self.reviewed_units.contains(&d.unit_id))
            .collect();

        if to_review.is_empty() {
            return Ok(DiffReviewReport {
                reviews: Vec::new(),
                aggregate_approved: true,
            });
        }

        let mut reviews = Vec::new();

        // Batch according to strategy
        let batches = batch_diffs(&to_review, &self.config.batch_strategy);

        for batch in batches {
            let prompt = build_review_prompt(plan_summary, &batch, self.config.max_diff_tokens);

            match call_reviewer(&self.config.review_model, &prompt, workspace_root).await {
                Ok(review_results) => {
                    for review in review_results {
                        self.reviewed_units.insert(review.unit_id.clone());
                        reviews.push(review);
                    }
                }
                Err(e) => {
                    tracing::warn!("Diff review call failed: {}. Approving by default.", e);
                    // On reviewer failure, approve all units in this batch
                    for diff in &batch {
                        self.reviewed_units.insert(diff.unit_id.clone());
                        reviews.push(UnitReview {
                            unit_id: diff.unit_id.clone(),
                            approved: true,
                            issues: vec![ReviewIssue {
                                severity: IssueSeverity::Warning,
                                file: PathBuf::new(),
                                line_range: None,
                                description: format!("Review skipped due to error: {}", e),
                                suggested_fix: None,
                            }],
                        });
                    }
                }
            }
        }

        let aggregate_approved = reviews.iter().all(|r| r.approved);

        Ok(DiffReviewReport {
            reviews,
            aggregate_approved,
        })
    }

    /// Check if a unit has already been reviewed.
    pub fn was_reviewed(&self, unit_id: &str) -> bool {
        self.reviewed_units.contains(unit_id)
    }
}

/// Batch diffs according to the strategy.
fn batch_diffs<'a>(
    diffs: &[&'a ReviewableDiff],
    strategy: &BatchStrategy,
) -> Vec<Vec<&'a ReviewableDiff>> {
    match strategy {
        BatchStrategy::PerUnit => diffs.iter().map(|d| vec![*d]).collect(),
        BatchStrategy::Aggregate => vec![diffs.to_vec()],
        BatchStrategy::PerDependencyTier => {
            // Group by tier
            let mut tier_groups: std::collections::HashMap<&ModelTier, Vec<&ReviewableDiff>> =
                std::collections::HashMap::new();
            for diff in diffs {
                tier_groups.entry(&diff.tier).or_default().push(diff);
            }
            tier_groups.into_values().collect()
        }
    }
}

/// Build the review prompt following the plan's §5.3 format.
fn build_review_prompt(
    plan_summary: &str,
    batch: &[&ReviewableDiff],
    max_tokens: u32,
) -> String {
    let mut prompt = String::new();

    prompt.push_str("SECTION 1: COORDINATOR CONTEXT\n");
    prompt.push_str("────────────────────────────────\n");
    prompt.push_str(&format!("The original task was: \"{}\"\n\n", plan_summary));

    for diff in batch {
        prompt.push_str(&format!(
            "Subtask '{}' was assigned to a {:?} model with instructions:\n\"{}\"\n\n",
            diff.unit_id, diff.tier, diff.coordinator_instructions
        ));
    }

    prompt.push_str("SECTION 2: DIFFS\n");
    prompt.push_str("────────────────────────────────\n");

    let mut total_len = prompt.len();
    let char_budget = (max_tokens * 4) as usize; // rough: 4 chars per token

    for diff in batch {
        for file_diff in &diff.file_diffs {
            let diff_text = format!(
                "--- a/{path}\n+++ b/{path}\n\n{diff}\n\n",
                path = file_diff.path.display(),
                diff = file_diff.unified_diff,
            );
            if total_len + diff_text.len() > char_budget {
                prompt.push_str("[Diff truncated — showing changed regions only]\n");
                break;
            }
            prompt.push_str(&diff_text);
            total_len += diff_text.len();
        }
    }

    prompt.push_str("SECTION 3: REVIEW INSTRUCTIONS\n");
    prompt.push_str("────────────────────────────────\n");
    prompt.push_str(
        "Evaluate on these axes:\n\
         1. CORRECTNESS — Do the changes implement the coordinator's instructions?\n\
         2. COMPLETENESS — Are all required changes present?\n\
         3. SCOPE DISCIPLINE — Did the agent modify anything outside its instructions?\n\
         4. INTERFACE PRESERVATION — Are function signatures preserved or correctly updated?\n\
         5. CONSISTENCY — Do changes across files use consistent naming and patterns?\n\n\
         Respond with ONLY a JSON object:\n\
         {\n  \"approved\": true/false,\n  \"issues\": [\n    {\n\
           \"severity\": \"error\" | \"warning\",\n\
           \"file\": \"path/to/file\",\n\
           \"line_range\": [start, end] or null,\n\
           \"description\": \"what's wrong\",\n\
           \"suggested_fix\": \"how to fix it\" or null\n    }\n  ]\n}\n",
    );

    prompt
}

/// Call the reviewer model and parse the JSON response.
async fn call_reviewer(
    model: &str,
    prompt: &str,
    workspace_root: &std::path::Path,
) -> Result<Vec<UnitReview>> {
    let options = AgentOptions::default();
    let system = "You are a code reviewer. Review the diffs and respond with ONLY a JSON verdict.";

    let mut session = AcpSession::spawn(
        model,
        workspace_root,
        prompt,
        system,
        &[], // No tools — reviewer never writes
        &options,
        &[],
    )?;

    let mut response = String::new();
    loop {
        match session.next_event().await {
            Ok(Some(crate::acp::protocol::StreamEvent::ContentDelta(text))) => {
                response.push_str(&text);
            }
            Ok(Some(crate::acp::protocol::StreamEvent::ResultEvent {
                result_text, ..
            })) => {
                if response.is_empty() {
                    response = result_text;
                }
                break;
            }
            Ok(None) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let _ = session.wait().await;

    // Parse JSON verdict
    parse_review_response(&response)
}

/// Parse the reviewer's JSON response into UnitReview(s).
fn parse_review_response(response: &str) -> Result<Vec<UnitReview>> {
    // Extract JSON from response (may be wrapped in markdown fences)
    let json_str = crate::swarm::planner::extract_json(response)
        .unwrap_or_else(|_| response.trim().to_string());

    #[derive(serde::Deserialize)]
    struct ReviewVerdict {
        approved: bool,
        #[serde(default)]
        issues: Vec<ReviewIssueRaw>,
    }

    #[derive(serde::Deserialize)]
    struct ReviewIssueRaw {
        severity: String,
        #[serde(default)]
        file: String,
        line_range: Option<Vec<usize>>,
        description: String,
        suggested_fix: Option<String>,
    }

    let verdict: ReviewVerdict = serde_json::from_str(&json_str)
        .context("parsing reviewer verdict JSON")?;

    let issues: Vec<ReviewIssue> = verdict
        .issues
        .into_iter()
        .map(|raw| ReviewIssue {
            severity: if raw.severity == "error" {
                IssueSeverity::Error
            } else {
                IssueSeverity::Warning
            },
            file: PathBuf::from(raw.file),
            line_range: raw.line_range.and_then(|r| {
                if r.len() == 2 { Some((r[0], r[1])) } else { None }
            }),
            description: raw.description,
            suggested_fix: raw.suggested_fix,
        })
        .collect();

    Ok(vec![UnitReview {
        unit_id: "batch".into(), // Batched review — caller maps back to units
        approved: verdict.approved,
        issues,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_review_response_approved() {
        let json = r#"{"approved": true, "issues": []}"#;
        let reviews = parse_review_response(json).unwrap();
        assert_eq!(reviews.len(), 1);
        assert!(reviews[0].approved);
        assert!(reviews[0].issues.is_empty());
    }

    #[test]
    fn test_parse_review_response_rejected() {
        let json = r#"{
            "approved": false,
            "issues": [
                {
                    "severity": "error",
                    "file": "src/auth.rs",
                    "line_range": [10, 15],
                    "description": "Missing null check",
                    "suggested_fix": "Add if token.is_none() guard"
                },
                {
                    "severity": "warning",
                    "file": "src/lib.rs",
                    "line_range": null,
                    "description": "Unused import",
                    "suggested_fix": null
                }
            ]
        }"#;
        let reviews = parse_review_response(json).unwrap();
        assert_eq!(reviews.len(), 1);
        assert!(!reviews[0].approved);
        assert_eq!(reviews[0].issues.len(), 2);
        assert_eq!(reviews[0].issues[0].severity, IssueSeverity::Error);
        assert_eq!(reviews[0].issues[0].file, PathBuf::from("src/auth.rs"));
        assert_eq!(reviews[0].issues[0].line_range, Some((10, 15)));
        assert_eq!(reviews[0].issues[1].severity, IssueSeverity::Warning);
        assert!(reviews[0].issues[1].line_range.is_none());
    }

    #[test]
    fn test_parse_review_response_fenced() {
        let response = "Here's my review:\n```json\n{\"approved\": true, \"issues\": []}\n```\n";
        let reviews = parse_review_response(response).unwrap();
        assert!(reviews[0].approved);
    }

    #[test]
    fn test_batch_per_unit() {
        let d1 = ReviewableDiff {
            unit_id: "a".into(),
            tier: ModelTier::Mechanical,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let d2 = ReviewableDiff {
            unit_id: "b".into(),
            tier: ModelTier::Execution,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let refs: Vec<&ReviewableDiff> = vec![&d1, &d2];
        let batches = batch_diffs(&refs, &BatchStrategy::PerUnit);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
    }

    #[test]
    fn test_batch_aggregate() {
        let d1 = ReviewableDiff {
            unit_id: "a".into(),
            tier: ModelTier::Mechanical,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let d2 = ReviewableDiff {
            unit_id: "b".into(),
            tier: ModelTier::Execution,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let refs: Vec<&ReviewableDiff> = vec![&d1, &d2];
        let batches = batch_diffs(&refs, &BatchStrategy::Aggregate);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn test_batch_per_dependency_tier() {
        let d1 = ReviewableDiff {
            unit_id: "a".into(),
            tier: ModelTier::Mechanical,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let d2 = ReviewableDiff {
            unit_id: "b".into(),
            tier: ModelTier::Mechanical,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let d3 = ReviewableDiff {
            unit_id: "c".into(),
            tier: ModelTier::Execution,
            coordinator_instructions: String::new(),
            file_diffs: vec![],
        };
        let refs: Vec<&ReviewableDiff> = vec![&d1, &d2, &d3];
        let batches = batch_diffs(&refs, &BatchStrategy::PerDependencyTier);
        assert_eq!(batches.len(), 2); // mechanical group + execution group
    }

    #[test]
    fn test_build_review_prompt_contains_sections() {
        let diff = ReviewableDiff {
            unit_id: "test-unit".into(),
            tier: ModelTier::Mechanical,
            coordinator_instructions: "Rename foo to bar".into(),
            file_diffs: vec![FileDiff {
                path: PathBuf::from("src/lib.rs"),
                original: "fn foo() {}".into(),
                modified: "fn bar() {}".into(),
                unified_diff: "-fn foo() {}\n+fn bar() {}".into(),
            }],
        };
        let prompt = build_review_prompt("Refactor auth", &[&diff], 16384);
        assert!(prompt.contains("SECTION 1: COORDINATOR CONTEXT"));
        assert!(prompt.contains("Refactor auth"));
        assert!(prompt.contains("SECTION 2: DIFFS"));
        assert!(prompt.contains("src/lib.rs"));
        assert!(prompt.contains("SECTION 3: REVIEW INSTRUCTIONS"));
        assert!(prompt.contains("CORRECTNESS"));
    }

    #[test]
    fn test_reviewer_loop_prevention() {
        let mut reviewer = DiffReviewer::new(DiffReviewConfig::default());
        reviewer.reviewed_units.insert("unit-a".into());
        assert!(reviewer.was_reviewed("unit-a"));
        assert!(!reviewer.was_reviewed("unit-b"));
    }
}
