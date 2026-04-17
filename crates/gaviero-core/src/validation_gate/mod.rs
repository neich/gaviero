//! Inline validation gates that run after each agent file write.
//!
//! Fast gates (tree-sitter) run after every write. Slow gates (cargo check)
//! run at checkpoints. On failure the error is fed back to the agent as a
//! corrective prompt and the agent retries up to `work_unit.max_retries`.
//!
//! # Module layout
//! - `ValidationGate` — trait implemented by each gate
//! - `ValidationPipeline` — runs gates in order, short-circuits on first fail
//! - `TreeSitterGate`  — fast syntax check via tree-sitter (delegates to verify::structural)
//! - `CargoCheckGate`  — slow semantic check via `cargo check`

pub mod cargo_gate;
pub mod tree_sitter_gate;

pub use cargo_gate::CargoCheckGate;
pub use tree_sitter_gate::TreeSitterGate;

use std::path::{Path, PathBuf};

use async_trait::async_trait;

// ── Core types ───────────────────────────────────────────────

/// Outcome of running a single validation gate.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// The file(s) are structurally valid.
    Pass,
    /// Validation found a problem. Feed `message` (and optionally `suggestion`)
    /// back to the agent as a corrective prompt.
    Fail {
        message: String,
        suggestion: Option<String>,
    },
    /// This gate does not apply to the given files (e.g. unknown extension).
    Skip,
}

impl ValidationResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass | Self::Skip)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Fail { message, .. } => Some(message),
            _ => None,
        }
    }
}

// ── Trait ────────────────────────────────────────────────────

/// A single validation step that can be applied to modified files.
#[async_trait]
pub trait ValidationGate: Send + Sync {
    /// Short name shown in observer events and corrective prompts.
    fn name(&self) -> &'static str;

    /// Fast gates run after every write. Slow gates run only at checkpoints
    /// (currently: after the entire agent turn completes, before retry).
    fn is_fast(&self) -> bool;

    /// Validate `files` within `workdir`. May read the filesystem.
    async fn validate(&self, files: &[PathBuf], workdir: &Path) -> ValidationResult;
}

// ── Pipeline ─────────────────────────────────────────────────

/// Runs a sequence of `ValidationGate`s in order.
///
/// Stops and returns the first `Fail` result. Returns `Pass` if all gates
/// pass (or skip). Gates are run in the order they were added.
pub struct ValidationPipeline {
    gates: Vec<Box<dyn ValidationGate>>,
}

impl ValidationPipeline {
    /// Default pipeline for Rust projects: tree-sitter (fast) + cargo check (slow).
    pub fn default_for_rust() -> Self {
        Self {
            gates: vec![Box::new(TreeSitterGate), Box::new(CargoCheckGate)],
        }
    }

    /// Fast-only pipeline: tree-sitter syntax check only.
    /// Use this for per-write checks during streaming.
    pub fn fast_only() -> Self {
        Self {
            gates: vec![Box::new(TreeSitterGate)],
        }
    }

    /// Run all gates in order, reporting each result through `on_gate`.
    ///
    /// `on_gate` receives `(gate_name, passed)` for every gate that runs.
    /// If `fast_only` is `true`, only fast gates run.
    /// Returns `Some((gate_name, result))` for the first `Fail`, or `None` if all pass/skip.
    pub async fn run_reporting<F>(
        &self,
        files: &[PathBuf],
        workdir: &Path,
        fast_only: bool,
        mut on_gate: F,
    ) -> Option<(&'static str, ValidationResult)>
    where
        F: FnMut(&'static str, bool),
    {
        for gate in &self.gates {
            if fast_only && !gate.is_fast() {
                continue;
            }
            let result = gate.validate(files, workdir).await;
            match &result {
                ValidationResult::Skip => {}
                ValidationResult::Pass => on_gate(gate.name(), true),
                ValidationResult::Fail { .. } => {
                    on_gate(gate.name(), false);
                    return Some((gate.name(), result));
                }
            }
        }
        None
    }

    /// Convenience: run all gates, return the first failure (gate_name, result), or None.
    pub async fn run(
        &self,
        files: &[PathBuf],
        workdir: &Path,
        fast_only: bool,
    ) -> Option<(&'static str, ValidationResult)> {
        self.run_reporting(files, workdir, fast_only, |_, _| {})
            .await
    }
}

// ── Corrective prompt ────────────────────────────────────────

/// Format a corrective prompt to feed back to the agent after a validation failure.
pub fn corrective_prompt(gate_name: &str, path: &Path, message: &str) -> String {
    format!(
        "Your previous edit to `{}` failed validation:\n\
         {}: {}\n\
         Please fix the issue.",
        path.display(),
        gate_name,
        message,
    )
}
