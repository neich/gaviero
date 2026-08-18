//! Serializable plan IR for CLI `--plan` and external builders.
//!
//! `PlanDocument` round-trips the fields needed to rebuild a [`CompiledPlan`]
//! without going through the DSL. Loops and fan-out are preserved (unlike bare
//! `--work-units` JSON).

use serde::{Deserialize, Serialize};

use super::models::WorkUnit;
use super::plan::{CompiledPlan, ExecutionMode, FanoutOp, LoopConfig, VerificationConfig};
use crate::iteration::IterationConfig;

/// Versioned document consumed by `gaviero-cli --plan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDocument {
    #[serde(default = "plan_doc_version")]
    pub version: u32,
    pub work_units: Vec<WorkUnit>,
    #[serde(default)]
    pub max_parallel: Option<usize>,
    #[serde(default)]
    pub iteration_config: IterationConfig,
    #[serde(default)]
    pub verification_config: VerificationConfig,
    #[serde(default)]
    pub loop_configs: Vec<LoopConfig>,
    #[serde(default)]
    pub loop_judge_units: Vec<WorkUnit>,
    #[serde(default)]
    pub fanout_ops: Vec<FanoutOp>,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    /// Default memory read namespaces when units omit their own.
    #[serde(default)]
    pub read_namespaces: Vec<String>,
    /// Default memory write namespace when units omit their own.
    #[serde(default)]
    pub write_namespace: Option<String>,
}

fn plan_doc_version() -> u32 {
    1
}

impl PlanDocument {
    pub fn from_compiled(plan: &CompiledPlan) -> anyhow::Result<Self> {
        Ok(Self {
            version: 1,
            work_units: plan.work_units_ordered()?,
            max_parallel: plan.max_parallel,
            iteration_config: plan.iteration_config.clone(),
            verification_config: plan.verification_config.clone(),
            loop_configs: plan.loop_configs.clone(),
            loop_judge_units: plan.loop_judge_units.clone(),
            fanout_ops: plan.fanout_ops.clone(),
            execution_mode: plan.execution_mode,
            read_namespaces: vec![],
            write_namespace: None,
        })
    }

    pub fn into_compiled(self) -> CompiledPlan {
        let mut plan = CompiledPlan::from_work_units(self.work_units, self.max_parallel);
        plan.iteration_config = self.iteration_config;
        plan.verification_config = self.verification_config;
        plan.loop_configs = self.loop_configs;
        plan.loop_judge_units = self.loop_judge_units;
        plan.fanout_ops = self.fanout_ops;
        plan.execution_mode = self.execution_mode;
        plan
    }
}

impl From<PlanDocument> for CompiledPlan {
    fn from(doc: PlanDocument) -> Self {
        doc.into_compiled()
    }
}

/// Workspace capability flags for dual-surface memory + knowledge contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceCapabilities {
    pub memory: bool,
    pub knowledge: bool,
}

impl WorkspaceCapabilities {
    pub fn detect(memory: Option<&crate::memory::MemoryStores>, knowledge_ok: bool) -> Self {
        Self {
            memory: memory.is_some(),
            knowledge: knowledge_ok,
        }
    }

    /// Fill SwarmConfig namespace defaults when memory is present but empty.
    pub fn apply_swarm_memory_defaults(
        &self,
        read_namespaces: &mut Vec<String>,
        write_namespace: &mut String,
    ) {
        if !self.memory {
            return;
        }
        if read_namespaces.is_empty() {
            read_namespaces.push("shared".into());
        }
        if write_namespace.is_empty() {
            *write_namespace = "swarm".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;

    fn unit(id: &str) -> WorkUnit {
        WorkUnit {
            id: id.into(),
            description: id.into(),
            scope: FileScope::default(),
            produces: vec![],
            depends_on: vec![],
            backend: Default::default(),
            model: Some("claude:sonnet".into()),
            effort: None,
            extra: vec![],
            tier: Default::default(),
            privacy: Default::default(),
            coordinator_instructions: "do it".into(),
            estimated_tokens: 0,
            max_retries: 1,
            timeout_secs: 3600,
            escalation_tier: None,
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

    #[test]
    fn plan_document_round_trip() {
        let mut plan = CompiledPlan::from_work_units(vec![unit("a"), unit("b")], Some(2));
        plan.fanout_ops.push(FanoutOp {
            after_unit: "a".into(),
            max_spawn: 8,
            default_model: Some("claude:sonnet".into()),
            barrier_id: None,
        });
        let doc = PlanDocument::from_compiled(&plan).unwrap();
        let json = serde_json::to_string_pretty(&doc).unwrap();
        let back: PlanDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back.work_units.len(), 2);
        assert_eq!(back.fanout_ops.len(), 1);
        let compiled: CompiledPlan = back.into();
        assert_eq!(compiled.fanout_ops[0].after_unit, "a");
        assert_eq!(compiled.fanout_ops[0].max_spawn, 8);
    }
}
