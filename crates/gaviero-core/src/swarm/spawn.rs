//! Runtime fan-out: materialize WorkUnits from a SpawnManifest.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::swarm::backend::shared::validate_model_spec;
use crate::swarm::models::WorkUnit;
use crate::swarm::plan::{FanoutOp, SpawnManifest, SpawnWorkerSpec, TierBinding};
use crate::types::FileScope;

/// Parse and materialize workers from spawn-manifest JSON.
///
/// Caps at `op.max_spawn` (default 16). A worker's `tier` resolves through
/// `tiers` (the plan's profile bindings).
pub fn materialize_from_json(
    json: &str,
    op: &FanoutOp,
    tiers: &HashMap<String, TierBinding>,
) -> Result<Vec<WorkUnit>> {
    let manifest: SpawnManifest =
        serde_json::from_str(json).context("parsing spawn_manifest.json")?;
    materialize_from_manifest(&manifest, op, tiers)
}

/// Materialize workers from an already-parsed manifest.
pub fn materialize_from_manifest(
    manifest: &SpawnManifest,
    op: &FanoutOp,
    tiers: &HashMap<String, TierBinding>,
) -> Result<Vec<WorkUnit>> {
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
        units.push(worker_to_unit(w, op, tiers)?);
    }
    Ok(units)
}

fn worker_to_unit(
    w: &SpawnWorkerSpec,
    op: &FanoutOp,
    tiers: &HashMap<String, TierBinding>,
) -> Result<WorkUnit> {
    let binding = match w.tier.as_deref().filter(|t| !t.is_empty()) {
        Some(tier) => tiers.get(tier).cloned().with_context(|| {
            let mut known: Vec<&str> = tiers.keys().map(String::as_str).collect();
            known.sort_unstable();
            format!(
                "worker '{}' names tier `{tier}`, which the run's model profile does not \
                 define (it defines: {})",
                w.id,
                known.join(", ")
            )
        })?,
        None => {
            let model = op.default_model.clone().with_context(|| {
                format!("worker '{}' names no `tier` (a tier of the run's model profile)", w.id)
            })?;
            validate_model_spec(&model).with_context(|| format!("worker '{}'", w.id))?;
            TierBinding {
                model: Some(model),
                effort: None,
                extra: vec![],
                privacy: Default::default(),
                tier: Default::default(),
            }
        }
    };

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
        produces_json: vec![],
        produces: Vec::new(),
        depends_on: vec![op.after_unit.clone()],
        backend: Default::default(),
        model: binding.model,
        effort: binding.effort,
        extra: binding.extra,
        tier: binding.tier,
        privacy: binding.privacy,
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

    use crate::types::{ModelTier, PrivacyLevel};

    fn sample_json(n: usize) -> String {
        let workers: Vec<String> = (0..n)
            .map(|i| {
                format!(
                    r#"{{"id":"w{i}","tier":"low","description":"d{i}","prompt":"p{i}","owned":["src/{i}/"]}}"#
                )
            })
            .collect();
        format!(r#"{{"version":1,"workers":[{}]}}"#, workers.join(","))
    }

    fn op(max_spawn: u32, default_model: Option<&str>) -> FanoutOp {
        FanoutOp {
            after_unit: "discover".into(),
            max_spawn,
            default_model: default_model.map(str::to_string),
            barrier_id: None,
        }
    }

    fn tiers() -> HashMap<String, TierBinding> {
        HashMap::from([(
            "low".to_string(),
            TierBinding {
                model: Some("claude:sonnet".into()),
                effort: Some("medium".into()),
                extra: vec![],
                privacy: PrivacyLevel::Public,
                tier: ModelTier::Cheap,
            },
        )])
    }

    #[test]
    fn materialize_three_workers() {
        let units = materialize_from_json(&sample_json(3), &op(16, None), &tiers()).unwrap();
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].id, "w0");
        assert_eq!(units[0].model.as_deref(), Some("claude:sonnet"));
        assert_eq!(units[0].effort.as_deref(), Some("medium"));
        assert_eq!(units[0].depends_on, vec!["discover".to_string()]);
        assert_eq!(units[2].scope.owned_paths, vec!["src/2/".to_string()]);
    }

    #[test]
    fn a_worker_names_a_tier_of_the_profile() {
        let unknown = r#"{"version":1,"workers":[{"id":"w","tier":"top"}]}"#;
        let error = materialize_from_json(unknown, &op(16, None), &tiers())
            .unwrap_err()
            .to_string();
        assert!(error.contains("it defines: low"), "{error}");

        let none = r#"{"version":1,"workers":[{"id":"w"}]}"#;
        let error = materialize_from_json(none, &op(16, None), &tiers())
            .unwrap_err()
            .to_string();
        assert!(error.contains("names no `tier`"), "{error}");
        // A core-built op may still supply a default.
        let units = materialize_from_json(none, &op(16, Some("claude:haiku")), &tiers()).unwrap();
        assert_eq!(units[0].model.as_deref(), Some("claude:haiku"));
    }

    /// Models are chosen by tier only: a worker cannot name a model or an
    /// effort.
    #[test]
    fn rejects_model_and_effort_keys() {
        for json in [
            r#"{"version":1,"workers":[{"id":"w","model":"claude:sonnet"}]}"#,
            r#"{"version":1,"workers":[{"id":"w","tier":"low","effort":"high"}]}"#,
        ] {
            assert!(materialize_from_json(json, &op(16, None), &tiers()).is_err(), "{json}");
        }
    }

    #[test]
    fn rejects_over_cap() {
        assert!(materialize_from_json(&sample_json(3), &op(2, None), &tiers()).is_err());
    }
}
