//! Swarm-side writers: one per run, one per agent invocation.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::journal::{ClaimHandle, ClaimJournal, ClaimWriter, WriterKind};

/// What a front end hands the swarm pipeline (`SwarmConfig::claims`).
#[derive(Debug, Clone)]
pub struct SwarmClaimConfig {
    pub journal: ClaimJournal,
    /// The run's label ("gaviero-cli 'fix'", "SWARM 'fix'").
    pub label: String,
    /// Filter other writers' claimed files out of the dsh/agy reconcile
    /// (`turnReview.attribution`).
    pub consume: bool,
}

/// A running swarm's writers.
#[derive(Clone)]
pub struct RunClaims {
    /// Pipeline writes: merges, verdict files.
    pub run_writer: Arc<ClaimWriter>,
    pub label: String,
    pub consume: bool,
    next_agent: Arc<AtomicU64>,
}

impl std::fmt::Debug for RunClaims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunClaims")
            .field("run_writer", &self.run_writer.id())
            .field("label", &self.label)
            .field("consume", &self.consume)
            .finish()
    }
}

impl RunClaims {
    /// Register the run writer. `None` (with a warning) when the journal
    /// cannot be written: the run proceeds without publishing.
    pub fn start(cfg: &SwarmClaimConfig, run_id: &str, root: &Path) -> Option<Self> {
        let id = format!("swarm-{}-{run_id}", std::process::id());
        match cfg
            .journal
            .register(&id, WriterKind::SwarmRun, &cfg.label, vec![root.to_path_buf()])
        {
            Ok(run_writer) => Some(Self {
                run_writer,
                label: cfg.label.clone(),
                consume: cfg.consume,
                next_agent: Arc::new(AtomicU64::new(0)),
            }),
            Err(e) => {
                tracing::warn!(target: "write_claims", "swarm write claims unavailable: {e:#}");
                None
            }
        }
    }

    /// Register one agent invocation's writer.
    pub fn register_agent(&self, unit_id: &str, root: &Path) -> Option<ClaimHandle> {
        let n = self.next_agent.fetch_add(1, Ordering::SeqCst);
        let id = format!("{}-{unit_id}-{n}", self.run_writer.id());
        let label = format!("{} · {unit_id}", self.label);
        match self.run_writer.journal().register(
            &id,
            WriterKind::SwarmAgent,
            &label,
            vec![root.to_path_buf()],
        ) {
            Ok(writer) => Some(ClaimHandle {
                writer,
                consume: self.consume,
            }),
            Err(e) => {
                tracing::warn!(target: "write_claims", "agent write claims unavailable: {e:#}");
                None
            }
        }
    }
}
