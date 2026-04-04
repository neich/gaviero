//! Cargo check gate.
//!
//! Runs `cargo check --message-format json` against the nearest `Cargo.toml`
//! above the agent's workdir. Slow: spawns a subprocess. Only runs at the end
//! of an agent turn (not after every write), controlled by `is_fast() = false`.

use std::path::{Path, PathBuf};
use std::process::Command;

use async_trait::async_trait;

use super::{ValidationGate, ValidationResult};

pub struct CargoCheckGate;

#[async_trait]
impl ValidationGate for CargoCheckGate {
    fn name(&self) -> &'static str {
        "cargo-check"
    }

    fn is_fast(&self) -> bool {
        false
    }

    async fn validate(&self, _files: &[PathBuf], workdir: &Path) -> ValidationResult {
        let Some(manifest) = find_cargo_toml(workdir) else {
            // No Cargo.toml found — skip (not a Rust project)
            return ValidationResult::Skip;
        };

        let output = Command::new("cargo")
            .args([
                "check",
                "--manifest-path",
                manifest.to_str().unwrap_or("Cargo.toml"),
                "--message-format",
                "short",
            ])
            .current_dir(workdir)
            .output();

        match output {
            Err(e) => {
                // cargo not found or subprocess error — skip rather than fail
                tracing::debug!("cargo check subprocess error: {}", e);
                ValidationResult::Skip
            }
            Ok(out) if out.status.success() => ValidationResult::Pass,
            Ok(out) => {
                // Collect error lines from stderr
                let stderr = String::from_utf8_lossy(&out.stderr);
                let errors: Vec<&str> = stderr
                    .lines()
                    .filter(|l| l.contains("error") || l.starts_with("error"))
                    .take(10) // cap at 10 lines to keep corrective prompts concise
                    .collect();

                let message = if errors.is_empty() {
                    "cargo check failed (see output)".to_string()
                } else {
                    errors.join("\n")
                };

                ValidationResult::Fail {
                    message,
                    suggestion: Some(
                        "Fix the compiler errors shown above. \
                         Check type mismatches, missing imports, and undefined symbols."
                            .into(),
                    ),
                }
            }
        }
    }
}

/// Walk up the directory tree from `dir` to find the nearest `Cargo.toml`.
fn find_cargo_toml(dir: &Path) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}
