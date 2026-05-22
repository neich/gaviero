//! Roster expansion: turn a `loop { reviewers ... template_init T template_refine R }`
//! block into per-reviewer clones of T and R, each bound to a synthesised
//! anonymous client carrying the entry's model + effort.
//!
//! Roster sources:
//! - `ReviewerSource::Literal(list)` — the list is used verbatim.
//! - `ReviewerSource::ParamRef(name)` — resolved against CLI `--param` overrides
//!   first, then the workflow's `param <name> [defaults...]` declaration.
//!   Missing required params produce a compile error pointing at the loop.
//! - `ReviewerSource::None` — no expansion happens.
//!
//! CLI override format: `--param NAME=id1=provider:model[@effort],id2=...`.
//! Effort is optional and overrides the backend default per-entry.

use std::collections::HashMap;

use crate::ast::*;
use crate::error::{DslError, DslErrors};

/// Parse a `--param NAME=...` value string into a roster.
///
/// Format: `id=provider:model[@effort](,id=provider:model[@effort])*`.
/// Whitespace around each segment is tolerated. Empty input is an error.
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

/// Expand every `loop { reviewers ... }` block in the selected workflow
/// (or all workflows when `workflow_name` is `None`).
///
/// `override_params` is the CLI `--param NAME=VALUE` list. Entries whose key
/// matches a workflow `param <name>` declaration replace that param's
/// default. Params without a CLI value AND without a default produce a
/// compile error.
pub fn expand_reviewers_in_script(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_params: &[(String, String)],
) -> Result<(), DslErrors> {
    let mut errors = Vec::new();
    let mut new_agents: Vec<AgentDecl> = Vec::new();
    let mut new_clients: Vec<ClientDecl> = Vec::new();

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

    for item in script.items.iter_mut() {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name {
            if wf.name != name {
                continue;
            }
        }

        // Build a quick lookup of this workflow's param declarations.
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
                    // Resolve the roster.
                    let roster = match resolve_roster(
                        &lb.reviewers,
                        &params_by_name,
                        override_params,
                    ) {
                        Ok(r) => r,
                        Err(err) => {
                            errors.push(err);
                            // Drop the loop's reviewers so we don't try to
                            // expand again on a partially-broken roster.
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
                        // Synthesise an anonymous client carrying this
                        // entry's model + effort. Reviewer agents bind to
                        // it by name like any other client.
                        let init_client_name = format!("__roster_{}_init", entry.id);
                        let refine_client_name = format!("__roster_{}_refine", entry.id);
                        new_clients.push(synth_client(&init_client_name, entry));
                        new_clients.push(synth_client(&refine_client_name, entry));

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

/// Resolve a `ReviewerSource` (literal or param-ref) into a concrete roster.
fn resolve_roster(
    source: &ReviewerSource,
    params_by_name: &HashMap<String, ParamDecl>,
    override_params: &[(String, String)],
) -> Result<Vec<ReviewerEntry>, DslError> {
    match source {
        ReviewerSource::None => Ok(Vec::new()),
        ReviewerSource::Literal(list) => Ok(list.clone()),
        ReviewerSource::ParamRef(name, span) => {
            // Workflow must declare `param <name>` for the reference to
            // resolve. Without a declaration we cannot tell whether to
            // accept a default or surface a required-param error.
            let decl = params_by_name.get(name).ok_or_else(|| DslError::Compile {
                src: miette::NamedSource::new("script", String::new()),
                span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
                reason: format!(
                    "`reviewers {name}` references an undeclared workflow parameter; \
                     add `param {name}` to the workflow header"
                ),
            })?;

            // CLI override beats the declaration default.
            if let Some((_, v)) = override_params.iter().find(|(k, _)| k == name) {
                return parse_reviewers_override(v).map_err(|e| DslError::Compile {
                    src: miette::NamedSource::new("<cli>", String::new()),
                    span: (0, 1).into(),
                    reason: format!("--param {name}: {e}"),
                });
            }

            // Fall back to the in-script default.
            match &decl.default {
                Some(list) => Ok(list.clone()),
                None => Err(DslError::Compile {
                    src: miette::NamedSource::new("script", String::new()),
                    span: (
                        decl.name_span.start,
                        decl.name_span.end.saturating_sub(decl.name_span.start).max(1),
                    )
                        .into(),
                    reason: format!(
                        "workflow parameter `{name}` has no default and was not \
                         supplied on the CLI — pass `--param {name}=id=provider:model[@effort],...`"
                    ),
                }),
            }
        }
    }
}

/// Build an anonymous `client` for a roster entry. The client's model spec
/// and effort come straight from the entry; privacy defaults to `public`.
fn synth_client(name: &str, entry: &ReviewerEntry) -> ClientDecl {
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

fn inject_peer_vars(mut agent: AgentDecl, self_id: &str, peer_ids: &[String]) -> AgentDecl {
    let others: Vec<&str> = peer_ids
        .iter()
        .map(String::as_str)
        .filter(|id| *id != self_id)
        .collect();
    agent.vars.retain(|(k, _)| {
        !matches!(
            k.as_str(),
            "PEER_IDS" | "PEER_COUNT" | "PEER_READ_BLOCK" | "REVIEWER_ID"
        )
    });
    agent.vars.push(("REVIEWER_ID".into(), self_id.to_string()));
    agent
        .vars
        .push(("PEER_IDS".into(), others.join(",")));
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
    fn parse_simple_two_reviewers() {
        let entries =
            parse_reviewers_override("claude=claude:opus,codex=codex:gpt-5.5").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "claude");
        assert_eq!(entries[0].model, "claude:opus");
        assert!(entries[0].effort.is_none());
        assert_eq!(entries[1].id, "codex");
        assert_eq!(entries[1].model, "codex:gpt-5.5");
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
        assert!(
            err.contains("provider:model"),
            "expected provider:model error, got: {err}"
        );
    }

    #[test]
    fn parse_rejects_empty_id() {
        let err = parse_reviewers_override("=claude:opus").unwrap_err();
        assert!(err.contains("empty id"), "got: {err}");
    }

    #[test]
    fn parse_tolerates_whitespace() {
        let entries =
            parse_reviewers_override("  claude = claude:opus @ max , codex = codex:gpt-5.5 ")
                .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "claude");
        assert_eq!(entries[0].model, "claude:opus");
        assert_eq!(entries[0].effort.as_ref().unwrap().0, "max");
    }
}
