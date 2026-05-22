//! Workflow parameter materialization: client params and roster expansion.
//!
//! Client params (`param judge { model "claude:sonnet" ... }` or bare `param judge`
//! when an agent says `client judge`) synthesize `__param_<name>` clients and
//! rewrite agent bindings before compilation.
//!
//! Roster params expand `loop { reviewers ... }` into per-reviewer agents.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{DslError, DslErrors};

/// Synthesized client name for a workflow client param.
pub fn param_client_name(param: &str) -> String {
    format!("__param_{param}")
}

/// Parse `--param NAME=provider:model[@effort]` for a client param.
pub fn parse_client_override(value: &str) -> Result<ClientParamSpec, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("client param override is empty".into());
    }
    let (model_spec, effort) = match value.split_once('@') {
        Some((m, e)) => (m.trim().to_string(), Some(e.trim().to_string())),
        None => (value.to_string(), None),
    };
    if model_spec.is_empty() || !model_spec.contains(':') {
        return Err(format!(
            "client param override must be `provider:model` or `provider:model@effort`, got `{value}`"
        ));
    }
    let span: chumsky::span::SimpleSpan = (0..0).into();
    Ok(ClientParamSpec {
        model: Some((model_spec, span)),
        effort: effort.map(|e| (e, span)),
        privacy: None,
        extra: Vec::new(),
        span,
    })
}

/// Parse a `--param NAME=...` value string into a roster.
///
/// Format: `id=provider:model[@effort](,id=provider:model[@effort])*`.
pub fn parse_reviewers_override(value: &str) -> Result<Vec<ReviewerEntry>, String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (id, rest) = part
            .split_once('=')
            .ok_or_else(|| format!("invalid roster entry `{part}` (expected id=provider:model)"))?;
        let id = id.trim();
        if id.is_empty() {
            return Err(format!("invalid roster entry `{part}` (empty id)"));
        }
        let (model_spec, effort) = match rest.split_once('@') {
            Some((m, e)) => (m.trim().to_string(), Some(e.trim().to_string())),
            None => (rest.trim().to_string(), None),
        };
        if model_spec.is_empty() || !model_spec.contains(':') {
            return Err(format!(
                "invalid roster entry `{part}` (model must be `provider:model`)"
            ));
        }
        let span: chumsky::span::SimpleSpan = (0..0).into();
        out.push(ReviewerEntry {
            id: id.to_string(),
            id_span: span,
            model: model_spec,
            model_span: span,
            effort: effort.map(|e| (e, span)),
            span,
        });
    }
    if out.is_empty() {
        return Err("roster override is empty".into());
    }
    Ok(out)
}

/// Materialize client params and expand reviewer rosters for the selected workflow.
pub fn expand_workflow_params_in_script(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_params: &[(String, String)],
) -> Result<(), DslErrors> {
    let mut errors = Vec::new();
    let mut new_clients: Vec<ClientDecl> = Vec::new();
    let mut new_agents: Vec<AgentDecl> = Vec::new();

    let static_client_names: HashSet<String> = script
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Client(c) = i {
                Some(c.name.clone())
            } else {
                None
            }
        })
        .collect();

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
    let mut client_rewrites: Vec<(String, String)> = Vec::new();

    // Pass 1 — materialize client params (immutable workflow borrow).
    for item in &script.items {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name {
            if wf.name != name {
                continue;
            }
        }

        let usage = param_usage_for_workflow(wf, &agent_by_name);
        let params_by_name: HashMap<String, ParamDecl> = wf
            .params
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();

        let mut resolved_kinds: HashMap<String, ParamKind> = HashMap::new();
        for (pname, decl) in &params_by_name {
            match resolve_param_kind(pname, decl, &usage, &mut errors) {
                Some(kind) => {
                    resolved_kinds.insert(pname.clone(), kind);
                }
                None => {}
            }
        }

        if resolved_kinds.is_empty() {
            continue;
        }

        kinds_by_workflow.insert(wf.name.clone(), resolved_kinds.clone());

        for (param_name, kind) in &resolved_kinds {
            let ParamKind::Client(spec) = kind else { continue };
            if static_client_names.contains(param_name) {
                errors.push(DslError::Compile {
                    src: miette::NamedSource::new("script", String::new()),
                    span: (
                        params_by_name[param_name].name_span.start,
                        params_by_name[param_name]
                            .name_span
                            .end
                            .saturating_sub(params_by_name[param_name].name_span.start)
                            .max(1),
                    )
                        .into(),
                    reason: format!(
                        "workflow param `{param_name}` conflicts with top-level `client {param_name}` — \
                         use a different param or client name"
                    ),
                });
                continue;
            }

            let resolved = match resolve_client_spec(
                param_name,
                spec,
                &params_by_name[param_name],
                override_params,
            ) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };

            let synth_name = param_client_name(param_name);
            new_clients.push(synth_client_from_spec(&synth_name, &resolved));
            client_rewrites.push((param_name.clone(), synth_name));
        }
    }

    for (param_name, synth_name) in &client_rewrites {
        rewrite_agent_client_param(script, param_name, synth_name);
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }

    // Pass 2 — roster expansion (mutate workflow steps).
    for item in script.items.iter_mut() {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name {
            if wf.name != name {
                continue;
            }
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
                        &resolved_kinds,
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

                    let (init_name, init_span) = match &lb.template_init {
                        Some(t) => t.clone(),
                        None => {
                            errors.push(DslError::Compile {
                                src: miette::NamedSource::new("script", String::new()),
                                span: (lb.span.start, 1).into(),
                                reason: "loop with `reviewers` requires `template_init <agent>`"
                                    .into(),
                            });
                            continue;
                        }
                    };
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

                    let init_tpl = match agent_by_name.get(&init_name) {
                        Some(a) => a,
                        None => {
                            errors.push(DslError::Compile {
                                src: miette::NamedSource::new("script", String::new()),
                                span: (
                                    init_span.start,
                                    init_span.end.saturating_sub(init_span.start).max(1),
                                )
                                .into(),
                                reason: format!(
                                    "template_init `{}` is not a defined agent",
                                    init_name
                                ),
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
                        let init_client_name = format!("__roster_{}_init", entry.id);
                        let refine_client_name = format!("__roster_{}_refine", entry.id);
                        new_clients.push(synth_client_from_entry(&init_client_name, entry));
                        new_clients.push(synth_client_from_entry(&refine_client_name, entry));

                        let init_agent = clone_reviewer_agent(
                            init_tpl,
                            &format!("{}-init", entry.id),
                            entry,
                            &init_client_name,
                        );
                        new_agents.push(init_agent.clone());
                        expanded_steps.push(StepItem::Agent(
                            init_agent.name.clone(),
                            init_agent.name_span,
                        ));

                        let refine_agent = clone_reviewer_agent(
                            refine_tpl,
                            &format!("{}-refine", entry.id),
                            entry,
                            &refine_client_name,
                        );
                        let refine_agent =
                            inject_peer_vars(refine_agent, &entry.id, &peer_ids);
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

    for client in new_clients {
        script.items.push(Item::Client(client));
    }
    for agent in new_agents {
        script.items.push(Item::Agent(agent));
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
    client: bool,
}

fn param_usage_for_workflow(
    wf: &WorkflowDecl,
    agent_by_name: &HashMap<String, AgentDecl>,
) -> HashMap<String, ParamUsage> {
    let param_names: HashSet<String> = wf.params.iter().map(|p| p.name.clone()).collect();
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
                if let UntilCondition::Agent(judge, _) = &lb.until {
                    record_client_param_usage(&mut usage, judge, &param_names, agent_by_name);
                }
            }
            StepItem::Agent(name, _) => {
                record_client_param_usage(&mut usage, name, &param_names, agent_by_name);
            }
        }
    }
    usage
}

fn record_client_param_usage(
    usage: &mut HashMap<String, ParamUsage>,
    agent_name: &str,
    param_names: &HashSet<String>,
    agent_by_name: &HashMap<String, AgentDecl>,
) {
    let Some(agent) = agent_by_name.get(agent_name) else {
        return;
    };
    if let Some((client_name, _)) = &agent.client {
        if param_names.contains(client_name) {
            usage.entry(client_name.clone()).or_default().client = true;
        }
    }
}

fn resolve_param_kind(
    name: &str,
    decl: &ParamDecl,
    usage: &HashMap<String, ParamUsage>,
    errors: &mut Vec<DslError>,
) -> Option<ParamKind> {
    if let Some(shape) = &decl.shape {
        return Some(match shape {
            ParamShape::Roster(list) => ParamKind::Roster(Some(list.clone())),
            ParamShape::Client(spec) => ParamKind::Client(spec.clone()),
        });
    }

    let u = usage.get(name).cloned().unwrap_or_default();
    match (u.roster, u.client) {
        (true, true) => {
            errors.push(DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: (
                    decl.name_span.start,
                    decl.name_span.end.saturating_sub(decl.name_span.start).max(1),
                )
                .into(),
                reason: format!(
                    "workflow param `{name}` is used as both a roster (`reviewers {name}`) \
                     and a client (`client {name}`) — use distinct param names or add \
                     explicit `[...]` / `{{...}}` syntax"
                ),
            });
            None
        }
        (true, false) => Some(ParamKind::Roster(None)),
        (false, true) => Some(ParamKind::Client(ClientParamSpec {
            span: decl.span,
            ..Default::default()
        })),
        (false, false) => {
            errors.push(DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: (
                    decl.name_span.start,
                    decl.name_span.end.saturating_sub(decl.name_span.start).max(1),
                )
                .into(),
                reason: format!(
                    "workflow param `{name}` is never referenced — use `reviewers {name}`, \
                     `client {name}` on an agent, or remove the declaration"
                ),
            });
            None
        }
    }
}

fn resolve_client_spec(
    name: &str,
    spec: &ClientParamSpec,
    decl: &ParamDecl,
    override_params: &[(String, String)],
) -> Result<ClientParamSpec, DslError> {
    if let Some((_, v)) = override_params.iter().find(|(k, _)| k == name) {
        return parse_client_override(v).map_err(|e| DslError::Compile {
            src: miette::NamedSource::new("<cli>", String::new()),
            span: (0, 1).into(),
            reason: format!("--param {name}: {e}"),
        });
    }

    if spec.model.is_some() {
        return Ok(spec.clone());
    }

    Err(DslError::Compile {
        src: miette::NamedSource::new("script", String::new()),
        span: (
            decl.name_span.start,
            decl.name_span.end.saturating_sub(decl.name_span.start).max(1),
        )
        .into(),
        reason: format!(
            "workflow client param `{name}` has no `model` default and was not \
             supplied on the CLI — pass `--param {name}=provider:model[@effort]`"
        ),
    })
}

fn resolve_roster(
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

            let ParamKind::Roster(default) = kind else {
                return Err(DslError::Compile {
                    src: miette::NamedSource::new("script", String::new()),
                    span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
                    reason: format!(
                        "`reviewers {name}` requires a roster param (`param {name} [ ... ]` \
                         or bare `param {name}` used only in `reviewers`)"
                    ),
                });
            };

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
                        decl.name_span.end.saturating_sub(decl.name_span.start).max(1),
                    )
                    .into(),
                    reason: format!(
                        "workflow roster param `{name}` has no default and was not \
                         supplied on the CLI — pass `--param {name}=id=provider:model[@effort],...`"
                    ),
                }),
            }
        }
    }
}

fn synth_client_from_spec(name: &str, spec: &ClientParamSpec) -> ClientDecl {
    let span = spec.span;
    ClientDecl {
        name: name.to_string(),
        name_span: span,
        tier: None,
        model: spec.model.clone(),
        effort: spec.effort.clone(),
        extra: spec.extra.clone(),
        privacy: spec
            .privacy
            .clone()
            .or(Some((PrivacyLit::Public, span))),
        is_default: false,
        span,
        file_id: 0,
    }
}

fn synth_client_from_entry(name: &str, entry: &ReviewerEntry) -> ClientDecl {
    ClientDecl {
        name: name.to_string(),
        name_span: entry.span,
        tier: None,
        model: Some((entry.model.clone(), entry.model_span)),
        effort: entry.effort.clone(),
        extra: Vec::new(),
        privacy: Some((PrivacyLit::Public, entry.span)),
        is_default: false,
        span: entry.span,
        file_id: 0,
    }
}

fn clone_reviewer_agent(
    template: &AgentDecl,
    name: &str,
    entry: &ReviewerEntry,
    client_name: &str,
) -> AgentDecl {
    let mut vars = template.vars.clone();
    vars.retain(|(k, _)| k != "REVIEWER_ID");
    vars.push(("REVIEWER_ID".into(), entry.id.clone()));

    let mut agent = AgentDecl {
        name: name.to_string(),
        name_span: template.name_span,
        description: template.description.clone(),
        client: Some((client_name.to_string(), entry.span)),
        tier_ref: None,
        vars,
        template: false,
        ..clone_agent_shell(template)
    };
    if let Some(scope) = agent.scope.as_mut() {
        for path in scope.owned.iter_mut().chain(scope.read_only.iter_mut()) {
            *path = path.replace("{{REVIEWER_ID}}", &entry.id);
        }
    }
    agent
}

fn clone_agent_shell(template: &AgentDecl) -> AgentDecl {
    AgentDecl {
        name: String::new(),
        name_span: template.name_span,
        description: None,
        client: None,
        tier_ref: None,
        scope: template.scope.clone(),
        depends_on: template.depends_on.clone(),
        prompt: template.prompt.clone(),
        max_retries: template.max_retries,
        memory: template.memory.clone(),
        context: template.context.clone(),
        vars: Vec::new(),
        tools: template.tools.clone(),
        template: false,
        span: template.span,
        file_id: template.file_id,
    }
}

fn rewrite_agent_client_param(script: &mut Script, param_name: &str, synth_name: &str) {
    for item in &mut script.items {
        let Item::Agent(agent) = item else { continue };
        if agent
            .client
            .as_ref()
            .is_some_and(|(n, _)| n == param_name)
        {
            let span = agent.client.as_ref().unwrap().1;
            agent.client = Some((synth_name.to_string(), span));
        }
    }
}

fn inject_peer_vars(mut agent: AgentDecl, self_id: &str, peer_ids: &[String]) -> AgentDecl {
    let others: Vec<&str> = peer_ids
        .iter()
        .map(String::as_str)
        .filter(|id| *id != self_id)
        .collect();
    agent.vars.push(("REVIEWER_ID".into(), self_id.to_string()));
    agent.vars.push(("PEER_IDS".into(), others.join(",")));
    agent.vars.push(("PEER_COUNT".into(), others.len().to_string()));
    agent
        .vars
        .push(("PEER_READ_BLOCK".into(), build_peer_read_block(&others)));
    agent
}

fn build_peer_read_block(peers: &[&str]) -> String {
    let mut s = String::from(
        "    Peer documents to read (all other providers for this iteration):\n",
    );
    for peer in peers {
        s.push_str(&format!(
            "      - {{OUT_DIR}}/{peer}-conclusion-v{{{{PREV_ITER}}}}.md (or -init- on first refine pass)\n\
               - {{OUT_DIR}}/{peer}-summary-v{{{{PREV_ITER}}}}.md (omit on PREV_ITER=1)\n"
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

    #[test]
    fn parse_client_override_with_effort() {
        let spec = parse_client_override("claude:sonnet@medium").unwrap();
        assert_eq!(spec.model.as_ref().unwrap().0, "claude:sonnet");
        assert_eq!(spec.effort.as_ref().unwrap().0, "medium");
    }

    #[test]
    fn parse_client_override_rejects_bare_model() {
        let err = parse_client_override("opus").unwrap_err();
        assert!(err.contains("provider:model"), "got: {err}");
    }

    #[test]
    fn parse_simple_two_reviewers() {
        let entries =
            parse_reviewers_override("claude=claude:opus,codex=codex:gpt-5.5").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "claude");
        assert_eq!(entries[0].model, "claude:opus");
    }

    #[test]
    fn parse_with_effort() {
        let entries =
            parse_reviewers_override("claude=claude:opus@max,codex=codex:gpt-5.5@high")
                .unwrap();
        assert_eq!(entries[0].effort.as_ref().unwrap().0, "max");
        assert_eq!(entries[1].effort.as_ref().unwrap().0, "high");
    }

    #[test]
    fn parse_rejects_missing_provider() {
        let err = parse_reviewers_override("claude=opus").unwrap_err();
        assert!(err.contains("provider:model"), "got: {err}");
    }
}
