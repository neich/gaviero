//! Runtime-sized `for`: `for <var> in json "<path>" parallel max <N> { <agent> }`.
//!
//! The DSL compiles the body agent once, as a template whose `{{<var>.*}}`
//! placeholders survive compilation, and records a
//! [`RuntimeForOp`](crate::swarm::plan::RuntimeForOp). When the scheduler
//! reaches the template it reads the JSON list ([`load_items`]) and clones the
//! template once per item ([`materialize`]); the pipeline dispatches the clones
//! in parallel and records the template as a barrier over them.
//!
//! The list file is a JSON **array of flat objects**: `id` is required (a
//! string matching `[A-Za-z0-9][A-Za-z0-9_.-]*`, unique), `tier` is optional
//! (a tier of the run's model profile; the clone runs on it instead of the
//! template's tier), and every other value becomes an item field. `model` and
//! `effort` keys are rejected: a model is chosen by tier only. Strings are
//! taken as-is, numbers and bools are rendered as text, arrays and objects as
//! compact JSON. Item values are not themselves var-substituted.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::swarm::models::WorkUnit;
use crate::swarm::plan::{RuntimeForOp, TierBinding};

/// One element of a runtime `for` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeItem {
    pub id: String,
    /// A tier of the run's model profile (`plan.tier_bindings`).
    pub tier: Option<String>,
    /// Every other key, in file order.
    pub fields: Vec<(String, String)>,
}

fn valid_item_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Read and validate the list file at `workspace_root/rel_path`.
pub fn load_items(workspace_root: &Path, rel_path: &str) -> Result<Vec<RuntimeItem>> {
    let path = workspace_root.join(rel_path);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("runtime for: reading list file {rel_path}"))?;
    parse_items(&text).with_context(|| format!("runtime for: list file {rel_path}"))
}

/// Parse and validate a list document (see the module docs for the shape).
pub fn parse_items(text: &str) -> Result<Vec<RuntimeItem>> {
    let value: serde_json::Value = serde_json::from_str(text).context("invalid JSON")?;
    let serde_json::Value::Array(elements) = value else {
        bail!("the top level must be a JSON array of objects");
    };
    let mut items = Vec::with_capacity(elements.len());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (index, element) in elements.iter().enumerate() {
        let serde_json::Value::Object(map) = element else {
            bail!("element {} is not a JSON object", index + 1);
        };
        let id = match map.get("id") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(_) => bail!("element {}: `id` must be a string", index + 1),
            None => bail!("element {} has no `id`", index + 1),
        };
        if !valid_item_id(&id) {
            bail!(
                "element {}: id `{id}` must match [A-Za-z0-9][A-Za-z0-9_.-]*",
                index + 1
            );
        }
        if !seen.insert(id.clone()) {
            bail!("duplicate id `{id}`");
        }
        let mut item = RuntimeItem {
            id,
            tier: None,
            fields: Vec::new(),
        };
        for (key, value) in map {
            match key.as_str() {
                "id" => {}
                "tier" => item.tier = Some(render_value(value)).filter(|t| !t.is_empty()),
                "model" | "effort" => bail!(
                    "element {}: `{key}` is not an item key — give the item a `tier` of \
                     the run's model profile instead",
                    index + 1
                ),
                _ => item.fields.push((key.clone(), render_value(value))),
            }
        }
        items.push(item);
    }
    Ok(items)
}

/// The `{{<var>.<key>}}` values for item `index` of `items`.
pub fn item_vars(var: &str, items: &[RuntimeItem], index: usize) -> Vec<(String, String)> {
    let item = &items[index];
    let peers: Vec<&str> = items
        .iter()
        .filter(|other| other.id != item.id)
        .map(|other| other.id.as_str())
        .collect();
    let mut vars = vec![(format!("{var}.id"), item.id.clone())];
    for (key, value) in &item.fields {
        vars.push((format!("{var}.{key}"), value.clone()));
    }
    if let Some(tier) = &item.tier {
        vars.push((format!("{var}.tier"), tier.clone()));
    }
    vars.push((format!("{var}.index"), (index + 1).to_string()));
    vars.push((format!("{var}.count"), items.len().to_string()));
    vars.push((format!("{var}.peers"), peers.join(",")));
    vars.push((format!("{var}.peer_count"), peers.len().to_string()));
    vars
}

fn substitute(text: &str, vars: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// The first `{{<var>.…}}` placeholder left in `text`, if any.
fn leftover_placeholder(text: &str, var: &str) -> Option<String> {
    let needle = format!("{{{{{var}.");
    let start = text.find(&needle)?;
    let rest = &text[start..];
    let end = rest.find("}}").map(|e| e + 2).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Clone `template` once per item.
///
/// Clones are named `<unit_id>@<item-id>`. Item vars are substituted in the
/// description, instructions, owned / read-only paths and declared outputs;
/// any `{{<var>.…}}` left afterwards is an error naming the item. An item's
/// `tier` replaces the template's model with that tier's binding.
pub fn materialize(
    template: &WorkUnit,
    op: &RuntimeForOp,
    items: &[RuntimeItem],
    tiers: &HashMap<String, TierBinding>,
) -> Result<Vec<WorkUnit>> {
    if items.len() > op.max_items as usize {
        bail!(
            "runtime for `{}`: the list has {} items, over its `max {}`",
            op.unit_id,
            items.len(),
            op.max_items
        );
    }
    let mut clones = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let vars = item_vars(&op.var, items, index);
        let mut clone = template.clone();
        clone.id = format!("{}@{}", op.unit_id, item.id);
        clone.description = substitute(&template.description, &vars);
        clone.coordinator_instructions = substitute(&template.coordinator_instructions, &vars);
        clone.scope.owned_paths = template
            .scope
            .owned_paths
            .iter()
            .map(|p| substitute(p, &vars))
            .collect();
        clone.scope.read_only_paths = template
            .scope
            .read_only_paths
            .iter()
            .map(|p| substitute(p, &vars))
            .collect();
        clone.produces = template
            .produces
            .iter()
            .map(|p| substitute(p, &vars))
            .collect();
        clone.produces_json = template
            .produces_json
            .iter()
            .map(|p| substitute(p, &vars))
            .collect();

        let texts = std::iter::once(&clone.description)
            .chain(std::iter::once(&clone.coordinator_instructions))
            .chain(clone.scope.owned_paths.iter())
            .chain(clone.scope.read_only_paths.iter())
            .chain(clone.produces.iter())
            .chain(clone.produces_json.iter());
        for text in texts {
            if let Some(placeholder) = leftover_placeholder(text, &op.var) {
                bail!(
                    "runtime for `{}`: item `{}` has no value for {placeholder}",
                    op.unit_id,
                    item.id
                );
            }
        }

        if let Some(alias) = item.tier.as_deref() {
            let binding = tiers.get(alias).with_context(|| {
                let mut known: Vec<&str> = tiers.keys().map(String::as_str).collect();
                known.sort_unstable();
                format!(
                    "runtime for `{}`: item `{}` names tier `{alias}`, which the run's \
                     model profile does not define (it defines: {})",
                    op.unit_id,
                    item.id,
                    known.join(", ")
                )
            })?;
            clone.model = binding.model.clone();
            clone.effort = binding.effort.clone();
            clone.extra = binding.extra.clone();
            clone.privacy = binding.privacy;
            clone.tier = binding.tier;
        }
        if let (Some(model), Some(effort)) = (clone.model.as_deref(), clone.effort.as_deref())
            && let Some(problem) = crate::swarm::backend::effort::effort_problem(model, effort)
        {
            bail!(
                "runtime for `{}`: item `{}`: {problem}",
                op.unit_id,
                item.id
            );
        }
        clones.push(clone);
    }
    Ok(clones)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileScope, ModelTier, PrivacyLevel};

    #[allow(deprecated)]
    fn template() -> WorkUnit {
        WorkUnit {
            id: "worker".into(),
            description: "investigate {{q.id}}".into(),
            scope: FileScope {
                owned_paths: vec!["out/{{q.id}}-v*.md".into()],
                read_only_paths: vec!["brief/{{q.focus}}.md".into()],
                interface_contracts: Default::default(),
            },
            produces_json: vec![],
            produces: vec!["out/{{q.id}}-v{{ITER}}.md".into()],
            depends_on: vec!["planner".into()],
            backend: Default::default(),
            model: Some("claude:sonnet".into()),
            effort: None,
            extra: vec![],
            tier: ModelTier::Cheap,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: "MARK-{{q.id}} focus={{q.focus}} {{q.index}}/{{q.count}} peers={{q.peers}}".into(),
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

    fn op(max_items: u32) -> RuntimeForOp {
        RuntimeForOp {
            unit_id: "worker".into(),
            var: "q".into(),
            source: "out/items.json".into(),
            max_items,
        }
    }

    #[test]
    fn loads_flat_objects_and_stringifies_scalars() {
        let items = parse_items(
            r#"[{"id":"a","focus":"x","n":3,"ok":true,"tags":["p","q"],"tier":"high"}]"#,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "a");
        assert_eq!(items[0].tier.as_deref(), Some("high"));
        let field = |k: &str| {
            items[0]
                .fields
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(field("focus").as_deref(), Some("x"));
        assert_eq!(field("n").as_deref(), Some("3"));
        assert_eq!(field("ok").as_deref(), Some("true"));
        assert_eq!(field("tags").as_deref(), Some(r#"["p","q"]"#));
    }

    #[test]
    fn loads_items_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("items.json"), r#"[{"id":"a"}]"#).unwrap();
        assert_eq!(load_items(dir.path(), "items.json").unwrap()[0].id, "a");
        let missing = load_items(dir.path(), "nope.json").unwrap_err();
        assert!(format!("{missing:#}").contains("nope.json"), "{missing:#}");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let err = parse_items(r#"[{"id":"a"},{"id":"a"}]"#).unwrap_err();
        assert!(err.to_string().contains("duplicate id `a`"), "{err}");
    }

    #[test]
    fn rejects_non_array() {
        assert!(parse_items(r#"{"items":[]}"#).is_err());
        assert!(parse_items(r#"[1]"#).is_err());
        assert!(parse_items(r#"[{"name":"no id"}]"#).is_err());
        assert!(parse_items(r#"[{"id":"-bad"}]"#).is_err());
    }

    #[test]
    fn materialize_names_clones_and_substitutes_paths() {
        let items = parse_items(r#"[{"id":"a","focus":"x"},{"id":"b","focus":"y"}]"#).unwrap();
        let clones = materialize(&template(), &op(16), &items, &HashMap::new()).unwrap();
        assert_eq!(clones.len(), 2);
        assert_eq!(clones[0].id, "worker@a");
        assert_eq!(clones[1].id, "worker@b");
        assert_eq!(clones[0].description, "investigate a");
        assert_eq!(clones[0].scope.owned_paths, vec!["out/a-v*.md"]);
        assert_eq!(clones[1].scope.read_only_paths, vec!["brief/y.md"]);
        // {{ITER}} is left for the loop pass.
        assert_eq!(clones[0].produces, vec!["out/a-v{{ITER}}.md"]);
        assert_eq!(
            clones[1].coordinator_instructions,
            "MARK-b focus=y 2/2 peers=a"
        );
        assert_eq!(clones[0].depends_on, vec!["planner".to_string()]);
    }

    #[test]
    fn materialize_rejects_over_cap() {
        let items = parse_items(r#"[{"id":"a","focus":"x"},{"id":"b","focus":"y"},{"id":"c","focus":"z"}]"#)
            .unwrap();
        let err = materialize(&template(), &op(2), &items, &HashMap::new()).unwrap_err();
        assert!(err.to_string().contains("max 2"), "{err}");
    }

    #[test]
    fn materialize_rejects_unknown_item_placeholder() {
        let items = parse_items(r#"[{"id":"a"}]"#).unwrap();
        let err = materialize(&template(), &op(16), &items, &HashMap::new()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("{{q.focus}}"), "{msg}");
        assert!(msg.contains("`a`"), "{msg}");
    }

    #[test]
    fn materialize_resolves_tier_models() {
        let items = parse_items(r#"[{"id":"a","focus":"x","tier":"high"}]"#).unwrap();
        let mut tiers = HashMap::new();
        tiers.insert(
            "high".to_string(),
            TierBinding {
                model: Some("codex:gpt-5.5".into()),
                effort: Some("high".into()),
                extra: vec![("k".into(), "v".into())],
                privacy: PrivacyLevel::Public,
                tier: ModelTier::Expensive,
            },
        );
        let clones = materialize(&template(), &op(16), &items, &tiers).unwrap();
        assert_eq!(clones[0].model.as_deref(), Some("codex:gpt-5.5"));
        assert_eq!(clones[0].effort.as_deref(), Some("high"));
        assert_eq!(clones[0].extra, vec![("k".to_string(), "v".to_string())]);
        assert_eq!(clones[0].tier, ModelTier::Expensive);

        let unknown = parse_items(r#"[{"id":"a","focus":"x","tier":"nope"}]"#).unwrap();
        let err = materialize(&template(), &op(16), &unknown, &tiers).unwrap_err();
        assert!(err.to_string().contains("it defines: high"), "{err}");

        // Without `tier` the clone keeps the template's model.
        let plain = parse_items(r#"[{"id":"a","focus":"x"}]"#).unwrap();
        let clones = materialize(&template(), &op(16), &plain, &tiers).unwrap();
        assert_eq!(clones[0].model.as_deref(), Some("claude:sonnet"));
    }

    /// A model is chosen by tier only: an item may not name a model or an
    /// effort directly.
    #[test]
    fn items_reject_model_and_effort_keys() {
        for doc in [
            r#"[{"id":"a","model":"codex:gpt-5.5"}]"#,
            r#"[{"id":"a","model":"tier:high"}]"#,
            r#"[{"id":"a","effort":"high"}]"#,
        ] {
            let err = parse_items(doc).unwrap_err();
            assert!(err.to_string().contains("give the item a `tier`"), "{doc}: {err}");
        }
    }
}
