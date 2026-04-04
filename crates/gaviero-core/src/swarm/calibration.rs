//! Adaptive tier calibration: tracks per-tier success rates across runs.
//!
//! After each swarm run, stores accuracy stats to memory. The coordinator
//! can query these stats to improve future tier assignments.

use std::sync::Arc;

use crate::memory::MemoryStore;
use crate::swarm::models::{AgentManifest, AgentStatus, WorkUnit};
use crate::types::ModelTier;

/// Per-tier accuracy statistics for a single run.
#[derive(Debug, Clone, Default)]
pub struct TierStats {
    pub mechanical_total: usize,
    pub mechanical_succeeded: usize,
    pub mechanical_escalated: usize,
    pub execution_total: usize,
    pub execution_succeeded: usize,
    pub execution_escalated: usize,
    pub reasoning_total: usize,
    pub reasoning_succeeded: usize,
    pub reasoning_escalated: usize,
}

impl TierStats {
    /// Compute stats from completed manifests and their work units.
    pub fn from_results(manifests: &[AgentManifest], units: &[WorkUnit]) -> Self {
        let mut stats = TierStats::default();

        for manifest in manifests {
            let Some(unit) = units.iter().find(|u| u.id == manifest.work_unit_id) else {
                continue;
            };
            let succeeded = matches!(manifest.status, AgentStatus::Completed);

            match unit.tier {
                ModelTier::Mechanical => {
                    stats.mechanical_total += 1;
                    if succeeded {
                        stats.mechanical_succeeded += 1;
                    }
                }
                ModelTier::Execution => {
                    stats.execution_total += 1;
                    if succeeded {
                        stats.execution_succeeded += 1;
                    }
                }
                ModelTier::Reasoning | ModelTier::Coordinator => {
                    stats.reasoning_total += 1;
                    if succeeded {
                        stats.reasoning_succeeded += 1;
                    }
                }
            }
        }

        stats
    }

    /// Format as a human-readable summary for memory storage.
    pub fn to_summary(&self, run_id: &str) -> String {
        format!(
            "Tier accuracy for run {}: \
             mechanical={}/{} (escalations: {}), \
             execution={}/{} (escalations: {}), \
             reasoning={}/{} (escalations: {})",
            run_id,
            self.mechanical_succeeded, self.mechanical_total, self.mechanical_escalated,
            self.execution_succeeded, self.execution_total, self.execution_escalated,
            self.reasoning_succeeded, self.reasoning_total, self.reasoning_escalated,
        )
    }

    /// Success rate for a given tier (0.0 - 1.0, or None if no data).
    pub fn success_rate(&self, tier: ModelTier) -> Option<f64> {
        let (succeeded, total) = match tier {
            ModelTier::Mechanical => (self.mechanical_succeeded, self.mechanical_total),
            ModelTier::Execution => (self.execution_succeeded, self.execution_total),
            ModelTier::Reasoning | ModelTier::Coordinator => (self.reasoning_succeeded, self.reasoning_total),
        };
        if total == 0 { None } else { Some(succeeded as f64 / total as f64) }
    }
}

/// Store tier accuracy stats to memory after a swarm run.
pub async fn store_tier_stats(
    memory: &Option<Arc<MemoryStore>>,
    namespace: &str,
    run_id: &str,
    stats: &TierStats,
) {
    let Some(mem) = memory else { return };
    let key = format!("tiers:{}", run_id);
    let content = stats.to_summary(run_id);

    if let Err(e) = mem.store(namespace, &key, &content, None).await {
        tracing::warn!("Failed to store tier stats: {}", e);
    }
}

/// Store a verification summary to memory after Phase 4.
pub async fn store_verification_summary(
    memory: &Option<Arc<MemoryStore>>,
    namespace: &str,
    run_id: &str,
    passed: bool,
    escalation_count: usize,
    details: &str,
) {
    let Some(mem) = memory else { return };
    let key = format!("verification:{}", run_id);
    let content = format!(
        "Verification {}: {} escalations. {}",
        if passed { "PASSED" } else { "FAILED" },
        escalation_count,
        details,
    );

    if let Err(e) = mem.store(namespace, &key, &content, None).await {
        tracing::warn!("Failed to store verification summary: {}", e);
    }
}

/// Query recent tier accuracy from memory to inform coordinator calibration.
///
/// Returns a formatted string suitable for inclusion in the coordinator prompt.
pub async fn query_tier_history(
    memory: &Option<Arc<MemoryStore>>,
    namespaces: &[String],
    limit: usize,
) -> String {
    let Some(mem) = memory else { return String::new() };

    let results = match mem.search_multi(namespaces, "tier accuracy escalation", limit).await {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    let tier_entries: Vec<_> = results.iter()
        .filter(|r| r.entry.key.starts_with("tiers:"))
        .collect();

    if tier_entries.is_empty() {
        return String::new();
    }

    let mut out = String::from("[Tier calibration history]:\n");
    for entry in &tier_entries {
        out.push_str(&format!("- {}\n", entry.entry.content));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::models::{AgentManifest, AgentStatus};
    use crate::types::{FileScope, PrivacyLevel};
    use std::collections::HashMap;

    fn make_unit(id: &str, tier: ModelTier) -> WorkUnit {
        WorkUnit {
            id: id.into(),
            description: format!("Task {}", id),
            scope: FileScope {
                owned_paths: vec![],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            backend: Default::default(),
            model: None,
            tier,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
        }
    }

    fn make_manifest(id: &str, status: AgentStatus) -> AgentManifest {
        AgentManifest {
            work_unit_id: id.into(),
            status,
            modified_files: vec![],
            branch: None,
            summary: None,
            cost_usd: 0.0,
        }
    }

    #[test]
    fn test_tier_stats_from_results() {
        let units = vec![
            make_unit("a", ModelTier::Reasoning),
            make_unit("b", ModelTier::Execution),
            make_unit("c", ModelTier::Execution),
            make_unit("d", ModelTier::Mechanical),
        ];
        let manifests = vec![
            make_manifest("a", AgentStatus::Completed),
            make_manifest("b", AgentStatus::Completed),
            make_manifest("c", AgentStatus::Failed("error".into())),
            make_manifest("d", AgentStatus::Completed),
        ];

        let stats = TierStats::from_results(&manifests, &units);
        assert_eq!(stats.reasoning_total, 1);
        assert_eq!(stats.reasoning_succeeded, 1);
        assert_eq!(stats.execution_total, 2);
        assert_eq!(stats.execution_succeeded, 1);
        assert_eq!(stats.mechanical_total, 1);
        assert_eq!(stats.mechanical_succeeded, 1);
    }

    #[test]
    fn test_success_rate() {
        let mut stats = TierStats::default();
        stats.execution_total = 10;
        stats.execution_succeeded = 8;

        assert_eq!(stats.success_rate(ModelTier::Execution), Some(0.8));
        assert_eq!(stats.success_rate(ModelTier::Mechanical), None);
    }

    #[test]
    fn test_to_summary() {
        let stats = TierStats {
            mechanical_total: 5,
            mechanical_succeeded: 4,
            mechanical_escalated: 1,
            execution_total: 3,
            execution_succeeded: 3,
            execution_escalated: 0,
            reasoning_total: 1,
            reasoning_succeeded: 1,
            reasoning_escalated: 0,
        };
        let summary = stats.to_summary("run42");
        assert!(summary.contains("run42"));
        assert!(summary.contains("mechanical=4/5"));
        assert!(summary.contains("execution=3/3"));
    }

    #[test]
    fn test_empty_stats() {
        let stats = TierStats::from_results(&[], &[]);
        assert_eq!(stats.mechanical_total, 0);
        assert_eq!(stats.execution_total, 0);
        assert_eq!(stats.reasoning_total, 0);
    }

    #[tokio::test]
    async fn test_store_tier_stats_no_memory() {
        // Should not panic
        store_tier_stats(&None, "ns", "run1", &TierStats::default()).await;
    }

    #[tokio::test]
    async fn test_query_tier_history_no_memory() {
        let result = query_tier_history(&None, &["ns".into()], 5).await;
        assert!(result.is_empty());
    }
}
