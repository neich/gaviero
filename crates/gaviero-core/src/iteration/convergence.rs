//! Convergence detection for the iteration engine.
//!
//! Tracks whether successive agent attempts are making progress by comparing
//! the number of modified files. If the count stops changing for `STALL_LIMIT`
//! consecutive iterations, the agent is considered stuck and the outer loop
//! should stop retrying.

use std::path::PathBuf;

/// Number of consecutive non-improving iterations before declaring stall.
const STALL_LIMIT: u32 = 2;

/// Detects whether an agent has stalled (no progress across iterations).
#[derive(Debug, Default)]
pub struct ConvergenceDetector {
    last_count: usize,
    stall_count: u32,
}

impl ConvergenceDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the files modified in the latest attempt.
    ///
    /// Returns `true` if the agent appears stuck (no improvement for
    /// `STALL_LIMIT` iterations), `false` if still making progress.
    pub fn record(&mut self, modified_files: &[PathBuf]) -> bool {
        let count = modified_files.len();
        if count <= self.last_count && self.last_count > 0 {
            self.stall_count += 1;
        } else {
            self.stall_count = 0;
        }
        self.last_count = count;
        self.stall_count >= STALL_LIMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("file{}.rs", i))).collect()
    }

    #[test]
    fn no_stall_when_growing() {
        let mut d = ConvergenceDetector::new();
        assert!(!d.record(&paths(1)));
        assert!(!d.record(&paths(2)));
        assert!(!d.record(&paths(3)));
    }

    #[test]
    fn stall_after_two_flat_iterations() {
        let mut d = ConvergenceDetector::new();
        d.record(&paths(2));
        assert!(!d.record(&paths(2))); // first flat — stall_count = 1
        assert!(d.record(&paths(2)));  // second flat — stall_count = 2 → stalled
    }

    #[test]
    fn recovery_resets_stall() {
        let mut d = ConvergenceDetector::new();
        d.record(&paths(2));
        d.record(&paths(2)); // stall_count = 1
        d.record(&paths(3)); // recovery — stall_count resets to 0
        assert!(!d.record(&paths(3))); // stall_count = 1 again, not yet stalled
    }

    #[test]
    fn first_call_never_stalls() {
        let mut d = ConvergenceDetector::new();
        assert!(!d.record(&paths(0)));
        assert!(!d.record(&paths(0)));
    }
}
