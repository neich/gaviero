//! Runtime fan-out: materialize WorkUnits from a SpawnManifest.

use anyhow::{Context, Result, bail};

use crate::swarm::backend::shared::validate_model_spec;
use crate::swarm::models::WorkUnit;
use crate::swarm::plan::{FanoutOp, SpawnManifest, SpawnWorkerSpec};
use crate::types::FileScope;

/// Parse and materialize workers from spawn-manifest JSON.
///
/// Caps at `op.max_spawn` (default 16). Validates every `provider:model` spec.
pub fn materialize_from_json(json: &str, op: &FanoutOp) -> Result<Vec<WorkUnit>> {
    let manifest: SpawnManifest =
        serde_json::from_str(json).context("parsing spawn_manifest.json")?;
    materialize_from_manifest(&manifest, op)
}

/// Materialize workers from an already-parsed manifest.
pub fn materialize_from_manifest(manifest: &SpawnManifest, op: &FanoutOp) -> Result<Vec<WorkUnit>> {
    if manifest.workers.is_empty() {
        bail!("spawn manifest has no workers");
    }
    let cap = op.max_spawn.max(1) as usize;
    if manifest.workers.len() > cap {
        bail!(
            "spawn manifest has {} workers; max_spawn is {}",
            manifest.workers.len(),
            cap
        );
    }

    let mut units = Vec::with_capacity(manifest.workers.len());
    for w in &manifest.workers {
        units.push(worker_to_unit(w, op)?);
    }
    Ok(units)
}

fn worker_to_unit(w: &SpawnWorkerSpec, op: &FanoutOp) -> Result<WorkUnit> {
    let model = if w.model.is_empty() {
        op.default_model
            .clone()
            .context("worker missing model and FanoutOp has no default_model")?
    } else {
        w.model.clone()
    };
    validate_model_spec(&model).with_context(|| format!("worker '{}'", w.id))?;

    let prompt = if w.prompt.is_empty() {
        w.description.clone()
    } else {
        w.prompt.clone()
    };

    Ok(WorkUnit {
        id: w.id.clone(),
        description: if w.description.is_empty() {
            format!("fan-out worker {}", w.id)
        } else {
            w.description.clone()
        },
        scope: FileScope {
            owned_paths: w.owned.clone(),
            read_only_paths: w.read_only.clone(),
            interface_contracts: Default::default(),
        },
        // Runtime fan-out workers come from a SpawnManifest, which carries
        // scope but no output contract.
        produces: Vec::new(),
        depends_on: vec![op.after_unit.clone()],
        backend: Default::default(),
        model: Some(model),
        effort: w.effort.clone(),
        extra: vec![],
        tier: Default::default(),
        privacy: Default::default(),
        coordinator_instructions: prompt,
        estimated_tokens: 0,
        max_retries: 1,
        timeout_secs: crate::swarm::models::DEFAULT_AGENT_TIMEOUT_SECS,
        escalation_tier: None,
        read_namespaces: None,
        write_namespace: w.write_ns.clone(),
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
    })
}

/// Load spawn manifest JSON from a filesystem path.
pub fn load_manifest_file(path: &std::path::Path) -> Result<SpawnManifest> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading spawn manifest {}", path.display()))?;
    serde_json::from_str(&text).context("parsing spawn manifest file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::plan::FanoutOp;

    fn sample_json(n: usize) -> String {
        let workers: Vec<String> = (0..n)
            .map(|i| {
                format!(
                    r#"{{"id":"w{i}","model":"claude:sonnet","description":"d{i}","prompt":"p{i}","owned":["src/{i}/"]}}"#
                )
            })
            .collect();
        format!(r#"{{"version":1,"workers":[{}]}}"#, workers.join(","))
    }

    #[test]
    fn materialize_three_workers() {
        let op = FanoutOp {
            after_unit: "discover".into(),
            max_spawn: 16,
            default_model: None,
            barrier_id: None,
        };
        let units = materialize_from_json(&sample_json(3), &op).unwrap();
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].id, "w0");
        assert_eq!(units[0].model.as_deref(), Some("claude:sonnet"));
        assert_eq!(units[0].depends_on, vec!["discover".to_string()]);
        assert_eq!(units[2].scope.owned_paths, vec!["src/2/".to_string()]);
    }

    #[test]
    fn rejects_bare_model() {
        let op = FanoutOp {
            after_unit: "discover".into(),
            max_spawn: 16,
            default_model: None,
            barrier_id: None,
        };
        let json = r#"{"version":1,"workers":[{"id":"w","model":"sonnet"}]}"#;
        assert!(materialize_from_json(json, &op).is_err());
    }

    #[test]
    fn rejects_over_cap() {
        let op = FanoutOp {
            after_unit: "discover".into(),
            max_spawn: 2,
            default_model: None,
            barrier_id: None,
        };
        assert!(materialize_from_json(&sample_json(3), &op).is_err());
    }
}
