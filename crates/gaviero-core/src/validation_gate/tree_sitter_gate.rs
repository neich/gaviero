//! Tree-sitter syntax gate.
//!
//! Delegates entirely to `swarm::verify::structural::verify()` — no duplicated
//! tree-sitter logic. Fast: zero LLM calls, zero subprocess spawns.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::{ValidationGate, ValidationResult};
use crate::swarm::verify::structural;

pub struct TreeSitterGate;

#[async_trait]
impl ValidationGate for TreeSitterGate {
    fn name(&self) -> &'static str {
        "tree-sitter"
    }

    fn is_fast(&self) -> bool {
        true
    }

    async fn validate(&self, files: &[PathBuf], workdir: &Path) -> ValidationResult {
        let report = structural::verify(files, workdir);

        if report.failures.is_empty() {
            return ValidationResult::Pass;
        }

        // Format all failures into a single diagnostic message
        let messages: Vec<String> = report
            .failures
            .iter()
            .map(|f| {
                let errors: Vec<String> = f
                    .error_nodes
                    .iter()
                    .map(|e| {
                        format!(
                            "  line {}: {}",
                            e.line + 1,
                            if e.context_snippet.is_empty() {
                                "syntax error".to_string()
                            } else {
                                e.context_snippet.clone()
                            }
                        )
                    })
                    .collect();
                format!("{}: {}", f.path.display(), errors.join("\n"))
            })
            .collect();

        ValidationResult::Fail {
            message: messages.join("\n"),
            suggestion: Some(
                "Fix the syntax errors shown above. \
                 Make sure all brackets, braces, and parentheses are balanced."
                    .into(),
            ),
        }
    }
}
