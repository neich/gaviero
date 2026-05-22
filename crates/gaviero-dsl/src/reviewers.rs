//! Roster expansion: `reviewers [ { id "…" client … } ]` clones template agents.

use std::collections::HashMap;

use crate::ast::*;
use crate::error::{DslError, DslErrors};

/// Parse `REVIEWERS=claude:opus,codex:codex-5-5` from CLI overrides.
pub fn parse_reviewers_override(value: &str) -> Result<Vec<ReviewerEntry>, String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (id, client) = part
            .split_once(':')
            .ok_or_else(|| format!("invalid reviewer entry `{part}` (expected id:client)"))?;
        let id = id.trim();
        let client = client.trim();
        if id.is_empty() || client.is_empty() {
            return Err(format!("invalid reviewer entry `{part}`"));
        }
        let span = 0..0;
        out.push(ReviewerEntry {
            id: id.to_string(),
            id_span: span.clone().into(),
            client: client.to_string(),
            client_span: span.clone().into(),
            span: span.into(),
        });
    }
    if out.is_empty() {
        return Err("REVIEWERS override is empty".into());
    }
    Ok(out)
}

/// Expand every `loop { reviewers … }` in the selected workflow (or all workflows).
pub fn expand_reviewers_in_script(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_vars: &[(String, String)],
) -> Result<(), DslErrors> {
    let cli_roster = override_vars
        .iter()
        .find(|(k, _)| k == "REVIEWERS")
        .map(|(_, v)| parse_reviewers_override(v))
        .transpose()
        .map_err(|e| {
            DslErrors::single(DslError::Compile {
                src: miette::NamedSource::new("<cli>", String::new()),
                span: (0, 1).into(),
                reason: e,
            })
        })?;

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

    for item in script.items.iter_mut() {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name {
            if wf.name != name {
                continue;
            }
        }
        let Some((steps, _)) = wf.steps.as_mut() else {
            continue;
        };

        let mut expanded_steps: Vec<StepItem> = Vec::new();
        for step in steps.drain(..) {
            match step {
                StepItem::Loop(mut lb) if !lb.reviewers.is_empty() => {
                    let roster = cli_roster
                        .clone()
                        .unwrap_or_else(|| lb.reviewers.clone());
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
                        let init_agent =
                            clone_reviewer_agent(init_tpl, &format!("{}-init", entry.id), entry);
                        new_agents.push(init_agent.clone());
                        expanded_steps.push(StepItem::Agent(
                            init_agent.name.clone(),
                            init_agent.name_span,
                        ));

                        let refine_agent = clone_reviewer_agent(
                            refine_tpl,
                            &format!("{}-refine", entry.id),
                            entry,
                        );
                        let refine_agent =
                            inject_peer_vars(refine_agent, &entry.id, &peer_ids);
                        refine_agent_names
                            .push((refine_agent.name.clone(), refine_agent.name_span));
                        new_agents.push(refine_agent);
                    }

                    lb.agents = refine_agent_names;
                    lb.reviewers = roster;
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

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    Ok(())
}

fn clone_reviewer_agent(template: &AgentDecl, name: &str, entry: &ReviewerEntry) -> AgentDecl {
    let mut vars = template.vars.clone();
    vars.retain(|(k, _)| k != "REVIEWER_ID");
    vars.push(("REVIEWER_ID".into(), entry.id.clone()));
    // Replace {{REVIEWER_ID}} in owned scope paths at clone time.

    let mut client = template.client.clone();
    let mut tier_ref = template.tier_ref.clone();
    if template.client.is_some() || template.tier_ref.is_some() {
        client = Some((entry.client.clone(), entry.client_span));
        tier_ref = None;
    }

    let mut agent = AgentDecl {
        name: name.to_string(),
        name_span: template.name_span,
        description: template.description.clone(),
        client,
        tier_ref,
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
