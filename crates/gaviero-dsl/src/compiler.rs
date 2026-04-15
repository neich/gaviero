#![allow(deprecated)] // AgentBackend::ClaudeCode is deprecated but still needed for WorkUnit

use std::collections::{HashMap, HashSet};

use miette::NamedSource;

use gaviero_core::swarm::models::{AgentBackend, WorkUnit};
use gaviero_core::swarm::plan::{CompiledPlan, LoopConfig, LoopUntilCondition};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};

use crate::ast::*;
use crate::error::{DslError, DslErrors};

// ── Public types ───────────────────────────────────────────────────────────

/// Backward-compatible alias. New code should use `CompiledPlan` directly.
#[deprecated(note = "Use `CompiledPlan` from `gaviero_core::swarm::plan` instead")]
pub type CompiledScript = CompiledPlan;

// ── AST → WorkUnit compilation ─────────────────────────────────────────────

/// Compile a parsed [`Script`] into a [`CompiledScript`] ready for
/// [`gaviero_core::swarm::pipeline::execute`].
///
/// `workflow` selects which `workflow { steps [...] }` declaration to use
/// for filtering and ordering. `None` means: run all agents in declared
/// order (dependency order check is still performed).
pub fn compile_ast(
    script: &Script,
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, DslErrors> {
    let src = || NamedSource::new(filename, source.to_string());

    // ── Phase 1: index declarations ───────────────────────────────

    let mut client_map: HashMap<&str, &ClientDecl> = HashMap::new();
    let mut agent_map: HashMap<&str, &AgentDecl> = HashMap::new();
    let mut workflow_map: HashMap<&str, &WorkflowDecl> = HashMap::new();
    let mut prompt_map: HashMap<&str, &str> = HashMap::new();
    let mut script_vars: Vec<(String, String)> = Vec::new();
    let mut default_client: Option<&ClientDecl> = None;
    let mut errors: Vec<DslError> = Vec::new();

    for item in &script.items {
        match item {
            Item::Vars(pairs) => {
                // Later declarations override earlier ones for the same key.
                script_vars.extend(pairs.clone());
            }
            Item::Client(c) => {
                if client_map.insert(c.name.as_str(), c).is_some() {
                    errors.push(DslError::Compile {
                        src: src(),
                        span: (c.name_span.start, c.name_span.end.saturating_sub(c.name_span.start)).into(),
                        reason: format!("duplicate client name `{}`", c.name),
                    });
                }
                // Track default client — only one is allowed
                if c.is_default {
                    if default_client.is_some() {
                        errors.push(DslError::Compile {
                            src: src(),
                            span: (c.name_span.start, c.name_span.end.saturating_sub(c.name_span.start)).into(),
                            reason: "only one client can be declared `default`".into(),
                        });
                    } else {
                        default_client = Some(c);
                    }
                }
            }
            Item::Agent(a) => {
                if agent_map.insert(a.name.as_str(), a).is_some() {
                    errors.push(DslError::Compile {
                        src: src(),
                        span: (a.name_span.start, a.name_span.end.saturating_sub(a.name_span.start)).into(),
                        reason: format!("duplicate agent name `{}`", a.name),
                    });
                }
            }
            Item::Workflow(w) => {
                if workflow_map.insert(w.name.as_str(), w).is_some() {
                    errors.push(DslError::Compile {
                        src: src(),
                        span: (w.name_span.start, w.name_span.end.saturating_sub(w.name_span.start)).into(),
                        reason: format!("duplicate workflow name `{}`", w.name),
                    });
                }
            }
            Item::Prompt(p) => {
                if prompt_map.insert(p.name.as_str(), p.content.as_str()).is_some() {
                    errors.push(DslError::Compile {
                        src: src(),
                        span: (p.name_span.start, p.name_span.end.saturating_sub(p.name_span.start)).into(),
                        reason: format!("duplicate prompt name `{}`", p.name),
                    });
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }

    // ── Phase 2: determine agent execution order ──────────────────

    // Returns (agents, max_parallel, workflow_memory)
    let (agent_order, workflow_max_parallel, workflow_memory): (Vec<&AgentDecl>, Option<usize>, Option<&MemoryBlock>) =
        if let Some(wf_name) = workflow {
            // Explicit workflow named
            let wf = workflow_map.get(wf_name).ok_or_else(|| {
                DslErrors::single(DslError::Compile {
                    src: src(),
                    span: (0, 1).into(),
                    reason: format!("workflow `{}` is not defined", wf_name),
                })
            })?;
            let agents = ordered_agents_from_workflow(wf, &agent_map, source, filename)?;
            let mp = wf.max_parallel.as_ref().map(|(n, _)| *n);
            (agents, mp, wf.memory.as_ref())
        } else if workflow_map.len() == 1 {
            // Exactly one workflow — use it implicitly
            let wf = workflow_map.values().next().unwrap();
            let agents = ordered_agents_from_workflow(wf, &agent_map, source, filename)?;
            let mp = wf.max_parallel.as_ref().map(|(n, _)| *n);
            (agents, mp, wf.memory.as_ref())
        } else if workflow_map.is_empty() {
            // No workflow — run all agents in declaration order
            let agents = script
                .items
                .iter()
                .filter_map(|i| if let Item::Agent(a) = i { Some(a) } else { None })
                .collect();
            (agents, None, None)
        } else {
            // Multiple workflows with no selector — ambiguous
            return Err(DslErrors::single(DslError::Compile {
                src: src(),
                span: (0, 1).into(),
                reason: format!(
                    "multiple workflows defined ({}); pass --workflow <name> to select one",
                    workflow_map.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            }));
        };

    // ── Phase 3: compile each agent to WorkUnit ───────────────────

    let mut work_units = Vec::with_capacity(agent_order.len());
    let mut compile_errors: Vec<DslError> = Vec::new();

    for decl in agent_order {
        match compile_agent(decl, &client_map, default_client, &prompt_map, &script_vars, workflow_memory, source, filename, runtime_prompt) {
            Ok(wu) => work_units.push(wu),
            Err(e) => compile_errors.push(e),
        }
    }

    // ── Phase 4: validate depends_on references ───────────────────

    for wu in &work_units {
        if let Some(decl) = agent_map.get(wu.id.as_str()) {
            if let Some((deps, _)) = &decl.depends_on {
                for (dep_name, dep_span) in deps {
                    if !agent_map.contains_key(dep_name.as_str()) {
                        compile_errors.push(DslError::Compile {
                            src: src(),
                            span: (dep_span.start, dep_span.end.saturating_sub(dep_span.start).max(1)).into(),
                            reason: format!(
                                "agent `{}` depends_on `{}` which is not defined",
                                wu.id, dep_name
                            ),
                        });
                    }
                }
            }
        }
    }

    if !compile_errors.is_empty() {
        return Err(DslErrors::new(compile_errors));
    }

    // ── Phase 5: detect dependency cycles ─────────────────────────

    if let Some(cycle) = detect_cycle(&work_units) {
        let cycle_agent = agent_map.get(cycle[0].as_str());
        let span = cycle_agent
            .and_then(|a| a.depends_on.as_ref())
            .map(|(_, s)| (s.start, s.end.saturating_sub(s.start).max(1)))
            .unwrap_or((0, 1));
        return Err(DslErrors::single(DslError::Compile {
            src: src(),
            span: span.into(),
            reason: format!(
                "dependency cycle detected: {}",
                cycle.join(" -> ")
            ),
        }));
    }

    // ── Phase 6: build iteration / verification configs ───────────

    // Warn when all agents are independent (no depends_on); single-agent with
    // strategy refine may be a better choice.
    let independent_count = work_units.iter().filter(|u| u.depends_on.is_empty()).count();
    if work_units.len() > 1 && independent_count == work_units.len() {
        eprintln!(
            "⚠ This workflow has {} independent agents. Consider using a single agent with `strategy refine`.",
            work_units.len()
        );
    }

    // Resolve which workflow was selected (if any).
    let selected_workflow: Option<&WorkflowDecl> = if let Some(wf_name) = workflow {
        workflow_map.get(wf_name).copied()
    } else if workflow_map.len() == 1 {
        workflow_map.values().next().copied()
    } else {
        None
    };

    let iteration_config = selected_workflow
        .map(build_iteration_config)
        .unwrap_or_default();

    let verification_config = selected_workflow
        .map(build_verification_config)
        .unwrap_or_default();

    // ── Phase 7: extract LoopConfigs from workflow steps ────────

    let loop_configs = extract_loop_configs(selected_workflow);

    let mut plan = CompiledPlan::from_work_units(work_units, workflow_max_parallel);
    plan.iteration_config = iteration_config;
    plan.verification_config = verification_config;
    plan.loop_configs = loop_configs;
    Ok(plan)
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// DFS-based cycle detection. Returns the cycle path if found.
fn detect_cycle(work_units: &[WorkUnit]) -> Option<Vec<String>> {
    // Use String keys to avoid lifetime entanglement between deps map and state map.
    let deps: HashMap<String, Vec<String>> = work_units
        .iter()
        .map(|wu| (wu.id.clone(), wu.depends_on.clone()))
        .collect();

    #[derive(Clone, Copy, PartialEq)]
    enum Visit { Unvisited, InProgress, Done }

    let mut state: HashMap<String, Visit> = deps.keys().map(|k| (k.clone(), Visit::Unvisited)).collect();
    let mut path: Vec<String> = Vec::new();

    let keys: Vec<String> = deps.keys().cloned().collect();
    for node in &keys {
        if state[node] == Visit::Unvisited && dfs_visit(node, &deps, &mut state, &mut path) {
            return Some(path);
        }
    }
    return None;

    fn dfs_visit(
        node: &str,
        deps: &HashMap<String, Vec<String>>,
        state: &mut HashMap<String, Visit>,
        path: &mut Vec<String>,
    ) -> bool {
        state.insert(node.to_string(), Visit::InProgress);
        path.push(node.to_string());

        if let Some(dep_list) = deps.get(node) {
            for dep in dep_list {
                match state.get(dep.as_str()).copied() {
                    Some(Visit::InProgress) => {
                        path.push(dep.clone());
                        return true;
                    }
                    Some(Visit::Unvisited) | None => {
                        if dfs_visit(dep, deps, state, path) {
                            return true;
                        }
                    }
                    Some(Visit::Done) => {}
                }
            }
        }

        path.pop();
        state.insert(node.to_string(), Visit::Done);
        false
    }
}

fn ordered_agents_from_workflow<'a>(
    wf: &WorkflowDecl,
    agent_map: &HashMap<&str, &'a AgentDecl>,
    source: &str,
    filename: &str,
) -> Result<Vec<&'a AgentDecl>, DslErrors> {
    let src = || NamedSource::new(filename, source.to_string());

    let steps = match &wf.steps {
        Some((s, _)) => s,
        None => {
            return Err(DslErrors::single(DslError::Compile {
                src: src(),
                span: (wf.span.start, wf.span.end.saturating_sub(wf.span.start)).into(),
                reason: format!("workflow `{}` has no `steps` field", wf.name),
            }));
        }
    };

    let mut agents: Vec<&AgentDecl> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut errors = Vec::new();

    // Flatten StepItems: agent refs resolve directly, loop blocks contribute
    // their inner agent list. Agents are deduplicated by name so the same
    // agent can appear in both a linear step and a loop without being compiled
    // twice. Declaration order is preserved (first occurrence wins).
    for step in steps {
        let refs: Vec<(&str, &Span)> = match step {
            StepItem::Agent(name, span) => vec![(name.as_str(), span)],
            StepItem::Loop(lb) => lb.agents.iter().map(|(n, s)| (n.as_str(), s)).collect(),
        };
        for (name, name_span) in refs {
            if seen.contains(name) {
                continue;
            }
            match agent_map.get(name) {
                Some(a) => {
                    seen.insert(name);
                    agents.push(*a);
                }
                None => errors.push(DslError::Compile {
                    src: src(),
                    span: (name_span.start, name_span.end.saturating_sub(name_span.start)).into(),
                    reason: format!("workflow step `{}` is not a defined agent", name),
                }),
            }
        }
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    Ok(agents)
}

/// Apply compile-time variable substitution to a template string.
///
/// Substitution order (agent vars override script vars; built-ins override all):
/// 1. Merge script vars + agent vars (agent wins on collision)
/// 2. Apply merged vars in a single pass
/// 3. Apply built-ins: `{{PROMPT}}` and `{{AGENT}}` (cannot be shadowed)
///
/// `{{ITER}}` and `{{PREV_ITER}}` are intentionally left untouched —
/// they are injected at runtime by the loop executor.
fn apply_vars(
    text: &str,
    script_vars: &[(String, String)],
    agent_vars: &[(String, String)],
    agent_name: &str,
    runtime_prompt: Option<&str>,
) -> String {
    const RESERVED: &[&str] = &["PROMPT", "AGENT", "ITER", "PREV_ITER"];

    // Build a merged list: script vars first, then agent vars overriding same keys.
    // We apply the merged list in a single pass so that agent vars win.
    let mut merged: Vec<(&str, &str)> = Vec::new();

    for (k, v) in script_vars {
        if RESERVED.contains(&k.as_str()) {
            continue;
        }
        // Only include if not overridden by an agent var
        if !agent_vars.iter().any(|(ak, _)| ak == k) {
            merged.push((k.as_str(), v.as_str()));
        }
    }
    for (k, v) in agent_vars {
        if RESERVED.contains(&k.as_str()) {
            eprintln!("⚠ agent `{agent_name}`: vars key `{k}` shadows a reserved variable and will be ignored");
            continue;
        }
        merged.push((k.as_str(), v.as_str()));
    }

    let mut result = text.to_string();
    for (k, v) in merged {
        result = result.replace(&format!("{{{{{k}}}}}"), v);
    }

    // Built-ins (applied last, cannot be overridden by user vars)
    if let Some(rp) = runtime_prompt {
        result = result.replace("{{PROMPT}}", rp);
    }
    result = result.replace("{{AGENT}}", agent_name);
    result
}

fn compile_agent(
    decl: &AgentDecl,
    client_map: &HashMap<&str, &ClientDecl>,
    default_client: Option<&ClientDecl>,
    prompt_map: &HashMap<&str, &str>,
    script_vars: &[(String, String)],
    workflow_memory: Option<&MemoryBlock>,
    source: &str,
    filename: &str,
    runtime_prompt: Option<&str>,
) -> Result<WorkUnit, DslError> {
    let src = || NamedSource::new(filename, source.to_string());

    // Resolve client reference
    let (tier, model, privacy) = if let Some((client_name, client_span)) = &decl.client {
        // Explicit client reference
        let cd = client_map.get(client_name.as_str()).ok_or_else(|| DslError::Compile {
            src: src(),
            span: (client_span.start, client_span.end.saturating_sub(client_span.start).max(1)).into(),
            reason: format!("undefined client `{}`", client_name),
        })?;
        let tier = cd.tier.as_ref().map(|(t, _)| map_tier(*t)).unwrap_or_default();
        let model = cd.model.as_ref().map(|(m, _)| m.clone());
        let privacy = cd.privacy.as_ref().map(|(p, _)| map_privacy(*p)).unwrap_or_default();
        (tier, model, privacy)
    } else if let Some(dc) = default_client {
        // No explicit client, but a default exists — use it
        let tier = dc.tier.as_ref().map(|(t, _)| map_tier(*t)).unwrap_or_default();
        let model = dc.model.as_ref().map(|(m, _)| m.clone());
        let privacy = dc.privacy.as_ref().map(|(p, _)| map_privacy(*p)).unwrap_or_default();
        (tier, model, privacy)
    } else {
        // No explicit client, no default — warn if any clients are declared
        if !client_map.is_empty() {
            eprintln!(
                "⚠ agent `{}`: no client declared and no default client set; \
                 falling back to tier=Cheap/model=None. \
                 Declare a default client with `default` in the client block.",
                decl.name
            );
        }
        (ModelTier::default(), None, PrivacyLevel::default())
    };

    // Build scope
    let scope = if let Some(sb) = &decl.scope {
        FileScope {
            owned_paths: sb.owned.clone(),
            read_only_paths: sb.read_only.clone(),
            interface_contracts: HashMap::new(),
        }
    } else {
        FileScope {
            owned_paths: vec![".".to_string()],
            read_only_paths: Vec::new(),
            interface_contracts: HashMap::new(),
        }
    };

    let depends_on = decl
        .depends_on
        .as_ref()
        .map(|(deps, _)| deps.iter().map(|(s, _)| s.clone()).collect())
        .unwrap_or_default();

    let description = {
        let raw = match &decl.description {
            Some((s, _)) => s.as_str(),
            None => &decl.name,
        };
        apply_vars(raw, script_vars, &decl.vars, &decl.name, runtime_prompt)
    };

    // Resolve prompt: inline string or reference to a named prompt declaration.
    let prompt_text: Option<&str> = match &decl.prompt {
        Some((PromptSource::Inline(s), _)) => Some(s.as_str()),
        Some((PromptSource::Ref(name, ref_span), _)) => {
            match prompt_map.get(name.as_str()) {
                Some(content) => Some(content),
                None => {
                    return Err(DslError::Compile {
                        src: src(),
                        span: (ref_span.start, ref_span.end.saturating_sub(ref_span.start).max(1)).into(),
                        reason: format!("undefined prompt `{}`", name),
                    });
                }
            }
        }
        None => None,
    };

    // Apply all compile-time substitutions.
    // {{ITER}} and {{PREV_ITER}} are intentionally left for runtime.
    let coordinator_instructions = match prompt_text {
        Some(s) => apply_vars(s, script_vars, &decl.vars, &decl.name, runtime_prompt),
        None => runtime_prompt.unwrap_or("").to_string(),
    };

    let max_retries = decl.max_retries.as_ref().map(|(n, _)| *n).unwrap_or(1);

    // ── Memory merge logic ────────────────────────────────────────
    //
    // read_ns: additive — workflow namespaces prepended, agent namespaces
    //          appended, duplicates removed (order preserved).
    // write_ns: agent overrides workflow; None if neither declares one.
    // importance: agent-only; None → store uses default (0.5).
    // staleness_sources: agent-only.
    let read_namespaces: Option<Vec<String>> = {
        let mut merged: Vec<String> = workflow_memory
            .map(|m| m.read_ns.clone())
            .unwrap_or_default();
        for ns in decl.memory.as_ref().map(|m| m.read_ns.clone()).unwrap_or_default() {
            if !merged.contains(&ns) {
                merged.push(ns);
            }
        }
        if merged.is_empty() { None } else { Some(merged) }
    };

    let write_namespace: Option<String> = decl.memory
        .as_ref()
        .and_then(|m| m.write_ns.clone())
        .or_else(|| workflow_memory.and_then(|m| m.write_ns.clone()));

    let memory_importance: Option<f32> = decl.memory.as_ref().and_then(|m| m.importance);

    let staleness_sources: Vec<String> = decl.memory
        .as_ref()
        .map(|m| m.staleness_sources.clone())
        .unwrap_or_default();

    // ── Explicit memory control fields ───────────────────────────
    let memory_read_query: Option<String> = decl.memory
        .as_ref()
        .and_then(|m| m.read_query.as_ref())
        .map(|(s, _)| apply_vars(s, script_vars, &decl.vars, &decl.name, runtime_prompt));

    let memory_read_limit: Option<usize> = decl.memory
        .as_ref()
        .and_then(|m| m.read_limit.as_ref())
        .map(|(n, _)| *n);

    let memory_write_content: Option<String> = decl.memory
        .as_ref()
        .and_then(|m| m.write_content.as_ref())
        .map(|(s, _)| apply_vars(s, script_vars, &decl.vars, &decl.name, runtime_prompt));

    // ── Graph / impact fields ─────────────────────────────────────
    let impact_scope = decl.scope
        .as_ref()
        .and_then(|s| s.impact_scope.as_ref())
        .map(|(v, _)| *v)
        .unwrap_or(false);

    let context_callers_of: Vec<String> = decl.context
        .as_ref()
        .map(|c| c.callers_of.clone())
        .unwrap_or_default();

    let context_tests_for: Vec<String> = decl.context
        .as_ref()
        .map(|c| c.tests_for.clone())
        .unwrap_or_default();

    let context_depth: u32 = decl.context
        .as_ref()
        .and_then(|c| c.depth.as_ref())
        .map(|(n, _)| *n)
        .unwrap_or(2);

    Ok(WorkUnit {
        id: decl.name.clone(),
        description,
        scope,
        depends_on,
        backend: AgentBackend::default(),
        model,
        tier,
        privacy,
        coordinator_instructions,
        estimated_tokens: 0,
        max_retries,
        escalation_tier: None,
        read_namespaces,
        write_namespace,
        memory_importance,
        staleness_sources,
        memory_read_query,
        memory_read_limit,
        memory_write_content,
        impact_scope,
        context_callers_of,
        context_tests_for,
        context_depth,
    })
}

fn map_tier(t: TierLit) -> ModelTier {
    match t {
        TierLit::Cheap => ModelTier::Cheap,
        TierLit::Expensive => ModelTier::Expensive,
        // Deprecated aliases
        TierLit::Coordinator | TierLit::Reasoning => ModelTier::Expensive,
        TierLit::Execution | TierLit::Mechanical => ModelTier::Cheap,
    }
}

fn map_privacy(p: PrivacyLit) -> PrivacyLevel {
    match p {
        PrivacyLit::Public => PrivacyLevel::Public,
        PrivacyLit::LocalOnly => PrivacyLevel::LocalOnly,
    }
}

fn build_iteration_config(wf: &WorkflowDecl) -> gaviero_core::iteration::IterationConfig {
    use gaviero_core::iteration::{IterationConfig, Strategy};
    let strategy = wf
        .strategy
        .as_ref()
        .map(|(s, _)| match s {
            StrategyLit::SinglePass => Strategy::SinglePass,
            StrategyLit::Refine => Strategy::Refine,
            StrategyLit::BestOfN(n) => Strategy::BestOfN { n: *n },
        })
        .unwrap_or_default();
    let max_retries = wf.max_retries.as_ref().map(|(n, _)| *n).unwrap_or(5);
    let max_attempts = wf.attempts.as_ref().map(|(n, _)| *n).unwrap_or(1);
    let test_first = wf.test_first.as_ref().map(|(b, _)| *b).unwrap_or(false);
    let escalate_after = wf.escalate_after.as_ref().map(|(n, _)| *n).unwrap_or(3);
    IterationConfig {
        strategy,
        max_retries,
        max_attempts,
        test_first,
        escalate_after,
        ..IterationConfig::default()
    }
}

fn build_verification_config(wf: &WorkflowDecl) -> gaviero_core::swarm::plan::VerificationConfig {
    wf.verify
        .as_ref()
        .map(|v| gaviero_core::swarm::plan::VerificationConfig {
            compile: v.compile,
            clippy: v.clippy,
            test: v.test,
            impact_tests: v.impact_tests,
        })
        .unwrap_or_default()
}

fn extract_loop_configs(wf: Option<&WorkflowDecl>) -> Vec<LoopConfig> {
    let wf = match wf {
        Some(w) => w,
        None => return Vec::new(),
    };
    let steps = match &wf.steps {
        Some((s, _)) => s,
        None => return Vec::new(),
    };

    steps
        .iter()
        .filter_map(|step| {
            if let StepItem::Loop(lb) = step {
                Some(LoopConfig {
                    agent_ids: lb.agents.iter().map(|(n, _)| n.clone()).collect(),
                    until: map_until_condition(&lb.until),
                    max_iterations: lb.max_iterations,
                    iter_start: lb.iter_start,
                })
            } else {
                None
            }
        })
        .collect()
}

fn map_until_condition(cond: &UntilCondition) -> LoopUntilCondition {
    match cond {
        UntilCondition::Verify(vb) => LoopUntilCondition::Verify(
            gaviero_core::swarm::plan::VerificationConfig {
                compile: vb.compile,
                clippy: vb.clippy,
                test: vb.test,
                impact_tests: vb.impact_tests,
            },
        ),
        UntilCondition::Agent(name, _) => LoopUntilCondition::Agent(name.clone()),
        UntilCondition::Command(cmd, _) => LoopUntilCondition::Command(cmd.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};
    use gaviero_core::types::ModelTier;

    fn compile_str(src: &str) -> Result<Vec<WorkUnit>, DslErrors> {
        let (tokens, _) = lexer::lex(src);
        let (ast, errs) = parser::parse(&tokens, src, "test.gaviero");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        compile_ast(&ast.unwrap(), src, "test.gaviero", None, None)
            .map(|c| c.work_units_ordered().expect("toposort in tests"))
    }

    const FULL_EXAMPLE: &str = r##"
        client opus   { tier coordinator model "claude-opus-4-6" }
        client sonnet { tier execution   model "claude-sonnet-4-6" }

        agent researcher {
            description "Research the codebase"
            client opus
            scope { owned ["docs/architecture.md"] read_only ["src/**"] }
            prompt #"Analyze the codebase and write a comprehensive architecture document."#
            max_retries 2
        }

        agent implementer {
            description "Implement the feature"
            client sonnet
            depends_on [researcher]
            scope { owned ["src/feature/"] read_only ["docs/architecture.md"] }
            prompt #"Based on the architecture document, implement the feature."#
        }

        workflow feature_dev { steps [researcher implementer] }
    "##;

    #[test]
    fn full_example_compiles() {
        let units = compile_str(FULL_EXAMPLE).expect("should compile");
        assert_eq!(units.len(), 2);

        assert_eq!(units[0].id, "researcher");
        assert_eq!(units[0].tier, ModelTier::Expensive);
        assert_eq!(units[0].model, Some("claude-opus-4-6".to_string()));
        assert!(units[0].coordinator_instructions.contains("architecture document"));
        assert_eq!(units[0].max_retries, 2);
        assert_eq!(units[0].scope.owned_paths, vec!["docs/architecture.md"]);
        assert_eq!(units[0].scope.read_only_paths, vec!["src/**"]);

        assert_eq!(units[1].id, "implementer");
        assert_eq!(units[1].tier, ModelTier::Cheap);
        assert_eq!(units[1].depends_on, vec!["researcher"]);
        assert!(units[1].coordinator_instructions.contains("architecture document"));
    }

    #[test]
    fn undefined_client_ref() {
        let src = r#"agent x { client nonexistent }"#;
        let result = compile_str(src);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        let msg = format!("{}", errs.errors[0]);
        assert!(msg.contains("nonexistent"), "error: {}", msg);
    }

    #[test]
    fn duplicate_agent_name() {
        let src = r#"agent x { } agent x { }"#;
        let result = compile_str(src);
        assert!(result.is_err());
    }

    #[test]
    fn agent_no_client_uses_defaults() {
        let src = r#"agent simple { description "task" }"#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].tier, ModelTier::Cheap); // Cheap is now the default
        assert_eq!(units[0].model, None);
    }

    #[test]
    fn agent_no_client_uses_default_client() {
        let src = r#"
            client sonnet { tier cheap model "claude-sonnet-4-6" default }
            agent a { description "uses default" }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].tier, ModelTier::Cheap);
        assert_eq!(units[0].model, Some("claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn explicit_client_overrides_default() {
        let src = r#"
            client sonnet { tier cheap model "claude-sonnet-4-6" default }
            client opus { tier expensive model "claude-opus-4-6" }
            agent a { description "explicit opus" client opus }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].tier, ModelTier::Expensive);
        assert_eq!(units[0].model, Some("claude-opus-4-6".to_string()));
    }

    #[test]
    fn multiple_defaults_is_compile_error() {
        let src = r#"
            client sonnet { tier cheap model "claude-sonnet-4-6" default }
            client opus { tier expensive model "claude-opus-4-6" default }
            agent a { description "test" }
        "#;
        let result = compile_str(src);
        assert!(result.is_err(), "multiple defaults should error");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("default"), "error should mention default: {}", msg);
    }

    #[test]
    fn agent_no_scope_gets_dot_scope() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str(src).expect("compile");
        assert_eq!(units[0].scope.owned_paths, vec!["."]);
    }

    #[test]
    fn workflow_filter() {
        let src = r#"
            agent a { description "a" }
            agent b { description "b" }
            workflow only_a { steps [a] }
        "#;
        let (tokens, _) = lexer::lex(src);
        let (ast, _) = parser::parse(&tokens, src, "t");
        let compiled = compile_ast(&ast.unwrap(), src, "t", Some("only_a"), None).unwrap();
        let units = compiled.work_units_ordered().expect("toposort in tests");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "a");
    }

    #[test]
    fn undefined_workflow_step_errors() {
        let src = r#"agent a { } workflow w { steps [a ghost] }"#;
        let (tokens, _) = lexer::lex(src);
        let (ast, _) = parser::parse(&tokens, src, "t");
        let result = compile_ast(&ast.unwrap(), src, "t", None, None);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("ghost"), "{}", msg);
    }

    #[test]
    fn cycle_detection_simple() {
        let src = r#"
            agent a { depends_on [b] }
            agent b { depends_on [a] }
        "#;
        let result = compile_str(src);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("cycle"), "expected cycle error, got: {}", msg);
    }

    #[test]
    fn cycle_detection_three_nodes() {
        let src = r#"
            agent a { depends_on [c] }
            agent b { depends_on [a] }
            agent c { depends_on [b] }
        "#;
        let result = compile_str(src);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("cycle"), "expected cycle error, got: {}", msg);
    }

    #[test]
    fn no_cycle_linear_chain() {
        let src = r#"
            agent a { description "first" }
            agent b { depends_on [a] description "second" }
            agent c { depends_on [b] description "third" }
        "#;
        let result = compile_str(src);
        assert!(result.is_ok(), "linear chain should not be a cycle");
    }

    #[test]
    fn self_dependency_is_cycle() {
        let src = r#"agent self_ref { depends_on [self_ref] }"#;
        let result = compile_str(src);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("cycle"), "self-dep should be a cycle: {}", msg);
    }

    #[test]
    fn workflow_max_parallel_propagated() {
        let src = r#"
            agent a { description "first" }
            workflow w { steps [a] max_parallel 3 }
        "#;
        let (tokens, _) = lexer::lex(src);
        let (ast, _) = parser::parse(&tokens, src, "t");
        let compiled = compile_ast(&ast.unwrap(), src, "t", None, None).unwrap();
        assert_eq!(compiled.max_parallel, Some(3));
    }

    #[test]
    fn memory_merge_additive_read_ns() {
        let src = r#"
            agent a {
                memory {
                    read_ns ["agent-ns"]
                    write_ns "agent-out"
                }
            }
            workflow w {
                steps [a]
                memory {
                    read_ns ["workflow-ns"]
                    write_ns "wf-out"
                }
            }
        "#;
        let units = compile_str(src).unwrap();
        // read_ns: workflow first, agent second
        let ns = units[0].read_namespaces.as_ref().unwrap();
        assert_eq!(ns[0], "workflow-ns");
        assert_eq!(ns[1], "agent-ns");
        // write_ns: agent overrides workflow
        assert_eq!(units[0].write_namespace.as_deref(), Some("agent-out"));
    }

    #[test]
    fn memory_merge_workflow_write_ns_fallback() {
        let src = r#"
            agent a { description "t" }
            workflow w {
                steps [a]
                memory { write_ns "wf-default" }
            }
        "#;
        let units = compile_str(src).unwrap();
        assert_eq!(units[0].write_namespace.as_deref(), Some("wf-default"));
    }

    #[test]
    fn memory_importance_and_staleness() {
        let src = r#"
            agent scan {
                memory {
                    importance 0.8
                    staleness_sources ["src/" "Cargo.toml"]
                }
            }
        "#;
        let units = compile_str(src).unwrap();
        assert!(matches!(units[0].memory_importance, Some(v) if (v - 0.8).abs() < 1e-5));
        assert_eq!(units[0].staleness_sources, vec!["src/", "Cargo.toml"]);
    }

    #[test]
    fn memory_none_when_not_declared() {
        let src = r#"agent a { description "task" }"#;
        let units = compile_str(src).unwrap();
        assert!(units[0].read_namespaces.is_none());
        assert!(units[0].write_namespace.is_none());
        assert!(units[0].memory_importance.is_none());
        assert!(units[0].staleness_sources.is_empty());
    }

    #[test]
    fn undefined_depends_on_has_span() {
        let src = r#"agent a { depends_on [nonexistent] }"#;
        let result = compile_str(src);
        assert!(result.is_err());
        let err = &result.unwrap_err().errors[0];
        // The span should point at "nonexistent", not at file start (0,1)
        if let DslError::Compile { span, .. } = err {
            let offset: usize = span.offset();
            assert!(offset > 0, "span should point at the bad dep, not file start");
        }
    }

    fn compile_str_with_prompt<'a>(src: &'a str, runtime_prompt: Option<&'a str>) -> Result<Vec<WorkUnit>, DslErrors> {
        let (tokens, _) = lexer::lex(src);
        let (ast, errs) = parser::parse(&tokens, src, "test.gaviero");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        compile_ast(&ast.unwrap(), src, "test.gaviero", None, runtime_prompt)
            .map(|c| c.work_units_ordered().expect("toposort in tests"))
    }

    #[test]
    fn runtime_prompt_substituted_in_coordinator_instructions() {
        let src = r##"agent x { prompt #"Task: {{PROMPT}}"# }"##;
        let units = compile_str_with_prompt(src, Some("implement login")).unwrap();
        assert_eq!(units[0].coordinator_instructions, "Task: implement login");
    }

    #[test]
    fn runtime_prompt_fixed_prompt_unchanged() {
        let src = r##"agent x { prompt #"Fixed task: do the thing"# }"##;
        let units = compile_str_with_prompt(src, Some("ignored")).unwrap();
        assert_eq!(units[0].coordinator_instructions, "Fixed task: do the thing");
    }

    #[test]
    fn runtime_prompt_fallback_no_prompt_field() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str_with_prompt(src, Some("do the thing")).unwrap();
        assert_eq!(units[0].coordinator_instructions, "do the thing");
    }

    #[test]
    fn runtime_prompt_none_no_prompt_field_empty() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str_with_prompt(src, None).unwrap();
        assert_eq!(units[0].coordinator_instructions, "");
    }

    #[test]
    fn runtime_prompt_substituted_in_description() {
        let src = r#"agent x { description "Process {{PROMPT}}" }"#;
        let units = compile_str_with_prompt(src, Some("files")).unwrap();
        assert_eq!(units[0].description, "Process files");
    }

    #[test]
    fn runtime_prompt_multiple_placeholder_occurrences() {
        let src = r##"agent x { prompt #"{{PROMPT}} and also {{PROMPT}}"# }"##;
        let units = compile_str_with_prompt(src, Some("X")).unwrap();
        assert_eq!(units[0].coordinator_instructions, "X and also X");
    }

    #[test]
    fn memory_read_query_and_limit_compiled() {
        let src = r#"
            agent x {
                memory {
                    read_query "custom search query"
                    read_limit 10
                }
            }
        "#;
        let units = compile_str(src).unwrap();
        assert_eq!(units[0].memory_read_query.as_deref(), Some("custom search query"));
        assert_eq!(units[0].memory_read_limit, Some(10));
    }

    #[test]
    fn memory_write_content_compiled() {
        let src = r##"
            agent x {
                memory {
                    write_ns "output"
                    write_content #"Findings: {{SUMMARY}}"#
                }
            }
        "##;
        let units = compile_str(src).unwrap();
        assert_eq!(units[0].write_namespace.as_deref(), Some("output"));
        assert_eq!(units[0].memory_write_content.as_deref(), Some("Findings: {{SUMMARY}}"));
    }

    #[test]
    fn memory_read_query_prompt_substitution() {
        let src = r#"
            agent x {
                memory {
                    read_query "find results for {{PROMPT}}"
                }
            }
        "#;
        let units = compile_str_with_prompt(src, Some("auth bugs")).unwrap();
        assert_eq!(units[0].memory_read_query.as_deref(), Some("find results for auth bugs"));
    }

    #[test]
    fn memory_fields_none_when_not_declared() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str(src).unwrap();
        assert!(units[0].memory_read_query.is_none());
        assert!(units[0].memory_read_limit.is_none());
        assert!(units[0].memory_write_content.is_none());
    }

    // ── Loop tests ───────────────────────────────────────────────

    fn compile_plan(src: &str) -> Result<CompiledPlan, DslErrors> {
        let (tokens, _) = lexer::lex(src);
        let (ast, errs) = parser::parse(&tokens, src, "test.gaviero");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        compile_ast(&ast.unwrap(), src, "test.gaviero", None, None)
    }

    #[test]
    fn loop_config_extracted_verify() {
        let src = r#"
            agent a { description "impl" }
            agent b { description "test" }
            workflow w {
                steps [
                    loop {
                        agents [a b]
                        max_iterations 5
                        until { compile true test true }
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).unwrap();
        assert_eq!(plan.loop_configs.len(), 1);
        let lc = &plan.loop_configs[0];
        assert_eq!(lc.agent_ids, vec!["a", "b"]);
        assert_eq!(lc.max_iterations, 5);
        assert!(matches!(&lc.until, LoopUntilCondition::Verify(v) if v.compile && v.test && !v.clippy));
    }

    #[test]
    fn loop_config_extracted_command() {
        let src = r#"
            agent fixer { description "fix" }
            workflow w {
                steps [
                    loop {
                        agents [fixer]
                        max_iterations 3
                        until command "make test"
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).unwrap();
        assert_eq!(plan.loop_configs.len(), 1);
        assert!(matches!(&plan.loop_configs[0].until, LoopUntilCondition::Command(cmd) if cmd == "make test"));
    }

    #[test]
    fn loop_config_extracted_agent() {
        let src = r#"
            agent impl_agent { description "implement" }
            agent judge { description "evaluate quality" }
            workflow w {
                steps [
                    loop {
                        agents [impl_agent]
                        max_iterations 3
                        until agent judge
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).unwrap();
        assert_eq!(plan.loop_configs.len(), 1);
        assert!(matches!(&plan.loop_configs[0].until, LoopUntilCondition::Agent(name) if name == "judge"));
    }

    #[test]
    fn no_loop_configs_when_no_loops() {
        let src = r#"
            agent a { description "t" }
            workflow w { steps [a] }
        "#;
        let plan = compile_plan(src).unwrap();
        assert!(plan.loop_configs.is_empty());
    }

    #[test]
    fn loop_agents_included_in_work_units() {
        let src = r#"
            agent pre { description "pre-step" }
            agent looped { description "in loop" }
            agent post { description "post-step" }
            workflow w {
                steps [
                    pre
                    loop {
                        agents [looped]
                        max_iterations 3
                        until { test true }
                    }
                    post
                ]
            }
        "#;
        let plan = compile_plan(src).unwrap();
        let units = plan.work_units_ordered().unwrap();
        let ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
        assert!(ids.contains(&"pre"));
        assert!(ids.contains(&"looped"));
        assert!(ids.contains(&"post"));
    }

    // ── Graph / impact tests ─────────────────────────────────────

    #[test]
    fn impact_scope_compiled() {
        let src = r#"
            agent x {
                scope {
                    owned ["src/"]
                    impact_scope true
                }
            }
        "#;
        let units = compile_str(src).unwrap();
        assert!(units[0].impact_scope);
    }

    #[test]
    fn impact_scope_false_by_default() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str(src).unwrap();
        assert!(!units[0].impact_scope);
    }

    #[test]
    fn context_block_compiled() {
        let src = r#"
            agent x {
                context {
                    callers_of ["src/auth.rs"]
                    tests_for  ["src/auth/"]
                    depth      3
                }
            }
        "#;
        let units = compile_str(src).unwrap();
        assert_eq!(units[0].context_callers_of, vec!["src/auth.rs"]);
        assert_eq!(units[0].context_tests_for, vec!["src/auth/"]);
        assert_eq!(units[0].context_depth, 3);
    }

    #[test]
    fn context_defaults_when_not_declared() {
        let src = r#"agent x { description "t" }"#;
        let units = compile_str(src).unwrap();
        assert!(units[0].context_callers_of.is_empty());
        assert!(units[0].context_tests_for.is_empty());
        assert_eq!(units[0].context_depth, 2);
    }

    #[test]
    fn impact_tests_in_verify() {
        let src = r#"
            agent a { description "t" }
            workflow w {
                steps [a]
                verify { compile true impact_tests true }
            }
        "#;
        let plan = compile_plan(src).unwrap();
        assert!(plan.verification_config.compile);
        assert!(plan.verification_config.impact_tests);
        assert!(!plan.verification_config.test);
    }

    #[test]
    fn impact_tests_in_loop_until() {
        let src = r#"
            agent a { description "impl" }
            workflow w {
                steps [
                    loop {
                        agents [a]
                        max_iterations 3
                        until { compile true impact_tests true }
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).unwrap();
        assert_eq!(plan.loop_configs.len(), 1);
        if let LoopUntilCondition::Verify(v) = &plan.loop_configs[0].until {
            assert!(v.compile);
            assert!(v.impact_tests);
        } else {
            panic!("expected Verify");
        }
    }

    // ── Named prompt declaration tests ────────────────────────────────────────

    #[test]
    fn named_prompt_resolves() {
        let src = r#"
            prompt my-prompt "do the thing"
            agent x { prompt my-prompt }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "do the thing");
    }

    #[test]
    fn named_prompt_with_runtime_prompt_substitution() {
        let src = r#"
            prompt body "task: {{PROMPT}}"
            agent x { prompt body }
        "#;
        let (tokens, _) = lexer::lex(src);
        let (ast, errs) = parser::parse(&tokens, src, "test.gaviero");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let units = compile_ast(&ast.unwrap(), src, "test.gaviero", None, Some("build the feature"))
            .map(|c| c.work_units_ordered().expect("toposort"))
            .expect("compile");
        assert_eq!(units[0].coordinator_instructions, "task: build the feature");
    }

    #[test]
    fn named_prompt_agent_substitution() {
        let src = r#"
            prompt body "write to {{AGENT}}-output.md"
            agent alpha { prompt body }
            agent beta  { prompt body }
        "#;
        let units = compile_str(src).expect("should compile");
        let alpha = units.iter().find(|u| u.id == "alpha").unwrap();
        let beta  = units.iter().find(|u| u.id == "beta").unwrap();
        assert_eq!(alpha.coordinator_instructions, "write to alpha-output.md");
        assert_eq!(beta.coordinator_instructions, "write to beta-output.md");
    }

    #[test]
    fn named_prompt_undefined_ref_errors() {
        let src = r#"agent x { prompt missing-prompt }"#;
        let result = compile_str(src);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err().errors[0]);
        assert!(msg.contains("missing-prompt"), "error: {}", msg);
    }

    #[test]
    fn named_prompt_duplicate_decl_errors() {
        let src = r#"
            prompt foo "first"
            prompt foo "second"
            agent x { prompt foo }
        "#;
        let result = compile_str(src);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err().errors[0]);
        assert!(msg.contains("foo"), "error: {}", msg);
    }

    #[test]
    fn inline_prompt_still_works() {
        let src = r#"agent x { prompt "inline text" }"#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "inline text");
    }

    #[test]
    fn inline_prompt_agent_substitution() {
        let src = r#"agent my-agent { prompt "hello {{AGENT}}" }"#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "hello my-agent");
    }

    // ── vars tests ───────────────────────────────────────────────────────────

    #[test]
    fn script_vars_substituted_in_prompt() {
        let src = r#"
            vars { PLANS "plans" }
            agent x { prompt "write to {{PLANS}}/out.md" }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "write to plans/out.md");
    }

    #[test]
    fn agent_vars_substituted_in_prompt() {
        let src = r#"
            agent x {
                vars { MODEL "claude" }
                prompt "write to {{MODEL}}-output.md"
            }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "write to claude-output.md");
    }

    #[test]
    fn agent_vars_override_script_vars() {
        let src = r#"
            vars { MODEL "default" }
            agent x {
                vars { MODEL "claude" }
                prompt "model: {{MODEL}}"
            }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "model: claude");
    }

    #[test]
    fn script_vars_used_when_no_agent_vars() {
        let src = r#"
            vars { PLANS "plans" }
            agent x { prompt "path: {{PLANS}}/file.md" }
            agent y { prompt "also: {{PLANS}}/other.md" }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "path: plans/file.md");
        assert_eq!(units[1].coordinator_instructions, "also: plans/other.md");
    }

    #[test]
    fn iter_and_prev_iter_survive_compile_time() {
        // {{ITER}} and {{PREV_ITER}} must NOT be substituted at compile time —
        // they are reserved for the runtime loop executor.
        let src = r##"
            agent x { prompt #"read v{{PREV_ITER}} write v{{ITER}}"# }
        "##;
        let units = compile_str(src).expect("should compile");
        assert_eq!(units[0].coordinator_instructions, "read v{{PREV_ITER}} write v{{ITER}}");
    }

    #[test]
    fn iter_start_parsed_and_propagated() {
        let src = r#"
            agent a { description "refine" }
            workflow w {
                steps [
                    loop {
                        agents [a]
                        max_iterations 5
                        iter_start 2
                        until command "false"
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).expect("should compile");
        assert_eq!(plan.loop_configs.len(), 1);
        assert_eq!(plan.loop_configs[0].iter_start, 2);
        assert_eq!(plan.loop_configs[0].max_iterations, 5);
    }

    #[test]
    fn iter_start_defaults_to_1() {
        let src = r#"
            agent a { description "t" }
            workflow w {
                steps [
                    loop {
                        agents [a]
                        max_iterations 3
                        until command "false"
                    }
                ]
            }
        "#;
        let plan = compile_plan(src).expect("should compile");
        assert_eq!(plan.loop_configs[0].iter_start, 1);
    }

    #[test]
    fn vars_combined_with_agent_and_prompt_substitution() {
        // All three types of substitution should work together.
        let src = r#"
            vars { DIR "plans" }
            agent claude {
                vars { MODEL "claude" }
                prompt "Read {{DIR}}/codex-plan-v1.md write {{DIR}}/{{MODEL}}-plan-v2.md by {{AGENT}}"
            }
        "#;
        let units = compile_str(src).expect("should compile");
        assert_eq!(
            units[0].coordinator_instructions,
            "Read plans/codex-plan-v1.md write plans/claude-plan-v2.md by claude"
        );
    }

    #[test]
    fn template_plan_refinement_example() {
        // Smoke-test the full plan_refinement.gaviero example structure
        let src = r##"
            client opus  { tier expensive model "claude-opus-4-6" privacy public }
            client codex { tier expensive model "codex:gpt-5.4"   privacy public }

            vars { PLANS "plans" }

            prompt init-body   #"Write to {{PLANS}}/{{MODEL_NAME}}-plan-v1.md"#
            prompt refine-body #"Read {{PLANS}}/claude-plan-v{{PREV_ITER}}.md write {{PLANS}}/{{MODEL_NAME}}-plan-v{{ITER}}.md"#

            agent claude-init  { vars { MODEL_NAME "claude" } client opus  prompt init-body }
            agent codex-init   { vars { MODEL_NAME "codex"  } client codex prompt init-body }
            agent claude-refine { vars { MODEL_NAME "claude" } client opus  prompt refine-body }
            agent codex-refine  { vars { MODEL_NAME "codex"  } client codex prompt refine-body }

            workflow feature-plan-refinement {
                steps [
                    claude-init
                    codex-init
                    loop {
                        agents [claude-refine codex-refine]
                        max_iterations 5
                        iter_start 2
                        until command "false"
                    }
                ]
                max_parallel 2
            }
        "##;
        let plan = compile_plan(src).expect("should compile");

        // 4 distinct work units (no deduplication of different agents)
        let units = plan.work_units_ordered().expect("toposort");
        assert_eq!(units.len(), 4);

        // All use expensive tier
        for u in &units {
            assert_eq!(u.tier, ModelTier::Expensive, "agent {} should be expensive", u.id);
        }

        // Init agents have MODEL_NAME and PLANS substituted at compile time
        let cinit = units.iter().find(|u| u.id == "claude-init").unwrap();
        assert_eq!(cinit.coordinator_instructions, "Write to plans/claude-plan-v1.md");

        // Refine agents: PLANS and MODEL_NAME substituted; ITER/PREV_ITER left for runtime
        let crefine = units.iter().find(|u| u.id == "claude-refine").unwrap();
        assert_eq!(
            crefine.coordinator_instructions,
            "Read plans/claude-plan-v{{PREV_ITER}}.md write plans/claude-plan-v{{ITER}}.md"
        );

        // Loop config: iter_start=2, max_iterations=5
        assert_eq!(plan.loop_configs.len(), 1);
        assert_eq!(plan.loop_configs[0].iter_start, 2);
        assert_eq!(plan.loop_configs[0].max_iterations, 5);
        assert_eq!(plan.loop_configs[0].agent_ids, vec!["claude-refine", "codex-refine"]);

        // max_parallel preserved
        assert_eq!(plan.max_parallel, Some(2));
    }
}
