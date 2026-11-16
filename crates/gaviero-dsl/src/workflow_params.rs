//! Workflow parameter materialization: list params and roster expansion,
//! plus the workflow `tiers { }` block.
//!
//! Roster params expand `loop { reviewers ... }` into per-reviewer agents.
//! A model is chosen by tier only: an agent declares `tier <name>`, the
//! workflow's `tiers { agent tier }` block and `--tier agent=tier` replace it,
//! and a list item's `tier` replaces it for that item's clone.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{DslError, DslErrors};

/// Apply the selected workflow's `tiers { }` block, then the `--tier`
/// overrides, to the agents' declared tiers. Runs before any cloning, so
/// `for` and roster clones inherit the result; an item's own `tier` still
/// wins for its clone.
pub(crate) fn apply_agent_tiers(
    script: &mut Script,
    workflow_name: Option<&str>,
    cli_tiers: &[(String, String)],
) -> Result<(), DslErrors> {
    let block: Vec<AgentTier> = script
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Workflow(wf) if workflow_name.is_none_or(|n| wf.name == n) => {
                Some(wf.tiers.clone())
            }
            _ => None,
        })
        .flatten()
        .collect();
    let agents: HashSet<String> = script
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Agent(a) => Some(a.name.clone()),
            _ => None,
        })
        .collect();

    let mut errors = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut chosen: HashMap<String, (String, Span)> = HashMap::new();
    for entry in &block {
        let span = entry.agent_span;
        let at = (span.start, span.end.saturating_sub(span.start).max(1));
        if !agents.contains(&entry.agent) {
            errors.push(DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: at.into(),
                reason: format!("`tiers` names `{}`, which is not a defined agent", entry.agent),
            });
        } else if !seen.insert(entry.agent.as_str()) {
            errors.push(DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: at.into(),
                reason: format!("`tiers` sets agent `{}` twice", entry.agent),
            });
        }
        chosen.insert(entry.agent.clone(), (entry.tier.clone(), entry.tier_span));
    }
    for (agent, tier) in cli_tiers {
        if !agents.contains(agent) {
            errors.push(DslError::Compile {
                src: miette::NamedSource::new("<cli>", String::new()),
                span: (0, 1).into(),
                reason: format!("--tier {agent}={tier}: `{agent}` is not a defined agent"),
            });
            continue;
        }
        // A CLI tier has no place in the source; keep the agent's own span so
        // an unknown tier is reported on the agent.
        let span = chosen
            .get(agent)
            .map(|(_, s)| *s)
            .unwrap_or_else(|| (0..0).into());
        chosen.insert(agent.clone(), (tier.clone(), span));
    }
    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }

    for item in &mut script.items {
        let Item::Agent(agent) = item else { continue };
        if let Some((tier, span)) = chosen.get(&agent.name) {
            let span = if span.start == span.end {
                agent.tier_ref.as_ref().map(|(_, s)| *s).unwrap_or(agent.name_span)
            } else {
                *span
            };
            agent.tier_ref = Some((tier.clone(), span));
        }
    }
    Ok(())
}

/// Parse a `--param NAME=...` value string into a roster / list param.
///
/// Forms:
/// - a DSL list literal: `[{id "a" focus "x"} {id "b" tier high}]`
///   (any value whose trimmed form starts with `[`);
/// - the short form `id=tier(,id=tier)*`, where a part without `=` is an
///   id-only entry (`a,b,c`).
pub fn parse_reviewers_override(value: &str) -> Result<Vec<ReviewerEntry>, String> {
    if value.trim_start().starts_with('[') {
        return crate::parser::parse_list_literal(value);
    }
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let span: chumsky::span::SimpleSpan = (0..0).into();
        let (id, tier) = match part.split_once('=') {
            Some((id, tier)) => (id.trim(), Some(tier.trim())),
            None => (part, None),
        };
        if id.is_empty() {
            return Err(format!("invalid roster entry `{part}` (empty id)"));
        }
        if let Some(tier) = tier
            && (tier.is_empty() || tier.contains([':', '@', ' ']))
        {
            return Err(format!(
                "invalid roster entry `{part}` (expected `id=<tier>`, e.g. `claude=top-claude`; \
                 models are chosen by the profile's tiers)"
            ));
        }
        out.push(ReviewerEntry {
            id: id.to_string(),
            id_span: span,
            tier: tier.map(|t| (t.to_string(), span)),
            fields: Vec::new(),
            span,
        });
    }
    if out.is_empty() {
        return Err("roster override is empty".into());
    }
    Ok(out)
}

/// One workflow's params as a caller sees them: kind and default in
/// `--param` syntax. A param that cannot be classified is left out; compiling
/// reports it with a diagnostic.
pub(crate) fn describe_workflow_params(
    script: &Script,
    wf: &WorkflowDecl,
) -> Vec<crate::inspect::ParamInspection> {
    use crate::inspect::{ParamInspection, ParamKindInfo};

    let agent_by_name: HashMap<String, AgentDecl> = script
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Agent(a) => Some((a.name.clone(), a.clone())),
            _ => None,
        })
        .collect();
    let usage = param_usage_for_workflow(wf, &agent_by_name);

    wf.params
        .iter()
        .filter_map(|decl| {
            let mut ignored = Vec::new();
            let ParamKind::Roster(entries) =
                resolve_param_kind(&decl.name, decl, &usage, &mut ignored)?;
            Some(ParamInspection {
                name: decl.name.clone(),
                kind: ParamKindInfo::Roster,
                default: entries.filter(|e| !e.is_empty()).map(|entries| {
                    // Free fields only survive in the literal form.
                    if entries.iter().any(|r| !r.fields.is_empty()) {
                        let items: Vec<String> = entries.iter().map(list_literal_entry).collect();
                        return format!("[{}]", items.join(" "));
                    }
                    entries
                        .iter()
                        .map(|r| match &r.tier {
                            Some((tier, _)) => format!("{}={tier}", r.id),
                            None => r.id.clone(),
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                }),
            })
        })
        .collect()
}

/// One entry in list-literal syntax: `{id "a" tier high focus "x"}`.
pub fn list_literal_entry(entry: &ReviewerEntry) -> String {
    let mut out = format!("{{id {}", quote_literal(&entry.id));
    if let Some((tier, _)) = &entry.tier {
        out.push_str(&format!(" tier {tier}"));
    }
    for (name, value, _) in &entry.fields {
        out.push_str(&format!(" {name} {}", quote_literal(value)));
    }
    out.push('}');
    out
}

/// A DSL string literal holding `value` verbatim.
fn quote_literal(value: &str) -> String {
    if value.contains(['"', '\\', '\n', '\r']) {
        format!("#\"{value}\"#")
    } else {
        format!("\"{value}\"")
    }
}

/// Resolve list params and expand reviewer rosters for the selected workflow.
pub fn expand_workflow_params_in_script(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_params: &[(String, String)],
) -> Result<(), DslErrors> {
    let mut errors = Vec::new();
    let mut new_agents: Vec<AgentDecl> = Vec::new();

    let agent_by_name: HashMap<String, AgentDecl> = script
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Agent(a) = i {
                Some((a.name.clone(), a.clone()))
            } else {
                None
            }
        })
        .collect();

    // Per-workflow resolved param kinds (used by roster pass).
    let mut kinds_by_workflow: HashMap<String, HashMap<String, ParamKind>> = HashMap::new();

    // Pass 1 — resolve param kinds (immutable workflow borrow).
    for item in &script.items {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name
            && wf.name != name
        {
            continue;
        }

        let usage = param_usage_for_workflow(wf, &agent_by_name);
        let params_by_name: HashMap<String, ParamDecl> = wf
            .params
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();

        let mut resolved_kinds: HashMap<String, ParamKind> = HashMap::new();
        for (pname, decl) in &params_by_name {
            if let Some(kind) = resolve_param_kind(pname, decl, &usage, &mut errors) {
                resolved_kinds.insert(pname.clone(), kind);
            }
        }

        if resolved_kinds.is_empty() {
            continue;
        }

        kinds_by_workflow.insert(wf.name.clone(), resolved_kinds);
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }

    // Pass 2 — roster expansion (mutate workflow steps).
    for item in script.items.iter_mut() {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name
            && wf.name != name
        {
            continue;
        }

        let empty_kinds = HashMap::new();
        let resolved_kinds = kinds_by_workflow.get(&wf.name).unwrap_or(&empty_kinds);
        let params_by_name: HashMap<String, ParamDecl> = wf
            .params
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();

        let Some((steps, _)) = wf.steps.as_mut() else {
            continue;
        };

        let mut expanded_steps: Vec<StepItem> = Vec::new();
        for step in steps.drain(..) {
            match step {
                StepItem::Loop(mut lb) if !lb.reviewers.is_none() => {
                    let roster = match resolve_roster(
                        &lb.reviewers,
                        &params_by_name,
                        resolved_kinds,
                        override_params,
                    ) {
                        Ok(r) => r,
                        Err(err) => {
                            errors.push(err);
                            lb.reviewers = ReviewerSource::None;
                            expanded_steps.push(StepItem::Loop(lb));
                            continue;
                        }
                    };

                    let init_template = lb.template_init.as_ref().map(|(name, span)| {
                        match agent_by_name.get(name) {
                            Some(a) => Ok((a, name.clone(), *span)),
                            None => {
                                errors.push(DslError::Compile {
                                    src: miette::NamedSource::new("script", String::new()),
                                    span: (span.start, span.end.saturating_sub(span.start).max(1))
                                        .into(),
                                    reason: format!(
                                        "template_init `{name}` is not a defined agent"
                                    ),
                                });
                                Err(())
                            }
                        }
                    });
                    if init_template.as_ref().is_some_and(|r| r.is_err()) {
                        lb.reviewers = ReviewerSource::None;
                        expanded_steps.push(StepItem::Loop(lb));
                        continue;
                    }
                    let init_template = init_template.and_then(|r| r.ok());

                    let (refine_name, refine_span) = match &lb.template_refine {
                        Some(t) => t.clone(),
                        None => {
                            errors.push(DslError::Compile {
                                src: miette::NamedSource::new("script", String::new()),
                                span: (lb.span.start, 1).into(),
                                reason: "loop with `reviewers` requires `template_refine <agent>`"
                                    .into(),
                            });
                            continue;
                        }
                    };

                    let refine_tpl = match agent_by_name.get(&refine_name) {
                        Some(a) => a,
                        None => {
                            errors.push(DslError::Compile {
                                src: miette::NamedSource::new("script", String::new()),
                                span: (
                                    refine_span.start,
                                    refine_span.end.saturating_sub(refine_span.start).max(1),
                                )
                                    .into(),
                                reason: format!(
                                    "template_refine `{}` is not a defined agent",
                                    refine_name
                                ),
                            });
                            continue;
                        }
                    };

                    let peer_ids: Vec<String> = roster.iter().map(|r| r.id.clone()).collect();
                    let mut refine_agent_names: Vec<(String, chumsky::span::SimpleSpan)> =
                        Vec::new();

                    for entry in &roster {
                        if let Some((init_tpl, _, _)) = &init_template {
                            let init_agent = clone_reviewer_agent(
                                init_tpl,
                                &format!("{}-init", entry.id),
                                entry,
                            );
                            new_agents.push(init_agent.clone());
                            expanded_steps.push(StepItem::Agent(
                                init_agent.name.clone(),
                                init_agent.name_span,
                            ));
                        }

                        let refine_agent = clone_reviewer_agent(
                            refine_tpl,
                            &format!("{}-refine", entry.id),
                            entry,
                        );
                        let refine_agent = inject_peer_vars(refine_agent, &entry.id, &peer_ids);
                        refine_agent_names
                            .push((refine_agent.name.clone(), refine_agent.name_span));
                        new_agents.push(refine_agent);
                    }

                    lb.agents = refine_agent_names;
                    lb.reviewers = ReviewerSource::Literal(roster);
                    expanded_steps.push(StepItem::Loop(lb));
                }
                other => expanded_steps.push(other),
            }
        }
        *steps = expanded_steps;
    }

    for agent in new_agents {
        script.items.push(Item::Agent(agent));
    }

    // Pass 3 — `for` blocks (explicit parallel / sequential iteration).
    if errors.is_empty() {
        crate::for_loops::expand_for_blocks(
            script,
            workflow_name,
            override_params,
            &kinds_by_workflow,
            &mut errors,
        );
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    Ok(())
}

/// Backward-compatible alias used by the compiler.
pub fn expand_reviewers_in_script(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_params: &[(String, String)],
) -> Result<(), DslErrors> {
    expand_workflow_params_in_script(script, workflow_name, override_params)
}

#[derive(Clone, Default)]
struct ParamUsage {
    roster: bool,
}

fn param_usage_for_workflow(
    wf: &WorkflowDecl,
    _agent_by_name: &HashMap<String, AgentDecl>,
) -> HashMap<String, ParamUsage> {
    let mut usage: HashMap<String, ParamUsage> = HashMap::new();
    let Some((steps, _)) = &wf.steps else {
        return usage;
    };
    for step in steps {
        match step {
            StepItem::Loop(lb) => {
                if let ReviewerSource::ParamRef(name, _) = &lb.reviewers {
                    usage.entry(name.clone()).or_default().roster = true;
                }
                for fb in lb.fors.iter().filter(|fb| fb.json_source.is_none()) {
                    usage.entry(fb.list.clone()).or_default().roster = true;
                }
            }
            StepItem::Agent(..) => {}
            // A `for` iterates a list param, which is the roster kind.
            // (`for … in json` reads a file at run time and uses no param.)
            StepItem::For(fb) if fb.json_source.is_none() => {
                usage.entry(fb.list.clone()).or_default().roster = true;
            }
            StepItem::For(_) => {}
        }
    }
    usage
}

fn resolve_param_kind(
    name: &str,
    decl: &ParamDecl,
    usage: &HashMap<String, ParamUsage>,
    errors: &mut Vec<DslError>,
) -> Option<ParamKind> {
    if let Some(ParamShape::Roster(list)) = &decl.shape {
        return Some(ParamKind::Roster(Some(list.clone())));
    }
    if usage.get(name).is_some_and(|u| u.roster) {
        return Some(ParamKind::Roster(None));
    }
    errors.push(DslError::Compile {
        src: miette::NamedSource::new("script", String::new()),
        span: (
            decl.name_span.start,
            decl.name_span
                .end
                .saturating_sub(decl.name_span.start)
                .max(1),
        )
            .into(),
        reason: format!(
            "workflow param `{name}` is never referenced — iterate it with \
             `for x in {name}` or `reviewers {name}`, or remove the declaration \
             (params are lists; an agent's model comes from its `tier`)"
        ),
    });
    None
}

pub(crate) fn resolve_roster(
    source: &ReviewerSource,
    params_by_name: &HashMap<String, ParamDecl>,
    resolved_kinds: &HashMap<String, ParamKind>,
    override_params: &[(String, String)],
) -> Result<Vec<ReviewerEntry>, DslError> {
    match source {
        ReviewerSource::None => Ok(Vec::new()),
        ReviewerSource::Literal(list) => Ok(list.clone()),
        ReviewerSource::ParamRef(name, span) => {
            let decl = params_by_name.get(name).ok_or_else(|| DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
                reason: format!(
                    "`reviewers {name}` references an undeclared workflow parameter; \
                     add `param {name}` to the workflow header"
                ),
            })?;

            let kind = resolved_kinds.get(name).ok_or_else(|| DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
                reason: format!("workflow param `{name}` is not a roster param"),
            })?;

            let ParamKind::Roster(default) = kind;

            if let Some((_, v)) = override_params.iter().find(|(k, _)| k == name) {
                return parse_reviewers_override(v).map_err(|e| DslError::Compile {
                    src: miette::NamedSource::new("<cli>", String::new()),
                    span: (0, 1).into(),
                    reason: format!("--param {name}: {e}"),
                });
            }

            match default {
                Some(list) => Ok(list.clone()),
                None => Err(DslError::Compile {
                    src: miette::NamedSource::new("script", String::new()),
                    span: (
                        decl.name_span.start,
                        decl.name_span
                            .end
                            .saturating_sub(decl.name_span.start)
                            .max(1),
                    )
                        .into(),
                    reason: format!(
                        "workflow roster param `{name}` has no default and was not \
                         supplied on the CLI — pass `--param {name}=id=<tier>,...`"
                    ),
                }),
            }
        }
    }
}

fn clone_reviewer_agent(template: &AgentDecl, name: &str, entry: &ReviewerEntry) -> AgentDecl {
    let mut vars = template.vars.clone();
    vars.retain(|(k, _)| k != "REVIEWER_ID");
    vars.push(("REVIEWER_ID".into(), entry.id.clone()));

    let mut agent = AgentDecl {
        name: name.to_string(),
        name_span: template.name_span,
        description: template.description.clone(),
        tier_ref: entry.tier.clone().or_else(|| template.tier_ref.clone()),
        vars,
        template: false,
        ..clone_agent_shell(template)
    };
    if let Some(scope) = agent.scope.as_mut() {
        for path in scope.owned.iter_mut().chain(scope.read_only.iter_mut()) {
            *path = path.replace("{{REVIEWER_ID}}", &entry.id);
        }
    }
    // The output contract is per-reviewer for the same reason the owned globs
    // are: without the id prefix every clone would claim the same artefacts.
    for path in agent.produces.iter_mut().chain(agent.produces_json.iter_mut()) {
        *path = path.replace("{{REVIEWER_ID}}", &entry.id);
    }
    agent
}

fn clone_agent_shell(template: &AgentDecl) -> AgentDecl {
    AgentDecl {
        name: String::new(),
        name_span: template.name_span,
        description: None,
        tier_ref: template.tier_ref.clone(),
        scope: template.scope.clone(),
        depends_on: template.depends_on.clone(),
        produces: template.produces.clone(),
        produces_json: template.produces_json.clone(),
        prompt: template.prompt.clone(),
        max_retries: template.max_retries,
        timeout_secs: template.timeout_secs,
        memory: template.memory.clone(),
        context: template.context.clone(),
        vars: Vec::new(),
        item_vars: Vec::new(),
        tools: template.tools.clone(),
        template: false,
        span: template.span,
        file_id: template.file_id,
    }
}

fn inject_peer_vars(mut agent: AgentDecl, self_id: &str, peer_ids: &[String]) -> AgentDecl {
    let others: Vec<&str> = peer_ids
        .iter()
        .map(String::as_str)
        .filter(|id| *id != self_id)
        .collect();
    // Item vars are substituted before every other layer, so the
    // `{{OUT_DIR}}` / `{{PREV_ITER}}` placeholders inside the peer block are
    // still filled in by the script / CLI vars and the runtime.
    agent
        .item_vars
        .push(("REVIEWER_ID".into(), self_id.to_string()));
    agent.item_vars.push(("PEER_IDS".into(), others.join(",")));
    agent
        .item_vars
        .push(("PEER_COUNT".into(), others.len().to_string()));
    agent
        .item_vars
        .push(("PEER_READ_BLOCK".into(), build_peer_read_block(&others)));
    agent
}

fn build_peer_read_block(peers: &[&str]) -> String {
    let mut s =
        String::from("    Peer documents to read (all other providers for this iteration):\n");
    for peer in peers {
        // `{{{{OUT_DIR}}}}` is the format-string escape for a literal
        // `{{OUT_DIR}}` — the double-brace form the var substituter looks for.
        // Emitting `{{OUT_DIR}}` here would produce a single-braced
        // `{OUT_DIR}` that survives substitution and reaches the agent as a
        // literal path segment.
        s.push_str(&format!(
            "      - {{{{OUT_DIR}}}}/{peer}-conclusion-v{{{{PREV_ITER}}}}.md\n"
        ));
        s.push_str(&format!(
            "      - {{{{OUT_DIR}}}}/{peer}-summary-v{{{{PREV_ITER}}}}.md (skip if the file does not exist)\n"
        ));
    }
    if peers.is_empty() {
        s.push_str("      (none)\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier_of(entry: &ReviewerEntry) -> Option<&str> {
        entry.tier.as_ref().map(|(t, _)| t.as_str())
    }

    #[test]
    fn parse_simple_two_reviewers() {
        let entries = parse_reviewers_override("claude=top-claude,codex=top-codex").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "claude");
        assert_eq!(tier_of(&entries[0]), Some("top-claude"));
        assert_eq!(tier_of(&entries[1]), Some("top-codex"));
    }

    #[test]
    fn parse_list_literal_override() {
        let entries =
            parse_reviewers_override(r#"[{id "a" focus "x"} {id "b" tier high}]"#).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].fields[0].1, "x");
        assert_eq!(tier_of(&entries[1]), Some("high"));
    }

    #[test]
    fn parse_id_only_short_form() {
        let entries = parse_reviewers_override("a,b").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "a");
        assert!(entries[0].tier.is_none());
        assert_eq!(entries[1].id, "b");
    }

    /// Models are a profile's business: a roster entry names a tier, never
    /// a `provider:model`, an effort or a `tier:` spec.
    #[test]
    fn parse_rejects_model_specs() {
        for value in ["claude=claude:opus", "claude=opus@max", "claude=tier:high"] {
            let err = parse_reviewers_override(value).unwrap_err();
            assert!(err.contains("id=<tier>"), "{value}: {err}");
        }
    }

    #[test]
    fn a_list_literal_rejects_model_and_effort() {
        assert!(parse_reviewers_override(r#"[{id "a" model "claude:opus"}]"#).is_err());
        assert!(parse_reviewers_override(r#"[{id "a" effort high}]"#).is_err());
    }

    /// The block is substituted like any other prompt text, so its
    /// placeholders must survive `format!` in the double-brace form. A
    /// single-braced `{OUT_DIR}` reaches the agent verbatim and sends it
    /// looking for a directory literally named `{OUT_DIR}`.
    #[test]
    fn peer_read_block_emits_substitutable_placeholders() {
        let block = build_peer_read_block(&["codex", "cursor"]);
        assert_eq!(
            block.matches("{{OUT_DIR}}").count(),
            4,
            "expected one {{{{OUT_DIR}}}} per peer file, got: {block}"
        );
        assert_eq!(block.matches("{{PREV_ITER}}").count(), 4, "got: {block}");
        assert!(
            !block.contains("{OUT_DIR}/"),
            "single-braced placeholder leaked: {block}"
        );
    }

    /// Both bullets belong to the same indented list; a line-continuation
    /// escape silently ate the second one's indentation.
    #[test]
    fn peer_read_block_indents_every_bullet() {
        let block = build_peer_read_block(&["codex"]);
        let bullets: Vec<&str> = block
            .lines()
            .filter(|l| l.trim().starts_with('-'))
            .collect();
        assert_eq!(bullets.len(), 2, "got: {block}");
        for line in bullets {
            assert!(line.starts_with("      - "), "unindented bullet: {line:?}");
        }
    }

    #[test]
    fn peer_read_block_handles_solo_panel() {
        let block = build_peer_read_block(&[]);
        assert!(block.contains("(none)"), "got: {block}");
    }
}
