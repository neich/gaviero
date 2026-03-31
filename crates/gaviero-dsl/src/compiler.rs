#![allow(deprecated)] // AgentBackend::ClaudeCode is deprecated but still needed for WorkUnit

use std::collections::HashMap;

use miette::NamedSource;

use gaviero_core::swarm::models::{AgentBackend, WorkUnit};
use gaviero_core::types::{FileScope, ModelTier, PrivacyLevel};

use crate::ast::*;
use crate::error::{DslError, DslErrors};

// ── Public types ───────────────────────────────────────────────────────────

/// Output of a successful DSL compilation.
#[derive(Debug)]
pub struct CompiledScript {
    /// Work units ready for [`gaviero_core::swarm::pipeline::execute`].
    pub work_units: Vec<WorkUnit>,
    /// The workflow's `max_parallel` value, if one was declared.
    /// Callers should use this to override any default `SwarmConfig.max_parallel`.
    pub max_parallel: Option<usize>,
}

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
) -> Result<CompiledScript, DslErrors> {
    let src = || NamedSource::new(filename, source.to_string());

    // ── Phase 1: index declarations ───────────────────────────────

    let mut client_map: HashMap<&str, &ClientDecl> = HashMap::new();
    let mut agent_map: HashMap<&str, &AgentDecl> = HashMap::new();
    let mut workflow_map: HashMap<&str, &WorkflowDecl> = HashMap::new();
    let mut errors: Vec<DslError> = Vec::new();

    for item in &script.items {
        match item {
            Item::Client(c) => {
                if client_map.insert(c.name.as_str(), c).is_some() {
                    errors.push(DslError::Compile {
                        src: src(),
                        span: (c.name_span.start, c.name_span.end.saturating_sub(c.name_span.start)).into(),
                        reason: format!("duplicate client name `{}`", c.name),
                    });
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
        match compile_agent(decl, &client_map, workflow_memory, source, filename) {
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

    Ok(CompiledScript { work_units, max_parallel: workflow_max_parallel })
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

    let mut agents = Vec::new();
    let mut errors = Vec::new();
    for (name, name_span) in steps {
        match agent_map.get(name.as_str()) {
            Some(a) => agents.push(*a),
            None => errors.push(DslError::Compile {
                src: src(),
                span: (name_span.start, name_span.end.saturating_sub(name_span.start)).into(),
                reason: format!("workflow step `{}` is not a defined agent", name),
            }),
        }
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    Ok(agents)
}

fn compile_agent(
    decl: &AgentDecl,
    client_map: &HashMap<&str, &ClientDecl>,
    workflow_memory: Option<&MemoryBlock>,
    source: &str,
    filename: &str,
) -> Result<WorkUnit, DslError> {
    let src = || NamedSource::new(filename, source.to_string());

    // Resolve client reference
    let (tier, model, privacy) = if let Some((client_name, client_span)) = &decl.client {
        let cd = client_map.get(client_name.as_str()).ok_or_else(|| DslError::Compile {
            src: src(),
            span: (client_span.start, client_span.end.saturating_sub(client_span.start).max(1)).into(),
            reason: format!("undefined client `{}`", client_name),
        })?;
        let tier = cd.tier.as_ref().map(|(t, _)| map_tier(*t)).unwrap_or_default();
        let model = cd.model.as_ref().map(|(m, _)| m.clone());
        let privacy = cd.privacy.as_ref().map(|(p, _)| map_privacy(*p)).unwrap_or_default();
        (tier, model, privacy)
    } else {
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

    let description = decl
        .description
        .as_ref()
        .map(|(s, _)| s.clone())
        .unwrap_or_else(|| decl.name.clone());

    let coordinator_instructions = decl
        .prompt
        .as_ref()
        .map(|(s, _)| s.clone())
        .unwrap_or_default();

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
    })
}

fn map_tier(t: TierLit) -> ModelTier {
    match t {
        TierLit::Coordinator => ModelTier::Coordinator,
        TierLit::Reasoning => ModelTier::Reasoning,
        TierLit::Execution => ModelTier::Execution,
        TierLit::Mechanical => ModelTier::Mechanical,
    }
}

fn map_privacy(p: PrivacyLit) -> PrivacyLevel {
    match p {
        PrivacyLit::Public => PrivacyLevel::Public,
        PrivacyLit::LocalOnly => PrivacyLevel::LocalOnly,
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
        compile_ast(&ast.unwrap(), src, "test.gaviero", None).map(|c| c.work_units)
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
        assert_eq!(units[0].tier, ModelTier::Coordinator);
        assert_eq!(units[0].model, Some("claude-opus-4-6".to_string()));
        assert!(units[0].coordinator_instructions.contains("architecture document"));
        assert_eq!(units[0].max_retries, 2);
        assert_eq!(units[0].scope.owned_paths, vec!["docs/architecture.md"]);
        assert_eq!(units[0].scope.read_only_paths, vec!["src/**"]);

        assert_eq!(units[1].id, "implementer");
        assert_eq!(units[1].tier, ModelTier::Execution);
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
        assert_eq!(units[0].tier, ModelTier::default());
        assert_eq!(units[0].model, None);
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
        let compiled = compile_ast(&ast.unwrap(), src, "t", Some("only_a")).unwrap();
        let units = compiled.work_units;
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "a");
    }

    #[test]
    fn undefined_workflow_step_errors() {
        let src = r#"agent a { } workflow w { steps [a ghost] }"#;
        let (tokens, _) = lexer::lex(src);
        let (ast, _) = parser::parse(&tokens, src, "t");
        let result = compile_ast(&ast.unwrap(), src, "t", None);
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
        let compiled = compile_ast(&ast.unwrap(), src, "t", None).unwrap();
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
}
