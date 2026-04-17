//! Verification pipeline types and strategies.
//!
//! The coordinator selects a strategy during planning. The strategy
//! determines which verification steps run in Phase 4 of the pipeline.

pub mod combined;
pub mod diff_review;
pub mod structural;
pub mod test_runner;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::types::ModelTier;

// ── Strategies ──────────────────────────────────────────────────

/// Verification strategy selected by the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerificationStrategy {
    /// Tree-sitter structural validation only.
    StructuralOnly,
    /// Sonnet reviews diffs from specified tiers.
    DiffReview {
        review_tiers: Vec<ModelTier>,
        batch_strategy: BatchStrategy,
    },
    /// Run test suite after merge.
    TestSuite { command: String, targeted: bool },
    /// All three strategies in sequence with early termination.
    Combined {
        #[serde(default)]
        review_tiers: Vec<ModelTier>,
        #[serde(default)]
        test_command: Option<String>,
    },
}

impl Default for VerificationStrategy {
    fn default() -> Self {
        Self::Combined {
            review_tiers: vec![ModelTier::Cheap],
            test_command: None,
        }
    }
}

/// How to batch diffs for the LLM diff reviewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStrategy {
    /// One Sonnet call per WorkUnit.
    PerUnit,
    /// Group units sharing a dependency tier into one review call.
    PerDependencyTier,
    /// Single review call for all diffs.
    Aggregate,
}

impl Default for BatchStrategy {
    fn default() -> Self {
        Self::PerDependencyTier
    }
}

/// Identifies which verification step is running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationStep {
    Structural,
    DiffReview { units_to_review: usize },
    TestSuite { command: String, targeted: bool },
}

// ── Reports ─────────────────────────────────────────────────────

/// Aggregate report from the Combined verification strategy.
#[derive(Debug, Clone)]
pub struct CombinedReport {
    pub structural: StructuralReport,
    pub diff_review: Option<DiffReviewReport>,
    pub test_suite: Option<TestReport>,
    pub overall_passed: bool,
    pub escalations_performed: Vec<EscalationRecord>,
    pub cost_estimate: CostEstimate,
}

/// Report from structural (tree-sitter) verification.
#[derive(Debug, Clone)]
pub struct StructuralReport {
    pub files_checked: usize,
    pub files_passed: usize,
    pub failures: Vec<StructuralFailure>,
}

/// A structural verification failure for a single file.
#[derive(Debug, Clone)]
pub struct StructuralFailure {
    pub path: PathBuf,
    pub language: String,
    pub error_nodes: Vec<ErrorNode>,
    pub severity: FailureSeverity,
}

/// Location and context of a tree-sitter error node.
#[derive(Debug, Clone)]
pub struct ErrorNode {
    pub line: usize,
    pub column: usize,
    pub byte_range: std::ops::Range<usize>,
    pub parent_symbol: Option<String>,
    pub context_snippet: String,
}

/// Severity of a structural failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureSeverity {
    /// Tree-sitter ERROR node — definite parse failure.
    ParseError,
    /// MISSING node — tree-sitter recovered but something is absent.
    MissingNode,
    /// Symbol referenced in coordinator instructions but absent in final AST.
    MissingSymbol { expected: String },
}

/// Report from the LLM diff reviewer.
#[derive(Debug, Clone)]
pub struct DiffReviewReport {
    pub reviews: Vec<UnitReview>,
    pub aggregate_approved: bool,
}

/// Review result for a single work unit.
#[derive(Debug, Clone)]
pub struct UnitReview {
    pub unit_id: String,
    pub approved: bool,
    pub issues: Vec<ReviewIssue>,
}

/// An issue found during diff review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub severity: IssueSeverity,
    pub file: PathBuf,
    pub line_range: Option<(usize, usize)>,
    pub description: String,
    pub suggested_fix: Option<String>,
}

/// Severity of a review issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// Blocks approval — must be fixed.
    Error,
    /// Flagged but doesn't block.
    Warning,
}

/// Report from the test runner.
#[derive(Debug, Clone)]
pub struct TestReport {
    pub exit_code: i32,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub targeted_filter: Option<String>,
    pub parsed_results: Option<ParsedTestResults>,
}

/// Parsed test output (best-effort).
#[derive(Debug, Clone)]
pub struct ParsedTestResults {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub failures: Vec<TestFailure>,
}

/// A single test failure.
#[derive(Debug, Clone)]
pub struct TestFailure {
    pub test_name: String,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}

// ── Escalation ──────────────────────────────────────────────────

/// Record of an escalation event during verification.
#[derive(Debug, Clone)]
pub struct EscalationRecord {
    pub unit_id: String,
    pub reason: EscalationReason,
    pub from_tier: ModelTier,
    pub to_tier: ModelTier,
    pub succeeded: bool,
}

/// Why escalation was triggered.
#[derive(Debug, Clone)]
pub enum EscalationReason {
    StructuralParseError,
    DiffReviewRejection {
        issues: Vec<ReviewIssue>,
    },
    TestFailure {
        test_names: Vec<String>,
    },
    /// Agent failed during execution (timeout, backend error, panic).
    AgentFailure {
        reason: String,
    },
}

// ── Cost ────────────────────────────────────────────────────────

/// Estimated cost breakdown for a swarm run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostEstimate {
    pub coordinator_tokens: u64,
    pub reasoning_tokens: u64,
    pub execution_tokens: u64,
    pub mechanical_tokens: u64,
    pub review_tokens: u64,
    pub estimated_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_strategy_serde() {
        let strategy = VerificationStrategy::Combined {
            review_tiers: vec![ModelTier::Cheap],
            test_command: Some("cargo test".into()),
        };
        let json = serde_json::to_string(&strategy).unwrap();
        let back: VerificationStrategy = serde_json::from_str(&json).unwrap();
        match back {
            VerificationStrategy::Combined {
                review_tiers,
                test_command,
            } => {
                assert_eq!(review_tiers, vec![ModelTier::Cheap]);
                assert_eq!(test_command.as_deref(), Some("cargo test"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_structural_only_serde() {
        let s = VerificationStrategy::StructuralOnly;
        let json = serde_json::to_string(&s).unwrap();
        let back: VerificationStrategy = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, VerificationStrategy::StructuralOnly));
    }

    #[test]
    fn test_batch_strategy_serde() {
        let bs = BatchStrategy::PerUnit;
        let json = serde_json::to_string(&bs).unwrap();
        assert_eq!(json, "\"per_unit\"");
    }

    #[test]
    fn test_cost_estimate_default() {
        let c = CostEstimate::default();
        assert_eq!(c.estimated_usd, 0.0);
        assert_eq!(c.coordinator_tokens, 0);
    }
}
