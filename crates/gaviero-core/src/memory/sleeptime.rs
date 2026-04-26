//! Tier B / B5: sleeptime pass.
//!
//! Bulk hygiene that doesn't fit in the per-turn or per-session
//! latency budgets:
//!
//! 1. **Decay sweep** — recompute current recency under the B4 floor +
//!    exemptions; flag rows that are unretrievably-low for user review
//!    (never auto-delete).
//! 2. **Near-duplicate merge** — within a scope level, find
//!    cosine ≥ 0.92 same-type pairs and merge them. **Source-aware**:
//!    `user_remember` is ground truth and never silently merged
//!    *into*.
//! 3. **Cross-scope promotion** — runs the existing consolidator's
//!    "3+ module hits → repo" lift; lowers the threshold to 1 hit for
//!    `decision|convention|invariant` types.
//! 4. **Trust re-scoring** — uses B6 `retrieval_use` rates when
//!    available; falls back to raw injection counts (manifest hits)
//!    until B6 has produced enough rows.
//! 5. **KG node-doc refresh** — Tier D1 stub (no-op).
//!
//! Every operation is logged into `sleeptime_audit` so the Tier C2
//! `/forget` audit trail can reverse them by hand. `dry_run = true`
//! short-circuits all writes but still produces audit rows for review.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::scope::MemoryType;
use super::store::MemoryStore;
use super::trust_defaults::MemorySource;

/// Settings snapshot for one sleeptime invocation.
#[derive(Debug, Clone)]
pub struct SleeptimeConfig {
    pub enabled: bool,
    pub min_idle_minutes: usize,
    pub weekly_force_run: bool,
    pub near_dup_threshold: f32,
    pub first_run_require_confirm: bool,
    /// When true, all destructive ops are skipped; audit rows are
    /// still produced (with `dry_run = 1`) for review.
    pub dry_run: bool,
    /// Weighting hooks for trust re-scoring (B5 step 4 / B6).
    pub trust_min_injections: u32,
    pub trust_adjust_delta: f32,
    pub utilization_used_threshold: f32,
    pub utilization_unused_threshold: f32,
    pub trust_floor: f32,
    pub trust_ceiling_llm: f32,
}

impl Default for SleeptimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_idle_minutes: 10,
            weekly_force_run: true,
            near_dup_threshold: 0.92,
            first_run_require_confirm: true,
            dry_run: false,
            trust_min_injections: 5,
            trust_adjust_delta: 0.05,
            utilization_used_threshold: 0.6,
            utilization_unused_threshold: 0.1,
            trust_floor: 0.2,
            trust_ceiling_llm: 0.9,
        }
    }
}

/// Aggregate counts emitted at the end of a sleeptime pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SleeptimeReport {
    pub run_id: String,
    pub dry_run: bool,
    pub decay_flagged: usize,
    pub near_dup_merged: usize,
    pub promoted: usize,
    pub trust_adjusted: usize,
    pub telemetry_pruned: usize,
    pub kg_doc_refreshed: usize,
}

/// Per-operation observer. Implementations forward to the TUI panel
/// (Tier A4) for live display + persist into `sleeptime_audit` for the
/// audit trail.
pub trait SleeptimeObserver: Send + Sync {
    fn on_operation(&self, op: &SleeptimeOperation);
    fn on_complete(&self, report: &SleeptimeReport);
}

/// Stable description of a single sleeptime operation. Serialised into
/// the audit log; the panel renders one line per operation.
#[derive(Debug, Clone, PartialEq)]
pub enum SleeptimeOperation {
    DecayFlagged {
        memory_id: i64,
        recency: f32,
    },
    NearDupMerged {
        keep_id: i64,
        drop_id: i64,
        cosine: f32,
        keep_source: MemorySource,
        drop_source: MemorySource,
    },
    Promoted {
        memory_id: i64,
        from_scope_level: i32,
        to_scope_level: i32,
        memory_type: MemoryType,
    },
    TrustAdjusted {
        memory_id: i64,
        old_trust: f32,
        new_trust: f32,
        utilization_rate: Option<f32>,
        injections: u32,
    },
    TelemetryPruned {
        cutoff_days: u32,
        rows_removed: usize,
    },
    KgDocRefreshed {
        module_path: String,
    },
}

impl SleeptimeOperation {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::DecayFlagged { .. } => "decay_flagged",
            Self::NearDupMerged { .. } => "near_dup_merged",
            Self::Promoted { .. } => "promoted",
            Self::TrustAdjusted { .. } => "trust_adjusted",
            Self::TelemetryPruned { .. } => "telemetry_pruned",
            Self::KgDocRefreshed { .. } => "kg_doc_refreshed",
        }
    }

    /// Memory id this op operated on (or `None` for sweep-level ops).
    pub fn memory_id(&self) -> Option<i64> {
        match self {
            Self::DecayFlagged { memory_id, .. }
            | Self::Promoted { memory_id, .. }
            | Self::TrustAdjusted { memory_id, .. } => Some(*memory_id),
            Self::NearDupMerged { keep_id, .. } => Some(*keep_id),
            Self::TelemetryPruned { .. } | Self::KgDocRefreshed { .. } => None,
        }
    }

    /// Optional related id (e.g. the merge loser).
    pub fn related_id(&self) -> Option<i64> {
        match self {
            Self::NearDupMerged { drop_id, .. } => Some(*drop_id),
            _ => None,
        }
    }

    /// Opaque payload JSON for the audit table — the renderer in the
    /// panel consumes this verbatim.
    pub fn payload_json(&self) -> serde_json::Value {
        match self {
            Self::DecayFlagged { memory_id, recency } => serde_json::json!({
                "memory_id": memory_id,
                "recency": recency,
            }),
            Self::NearDupMerged {
                keep_id,
                drop_id,
                cosine,
                keep_source,
                drop_source,
            } => serde_json::json!({
                "keep_id": keep_id,
                "drop_id": drop_id,
                "cosine": cosine,
                "keep_source": keep_source.as_str(),
                "drop_source": drop_source.as_str(),
            }),
            Self::Promoted {
                memory_id,
                from_scope_level,
                to_scope_level,
                memory_type,
            } => serde_json::json!({
                "memory_id": memory_id,
                "from_scope_level": from_scope_level,
                "to_scope_level": to_scope_level,
                "memory_type": memory_type.as_str(),
            }),
            Self::TrustAdjusted {
                memory_id,
                old_trust,
                new_trust,
                utilization_rate,
                injections,
            } => serde_json::json!({
                "memory_id": memory_id,
                "old_trust": old_trust,
                "new_trust": new_trust,
                "utilization_rate": utilization_rate,
                "injections": injections,
            }),
            Self::TelemetryPruned {
                cutoff_days,
                rows_removed,
            } => serde_json::json!({
                "cutoff_days": cutoff_days,
                "rows_removed": rows_removed,
            }),
            Self::KgDocRefreshed { module_path } => serde_json::json!({
                "module_path": module_path,
            }),
        }
    }
}

/// Source-aware pick: when merging, the `user_remember` row always
/// wins over an LLM-authored near-duplicate. Returns `(keep, drop)`.
pub fn pick_merge_winner(
    a_id: i64,
    a_source: MemorySource,
    a_trust: f32,
    b_id: i64,
    b_source: MemorySource,
    b_trust: f32,
) -> (i64, i64) {
    use MemorySource::*;
    let user_authored = |s| matches!(s, UserRemember | UserPanel);
    let a_user = user_authored(a_source);
    let b_user = user_authored(b_source);
    if a_user && !b_user {
        (a_id, b_id)
    } else if b_user && !a_user {
        (b_id, a_id)
    } else if a_trust >= b_trust {
        (a_id, b_id)
    } else {
        (b_id, a_id)
    }
}

/// Single non-LLM sleeptime entry point. Caller passes a pre-built
/// `SleeptimeConfig` resolved from settings.
///
/// The function never panics on failure of an individual sub-step:
/// decay sweep failures don't block near-dup merge, etc. Failures are
/// logged at `warn` and surfaced via the report's counters staying
/// at zero for the affected step.
pub async fn run_sleeptime(
    store: &Arc<MemoryStore>,
    cfg: &SleeptimeConfig,
    observer: Option<&dyn SleeptimeObserver>,
) -> Result<SleeptimeReport> {
    let run_id = format!(
        "sleep-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let mut report = SleeptimeReport {
        run_id: run_id.clone(),
        dry_run: cfg.dry_run,
        ..Default::default()
    };

    // Step 1 — decay sweep. Pull memories whose recency under B4 has
    // bottomed out, and flag them. We never delete from the sweep.
    if let Ok(flagged) = store.sleeptime_decay_sweep(cfg.dry_run).await {
        for op in &flagged {
            audit(store, &run_id, op, cfg.dry_run).await;
            if let Some(o) = observer {
                o.on_operation(op);
            }
        }
        report.decay_flagged = flagged.len();
    }

    // Step 2 — near-dup merge.
    if let Ok(ops) = store
        .sleeptime_near_dup_merge(cfg.near_dup_threshold, cfg.dry_run)
        .await
    {
        for op in &ops {
            audit(store, &run_id, op, cfg.dry_run).await;
            if let Some(o) = observer {
                o.on_operation(op);
            }
        }
        report.near_dup_merged = ops.len();
    }

    // Step 3 — cross-scope promotion.
    if let Ok(ops) = store.sleeptime_promote(cfg.dry_run).await {
        for op in &ops {
            audit(store, &run_id, op, cfg.dry_run).await;
            if let Some(o) = observer {
                o.on_operation(op);
            }
        }
        report.promoted = ops.len();
    }

    // Step 4 — trust re-scoring (B6-aware; falls back to manifest
    // injection counts).
    if let Ok(ops) = store.sleeptime_trust_rescore(cfg).await {
        for op in &ops {
            audit(store, &run_id, op, cfg.dry_run).await;
            if let Some(o) = observer {
                o.on_operation(op);
            }
        }
        report.trust_adjusted = ops.len();
    }

    // Step 5 — telemetry retention prune (90-day default).
    let prune_cutoff_days: u32 = 90;
    if !cfg.dry_run {
        if let Ok(rows_removed) = store.sleeptime_prune_telemetry(prune_cutoff_days).await {
            let op = SleeptimeOperation::TelemetryPruned {
                cutoff_days: prune_cutoff_days,
                rows_removed,
            };
            audit(store, &run_id, &op, cfg.dry_run).await;
            if let Some(o) = observer {
                o.on_operation(&op);
            }
            report.telemetry_pruned = rows_removed;
        }
    }

    // Step 6 — KG node-doc refresh stub (Tier D1).
    // Intentionally no-op until D1 lands.

    if let Some(o) = observer {
        o.on_complete(&report);
    }
    Ok(report)
}

async fn audit(store: &Arc<MemoryStore>, run_id: &str, op: &SleeptimeOperation, dry_run: bool) {
    let payload = op.payload_json().to_string();
    if let Err(e) = store
        .log_sleeptime_audit(
            run_id,
            op.kind(),
            op.memory_id(),
            op.related_id(),
            &payload,
            dry_run,
        )
        .await
        .context("logging sleeptime audit")
    {
        tracing::warn!(
            target: "memory_sleeptime",
            run_id = run_id,
            kind = op.kind(),
            error = %e,
            "audit row write failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_remember_always_wins_merge() {
        let (keep, drop) = pick_merge_winner(
            1,
            MemorySource::LlmExtracted,
            0.9,
            2,
            MemorySource::UserRemember,
            0.5,
        );
        assert_eq!(keep, 2);
        assert_eq!(drop, 1);
    }

    #[test]
    fn user_panel_also_wins_over_llm() {
        let (keep, drop) = pick_merge_winner(
            1,
            MemorySource::LlmAnnotated,
            0.95,
            2,
            MemorySource::UserPanel,
            0.5,
        );
        assert_eq!(keep, 2);
        assert_eq!(drop, 1);
    }

    #[test]
    fn between_two_llm_sources_higher_trust_wins() {
        let (keep, drop) = pick_merge_winner(
            1,
            MemorySource::LlmExtracted,
            0.7,
            2,
            MemorySource::LlmAnnotated,
            0.6,
        );
        assert_eq!(keep, 1);
        assert_eq!(drop, 2);
    }

    #[test]
    fn op_payload_round_trips_kind() {
        let op = SleeptimeOperation::DecayFlagged {
            memory_id: 42,
            recency: 0.05,
        };
        assert_eq!(op.kind(), "decay_flagged");
        assert_eq!(op.memory_id(), Some(42));
        assert!(op.related_id().is_none());
        let p = op.payload_json();
        assert_eq!(p["memory_id"], 42);
    }

    #[test]
    fn near_dup_merge_op_carries_both_ids() {
        let op = SleeptimeOperation::NearDupMerged {
            keep_id: 5,
            drop_id: 6,
            cosine: 0.95,
            keep_source: MemorySource::UserRemember,
            drop_source: MemorySource::LlmExtracted,
        };
        assert_eq!(op.memory_id(), Some(5));
        assert_eq!(op.related_id(), Some(6));
    }
}
